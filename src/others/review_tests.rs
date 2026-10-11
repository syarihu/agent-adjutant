//! The queue of readers, what they hand in, and the material they read.

use super::model::{MyReview, is_stale};
use super::read_context::hunk_headers;
use super::read_through::MAX_READ_BYTES;
use super::request_read::HUNG_READER_SECS;
use super::store;
use super::tests::{NOW, pr, record};
use super::*;
use crate::infra::clock::utc_stamp;
use crate::infra::gh::GhRun;
use crate::registry::Liveness;

use serde_json::{Value, json};
use std::path::Path;
use std::sync::Mutex;

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn put(root: &Path, mut r: Record) {
    store::save(root, &mut r).unwrap();
}

fn load(root: &Path, number: u64) -> Record {
    store::load(root, &pr(number)).unwrap().unwrap()
}

fn status(root: &Path, number: u64) -> String {
    load(root, number).ai.unwrap().status
}

/// A record whose read is in `status`, asked for `asked` seconds after `NOW`.
fn asked(number: u64, status: &str, asked: i64) -> Record {
    let mut r = record(number);
    r.ai = Some(AiRead {
        status: status.to_string(),
        started_at: Some(utc_stamp(NOW + asked)),
        ..blank_ai()
    });
    r
}

fn blank_ai() -> AiRead {
    AiRead {
        status: String::new(),
        sha: None,
        started_at: None,
        finished_at: None,
        launched_at: None,
        read_from: None,
        session_id: None,
        pid: None,
        ps_started: None,
        terminal: None,
        error: None,
        counts: None,
        mode: None,
        stale: false,
        extra: Default::default(),
    }
}

fn sessions() -> impl Fn() -> Result<String, String> {
    let n = std::sync::atomic::AtomicUsize::new(0);
    move || {
        Ok(format!(
            "sid-{}",
            n.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ))
    }
}

// ── asking ───────────────────────────────────────────────────────────

#[test]
fn asking_queues_the_read_and_keeps_what_an_earlier_one_made() {
    let root = tempfile::tempdir().unwrap();
    let mut r = record(1);
    r.ai = Some(AiRead {
        status: "ready".to_string(),
        sha: Some("h0".to_string()),
        counts: Some(AiCounts {
            attention: 2,
            check: 1,
            drafts: 3,
            extra: Default::default(),
        }),
        finished_at: Some("20261001T000000Z".to_string()),
        session_id: Some("old".to_string()),
        pid: Some(5),
        error: Some("x".to_string()),
        ..blank_ai()
    });
    put(root.path(), r);
    store::write_read(root.path(), &store::stem(&pr(1)), &json!({"id": "x"})).unwrap();

    let Requested::Queued(r) = request_read(root.path(), &pr(1), NOW, ReadMode::Read).unwrap()
    else {
        panic!("not queued");
    };
    let ai = r.ai.unwrap();
    assert_eq!(ai.status, "queued");
    assert_eq!(ai.started_at, Some(utc_stamp(NOW)));
    assert_eq!(
        (ai.sha.as_deref(), ai.finished_at.is_some()),
        (Some("h0"), true)
    );
    assert_eq!(ai.counts.map(|c| c.drafts), Some(3));
    assert_eq!((ai.session_id, ai.pid, ai.error), (None, None, None));
    // The earlier read-through is still there, and is stale against the head.
    assert!(
        store::load_read(root.path(), &store::stem(&pr(1)))
            .unwrap()
            .is_some()
    );
    assert!(load(root.path(), 1).ai.unwrap().stale);
}

#[test]
fn asking_again_while_queued_or_running_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    for (number, state) in [(1, "queued"), (2, "running")] {
        put(root.path(), asked(number, state, 0));
        let before = std::fs::read(
            root.path()
                .join(format!("others/acme~widget~{number}.json")),
        )
        .unwrap();
        let got = request_read(root.path(), &pr(number), NOW + 500, ReadMode::Read).unwrap();
        assert!(matches!(got, Requested::Already(_)), "{state}");
        let after = std::fs::read(
            root.path()
                .join(format!("others/acme~widget~{number}.json")),
        )
        .unwrap();
        assert_eq!(before, after, "{state}");
    }
}

#[test]
fn asking_for_a_done_or_unknown_pr_is_refused_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let mut done = record(1);
    done.pr_state = PrOpenState::Merged;
    put(root.path(), done);
    assert_eq!(
        request_read(root.path(), &pr(1), NOW, ReadMode::Read),
        Err(RequestError::Done)
    );
    assert_eq!(
        request_read(root.path(), &pr(2), NOW, ReadMode::Read),
        Err(RequestError::NoRecord)
    );
    assert!(load(root.path(), 1).ai.is_none());
    assert!(!root.path().join("others/acme~widget~2.lock").exists());
}

// ── the queue ────────────────────────────────────────────────────────

#[test]
fn at_most_three_readers_run_and_they_start_in_the_order_asked() {
    let root = tempfile::tempdir().unwrap();
    // Asked in the order 5, 3, 4, 1, 2, and 3 and 4 asked at the same moment.
    for (number, at) in [(5, 0), (3, 10), (4, 10), (1, 20), (2, 30)] {
        put(root.path(), asked(number, "queued", at));
    }
    let claims = claim_reads(root.path(), NOW + 100, &sessions()).unwrap();
    let ids: Vec<&str> = claims.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["acme/widget#5", "acme/widget#3", "acme/widget#4"]);
    assert_eq!(claims[0].stem, "acme~widget~5");
    assert_eq!(claims[0].session_id, "sid-0");
    let r = load(root.path(), 5).ai.unwrap();
    assert_eq!(r.status, "running");
    assert_eq!(r.launched_at, Some(utc_stamp(NOW + 100)));
    assert_eq!(r.session_id.as_deref(), Some("sid-0"));
    assert_eq!(status(root.path(), 1), "queued");

    // Back to back: nothing more starts, and nothing running is claimed again.
    assert!(
        claim_reads(root.path(), NOW + 101, &sessions())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn readers_already_running_take_places_from_the_three() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), asked(1, "running", 0));
    put(root.path(), asked(2, "running", 1));
    put(root.path(), asked(3, "queued", 2));
    put(root.path(), asked(4, "queued", 3));
    let claims = claim_reads(root.path(), NOW, &sessions()).unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].id, "acme/widget#3");
    // A place freed is taken by the next in line.
    let pr1 = pr(1);
    fail_read(root.path(), &pr1, None, "gone", NOW).unwrap();
    let claims = claim_reads(root.path(), NOW, &sessions()).unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].id, "acme/widget#4");
}

#[test]
fn a_session_that_cannot_be_made_leaves_every_record_as_it_was() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), asked(1, "queued", 0));
    let err = claim_reads(root.path(), NOW, &|| Err("no entropy".to_string())).unwrap_err();
    assert_eq!(err, "no entropy");
    assert_eq!(status(root.path(), 1), "queued");
}

// ── a reader that is there, and one that is not ──────────────────────

fn running(number: u64, session: &str) -> Record {
    let mut r = asked(number, "running", 0);
    let ai = r.ai.as_mut().unwrap();
    ai.session_id = Some(session.to_string());
    ai.launched_at = Some(utc_stamp(NOW));
    r
}

#[test]
fn a_reader_is_noted_only_under_the_session_the_record_names() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    note_reader(
        root.path(),
        &pr(1),
        "sid-a",
        42,
        Some("started".to_string()),
        None,
    )
    .unwrap();
    let ai = load(root.path(), 1).ai.unwrap();
    assert_eq!(
        (ai.pid, ai.ps_started.as_deref()),
        (Some(42), Some("started"))
    );

    assert!(note_reader(root.path(), &pr(1), "sid-b", 43, None, None).is_err());
    assert_eq!(load(root.path(), 1).ai.unwrap().pid, Some(42));

    put(root.path(), asked(2, "queued", 0));
    assert!(note_reader(root.path(), &pr(2), "sid-a", 1, None, None).is_err());
    assert!(note_reader(root.path(), &pr(3), "sid-a", 1, None, None).is_err());
}

#[test]
fn a_failed_read_keeps_the_earlier_read_through_and_changes_only_a_current_one() {
    let root = tempfile::tempdir().unwrap();
    let mut r = running(1, "sid-a");
    {
        let ai = r.ai.as_mut().unwrap();
        ai.sha = Some("h0".to_string());
        ai.counts = Some(AiCounts {
            attention: 1,
            check: 0,
            drafts: 0,
            extra: Default::default(),
        });
    }
    put(root.path(), r);
    // Not the session that holds it.
    fail_read(root.path(), &pr(1), Some("sid-b"), "late", NOW).unwrap();
    assert_eq!(status(root.path(), 1), "running");
    fail_read(root.path(), &pr(1), Some("sid-a"), "boom", NOW + 5).unwrap();
    let ai = load(root.path(), 1).ai.unwrap();
    assert_eq!(ai.status, "failed");
    assert_eq!(ai.error.as_deref(), Some("boom"));
    assert_eq!(ai.finished_at, Some(utc_stamp(NOW + 5)));
    assert_eq!((ai.sha.as_deref(), ai.counts.is_some()), (Some("h0"), true));
    // A read that is ready is not turned into a failure by a late one.
    let mut ready = record(2);
    ready.ai = Some(AiRead {
        status: "ready".to_string(),
        ..blank_ai()
    });
    put(root.path(), ready);
    fail_read(root.path(), &pr(2), None, "late", NOW).unwrap();
    assert_eq!(status(root.path(), 2), "ready");
}

#[test]
fn a_reader_that_is_gone_fails_and_one_that_cannot_be_told_is_left() {
    let root = tempfile::tempdir().unwrap();
    for (number, pid) in [(1, 10), (2, 20), (3, 30)] {
        let mut r = running(number, &format!("sid-{number}"));
        r.ai.as_mut().unwrap().pid = Some(pid);
        put(root.path(), r);
    }
    let alive = |pid: u32, _: Option<&str>| match pid {
        10 => Liveness::Gone,
        20 => Liveness::CannotTell,
        _ => Liveness::Alive,
    };
    let failed = reap_reads(root.path(), NOW + 1000, &alive);
    assert_eq!(failed, ["acme/widget#1"]);
    assert_eq!(status(root.path(), 1), "failed");
    assert!(
        load(root.path(), 1)
            .ai
            .unwrap()
            .error
            .unwrap()
            .contains("exited")
    );
    assert_eq!(status(root.path(), 2), "running");
    assert_eq!(status(root.path(), 3), "running");
}

#[test]
fn a_reader_with_no_pid_is_waited_for_only_through_the_grace() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-1"));
    let never = |_: u32, _: Option<&str>| panic!("no pid to ask about");
    // Inside the grace.
    assert!(reap_reads(root.path(), NOW + READER_START_GRACE_SECS - 1, &never).is_empty());
    assert_eq!(status(root.path(), 1), "running");
    // Past it.
    let failed = reap_reads(root.path(), NOW + READER_START_GRACE_SECS + 1, &never);
    assert_eq!(failed, ["acme/widget#1"]);
    assert!(
        load(root.path(), 1)
            .ai
            .unwrap()
            .error
            .unwrap()
            .contains("did not start")
    );
}

// ── naming a PR ──────────────────────────────────────────────────────

#[test]
fn a_pr_is_named_by_its_url_or_its_key_and_nothing_else() {
    let want = Some(pr(7));
    assert_eq!(pr_of("https://github.com/Acme/Widget/pull/7"), want);
    assert_eq!(pr_of(" acme/widget#7 "), want);
    assert_eq!(pr_of("Acme/Widget#7"), want);
    for bad in [
        "",
        "#7",
        "7",
        "acme#7",
        "acme/widget#",
        "acme/widget#x",
        "-x/widget#7",
        "https://ghe.example.com/acme/widget/pull/7",
        "https://github.com/acme/widget/issues/7",
        "acme/widget/pull/7",
    ] {
        assert_eq!(pr_of(bad), None, "{bad}");
    }
}

// ── handing in ───────────────────────────────────────────────────────

fn valid() -> Value {
    json!({
        "version": 1,
        "sha": SHA,
        "three": {"purpose": "p", "approach": "a", "impact": "i", "essence": "e"},
        "points": [
            {"level": "attention", "path": "src/a.rs", "line": 4, "title": "t", "why": "w"},
            {"level": "check", "path": "src/b.rs", "title": "t", "why": "w"},
            {"level": "info", "path": "src/c.rs", "title": "t", "why": "w"}
        ],
        "order": [{"path": "src/a.rs", "category": "logic", "note": "n", "skippable": false}],
        "deleted": [{"path": "old.rs", "reason": null, "question": "why?"}],
        "decisions": [{"decision": "d", "gains": "g", "costs": "c"}],
        "tests": {"covered": ["x"], "notCovered": ["y"]},
        "unwritten": ["q"],
        "annotations": [{"path": "src/a.rs", "anchor": "fn a", "kind": "tr", "text": "t"}],
        "drafts": [{"path": "src/a.rs", "line": 4, "body": "b"}],
        "overall": {"body": "ok"},
        "sinceReview": null
    })
}

fn submit(
    root: &Path,
    number: u64,
    value: &Value,
    session: Option<&str>,
) -> Result<Submitted, SubmitError> {
    submit_read(root, &pr(number), &value.to_string(), session, NOW + 50)
}

fn nothing_changed(root: &Path, number: u64) {
    assert!(
        store::load_read(root, &store::stem(&pr(number)))
            .unwrap()
            .is_none()
    );
    let r = load(root, number);
    assert_eq!(r.ai.as_ref().and_then(|ai| ai.sha.clone()), None);
    assert_eq!(r.events.ai_ready, None);
}

type Change = Box<dyn Fn(&mut Value)>;

#[test]
fn a_file_that_breaks_the_contract_is_refused_whole_and_leaves_nothing() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    let cases: Vec<(&str, Change)> = vec![
        ("an unknown key", Box::new(|v| v["pionts"] = json!([]))),
        (
            "an unknown key deeper",
            Box::new(|v| v["three"]["extra"] = json!("x")),
        ),
        (
            "a bad level",
            Box::new(|v| v["points"][0]["level"] = json!("severe")),
        ),
        (
            "a bad category",
            Box::new(|v| v["order"][0]["category"] = json!("misc")),
        ),
        (
            "a bad side",
            Box::new(|v| v["drafts"][0]["side"] = json!("RIGHT")),
        ),
        (
            "a blank purpose",
            Box::new(|v| v["three"]["purpose"] = json!("  ")),
        ),
        (
            "a line break in three",
            Box::new(|v| v["three"]["impact"] = json!("a\nb")),
        ),
        (
            "a long line in three",
            Box::new(|v| v["three"]["approach"] = json!("x".repeat(301))),
        ),
        ("a short sha", Box::new(|v| v["sha"] = json!("abc123"))),
        (
            "an upper-case sha",
            Box::new(|v| v["sha"] = json!(SHA.to_uppercase())),
        ),
        ("line 0", Box::new(|v| v["points"][0]["line"] = json!(0))),
        (
            "a draft starting after its line",
            Box::new(|v| v["drafts"][0]["startLine"] = json!(9)),
        ),
        (
            "a leading slash",
            Box::new(|v| v["points"][0]["path"] = json!("/src/a.rs")),
        ),
        (
            "a multi-line anchor",
            Box::new(|v| v["annotations"][0]["anchor"] = json!("a\nb")),
        ),
        ("nth 0", Box::new(|v| v["annotations"][0]["nth"] = json!(0))),
        ("a wrong version", Box::new(|v| v["version"] = json!(2))),
        (
            "a verdict",
            Box::new(|v| v["overall"]["verdict"] = json!("approve")),
        ),
        (
            "a blank suggestion",
            Box::new(|v| v["overall"]["suggestion"] = json!("  ")),
        ),
        (
            "a two-line suggestion",
            Box::new(|v| v["overall"]["suggestion"] = json!("a\nb")),
        ),
        (
            "a 301-character suggestion",
            Box::new(|v| v["overall"]["suggestion"] = json!("x".repeat(301))),
        ),
        (
            "a suggestion with a tab",
            Box::new(|v| v["overall"]["suggestion"] = json!("a\tb")),
        ),
        (
            "a suggestion with a line separator",
            Box::new(|v| v["overall"]["suggestion"] = json!("a\u{2028}b")),
        ),
        (
            "a suggestion with a next-line",
            Box::new(|v| v["overall"]["suggestion"] = json!("a\u{85}b")),
        ),
        (
            "301 drafts",
            Box::new(|v| {
                v["drafts"] = json!(vec![json!({"path": "a", "line": 1, "body": "b"}); 301])
            }),
        ),
        (
            "a draft body the drafts file cannot hold",
            Box::new(|v| v["drafts"][0]["body"] = json!("x".repeat(20_001))),
        ),
        (
            "a draft path the drafts file cannot hold",
            Box::new(|v| v["drafts"][0]["path"] = json!("p".repeat(1025))),
        ),
        (
            "an overall body the drafts file cannot hold",
            Box::new(|v| v["overall"]["body"] = json!("x".repeat(20_001))),
        ),
        (
            "drafts that are too big together",
            Box::new(|v| {
                v["drafts"] = json!(vec![
                    json!({"path": "a", "line": 1, "body": "x".repeat(20_000)});
                    30
                ])
            }),
        ),
        (
            "points out of order",
            Box::new(|v| v["points"].as_array_mut().unwrap().reverse()),
        ),
        (
            "both reason and question",
            Box::new(|v| v["deleted"][0]["reason"] = json!("because")),
        ),
        (
            "neither reason nor question",
            Box::new(|v| v["deleted"][0]["question"] = json!(null)),
        ),
        (
            "a blank reason",
            Box::new(|v| {
                v["deleted"][0]["reason"] = json!(" ");
                v["deleted"][0]["question"] = json!(null);
            }),
        ),
    ];
    for (what, change) in &cases {
        let mut v = valid();
        change(&mut v);
        let err = submit(root.path(), 1, &v, None).unwrap_err();
        assert!(
            matches!(err, SubmitError::Invalid(ref p) if !p.is_empty()),
            "{what}: {err:?}"
        );
        nothing_changed(root.path(), 1);
    }
    // Not JSON, and too big.
    assert!(matches!(
        submit_read(root.path(), &pr(1), "not json", None, NOW),
        Err(SubmitError::Invalid(_))
    ));
    let big = " ".repeat(MAX_READ_BYTES + 1);
    assert!(matches!(
        submit_read(root.path(), &pr(1), &big, None, NOW),
        Err(SubmitError::Invalid(_))
    ));
    nothing_changed(root.path(), 1);
    assert_eq!(status(root.path(), 1), "running");
}

#[test]
fn every_problem_is_reported_in_one_answer() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    let mut v = valid();
    v["sha"] = json!("zz");
    v["three"]["purpose"] = json!("");
    v["points"][0]["line"] = json!(0);
    let Err(SubmitError::Invalid(problems)) = submit(root.path(), 1, &v, None) else {
        panic!("accepted");
    };
    assert_eq!(problems.len(), 3, "{problems:?}");
}

#[test]
fn a_missing_record_or_one_nobody_asked_about_is_refused() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), record(1));
    assert_eq!(
        submit(root.path(), 1, &valid(), None).unwrap_err(),
        SubmitError::NotAsked
    );
    assert_eq!(
        submit(root.path(), 2, &valid(), None).unwrap_err(),
        SubmitError::NoRecord
    );
    assert!(!root.path().join("others/reads").exists());
    assert!(!root.path().join("others/acme~widget~2.lock").exists());
}

#[test]
fn a_valid_file_is_stored_and_the_record_is_ready() {
    let root = tempfile::tempdir().unwrap();
    let mut r = running(1, "sid-a");
    r.ai.as_mut().unwrap().error = Some("an old failure".to_string());
    r.head_sha = SHA.to_string();
    put(root.path(), r);
    let done = submit(root.path(), 1, &valid(), Some("sid-a")).unwrap();
    let ai = done.record.ai.unwrap();
    assert_eq!(ai.status, "ready");
    assert_eq!(ai.sha.as_deref(), Some(SHA));
    assert_eq!(ai.finished_at, Some(utc_stamp(NOW + 50)));
    assert_eq!(
        ai.counts,
        Some(AiCounts {
            attention: 1,
            check: 1,
            drafts: 1,
            extra: Default::default(),
        })
    );
    assert_eq!((ai.error, ai.stale), (None, false));
    assert_eq!(done.record.events.ai_ready, Some(utc_stamp(NOW + 50)));
    assert_eq!(done.record.state, State::AiReady);
    // What a reader of the stored file sees.
    let stored = read_through(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(stored["id"], "acme/widget#1");
    assert_eq!(stored["madeAt"], utc_stamp(NOW + 50));
    assert_eq!(stored["session"], "sid-a");
    assert_eq!(stored["sha"], SHA);
    assert_eq!(stored["three"]["essence"], "e");
    assert_eq!(stored["drafts"][0]["side"], "right");
    assert_eq!(stored["overall"], json!({"body": "ok"}));
    assert_eq!(stored["sinceReview"], Value::Null);
    assert_eq!(stored["tests"]["notCovered"][0], "y");
    // The head the PR has moved to is not an error, only a stale read.
    let mut moved = load(root.path(), 1);
    moved.head_sha = "later".to_string();
    store::save(root.path(), &mut moved).unwrap();
    assert!(load(root.path(), 1).ai.unwrap().stale);
}

#[test]
fn an_overall_suggestion_is_optional_and_kept_beside_the_body() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    // 300 characters is the limit, and counted as characters, not bytes.
    let mut v = valid();
    v["overall"]["suggestion"] = json!("丸".repeat(300));
    submit(root.path(), 1, &v, None).unwrap();
    let stored = read_through(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(stored["overall"]["body"], "ok");
    assert_eq!(stored["overall"]["suggestion"], "丸".repeat(300));
    // Without it nothing is stored under the key: a read-through from before it stays as it was.
    put(root.path(), running(2, "sid-b"));
    submit(root.path(), 2, &valid(), None).unwrap();
    let stored = read_through(root.path(), &pr(2)).unwrap().unwrap();
    assert_eq!(stored["overall"], json!({"body": "ok"}));
}

#[test]
fn a_person_can_hand_a_file_in_by_hand_with_no_session() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    submit(root.path(), 1, &valid(), None).unwrap();
    let stored = read_through(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(stored["session"], Value::Null);
}

#[test]
fn a_re_read_waiting_in_line_keeps_its_place_when_the_old_reader_hands_in() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    // Asked again while the first reader is still there; the failure frees nothing here.
    fail_read(root.path(), &pr(1), Some("sid-a"), "x", NOW).unwrap();
    request_read(root.path(), &pr(1), NOW + 10, ReadMode::Read).unwrap();
    // A person's file is accepted and the re-read stays queued.
    submit(root.path(), 1, &valid(), None).unwrap();
    assert_eq!(status(root.path(), 1), "queued");
    assert!(read_through(root.path(), &pr(1)).unwrap().is_some());
}

#[test]
fn a_superseded_reader_is_refused_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-new"));
    assert_eq!(
        submit(root.path(), 1, &valid(), Some("sid-old")).unwrap_err(),
        SubmitError::Superseded
    );
    nothing_changed(root.path(), 1);

    // The record has no session at all because a re-read is waiting in the queue.
    let mut r = running(2, "sid-a");
    store::save(root.path(), &mut r).unwrap();
    fail_read(root.path(), &pr(2), Some("sid-a"), "x", NOW).unwrap();
    request_read(root.path(), &pr(2), NOW + 5, ReadMode::Read).unwrap();
    assert_eq!(load(root.path(), 2).ai.unwrap().session_id, None);
    assert_eq!(
        submit(root.path(), 2, &valid(), Some("sid-a")).unwrap_err(),
        SubmitError::Superseded
    );
    nothing_changed(root.path(), 2);
}

#[test]
fn a_record_that_cannot_be_saved_takes_the_file_back() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    let failing = |_: &mut Record| Err("disk full".to_string());
    // First with no earlier read-through: the new file goes away.
    let err = submit_read_with(
        root.path(),
        &pr(1),
        &valid().to_string(),
        None,
        NOW,
        &failing,
    )
    .unwrap_err();
    assert_eq!(err, SubmitError::Failed("disk full".to_string()));
    nothing_changed(root.path(), 1);

    // Then with one: the earlier text comes back as it was.
    submit(root.path(), 1, &valid(), None).unwrap();
    let before = store::load_read(root.path(), &store::stem(&pr(1)))
        .unwrap()
        .unwrap();
    let mut other = valid();
    other["three"]["essence"] = json!("another");
    let err = submit_read_with(
        root.path(),
        &pr(1),
        &other.to_string(),
        None,
        NOW + 9,
        &failing,
    )
    .unwrap_err();
    assert!(matches!(err, SubmitError::Failed(_)));
    let after = store::load_read(root.path(), &store::stem(&pr(1)))
        .unwrap()
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn a_key_a_newer_binary_wrote_survives_a_read_of_the_stored_file() {
    let root = tempfile::tempdir().unwrap();
    store::write_read(
        root.path(),
        &store::stem(&pr(1)),
        &json!({"id": "acme/widget#1", "madeAt": "20261001T000000Z", "session": null,
                "sha": SHA, "fromTheFuture": {"x": 1}}),
    )
    .unwrap();
    let got = read_through(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(got["fromTheFuture"], json!({"x": 1}));
    // No file is no answer; a file that is not a read-through is an error, not no answer.
    assert_eq!(read_through(root.path(), &pr(2)), Ok(None));
    store::write_text(
        &root.path().join("others/reads/acme~widget~3.json"),
        "{\"nope\": 1}",
    )
    .unwrap();
    assert!(read_through(root.path(), &pr(3)).is_err());
}

#[test]
fn staleness_is_derived_and_a_hand_written_flag_is_ignored() {
    let root = tempfile::tempdir().unwrap();
    let mut r = record(1);
    r.ai = Some(AiRead {
        status: "ready".to_string(),
        sha: Some("old".to_string()),
        ..blank_ai()
    });
    put(root.path(), r);
    assert!(load(root.path(), 1).ai.unwrap().stale);

    // A flag on disk says the opposite of the facts, in both directions.
    let path = root.path().join("others/acme~widget~1.json");
    let mut raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    raw["ai"]["stale"] = json!(false);
    std::fs::write(&path, raw.to_string()).unwrap();
    assert!(load(root.path(), 1).ai.unwrap().stale);
    raw["ai"]["sha"] = json!("h1");
    raw["ai"]["stale"] = json!(true);
    std::fs::write(&path, raw.to_string()).unwrap();
    assert!(!load(root.path(), 1).ai.unwrap().stale);
    assert!(!store::list(root.path())[0].ai.as_ref().unwrap().stale);
    // No sha, no staleness.
    raw["ai"]["sha"] = json!(null);
    std::fs::write(&path, raw.to_string()).unwrap();
    assert!(!load(root.path(), 1).ai.unwrap().stale);
}

// ── the material ─────────────────────────────────────────────────────

/// A `gh` that answers what the context asks for and writes down every call.
struct Gh {
    calls: Mutex<Vec<Vec<String>>>,
    /// The compare call fails, as it does after a force-push.
    compare_fails: bool,
    /// `gh pr diff` fails, as it does for a diff too big to give.
    diff_fails: bool,
    /// `gh pr view` returns a hundred commits.
    many_commits: bool,
    /// What `gh pr diff` prints, when it is not the one-line diff.
    diff_text: Option<String>,
    /// What the files endpoint prints (a list of files with their patches); it fails when none.
    files_answer: Option<String>,
}

impl Gh {
    fn new() -> Self {
        Gh {
            calls: Mutex::new(Vec::new()),
            compare_fails: false,
            diff_fails: false,
            many_commits: false,
            diff_text: None,
            files_answer: None,
        }
    }

    fn run(&self, args: &[&str]) -> Result<GhRun, String> {
        self.calls
            .lock()
            .unwrap()
            .push(args.iter().map(|a| a.to_string()).collect());
        let joined = args.join(" ");
        let ok = |stdout: String| {
            Ok(GhRun {
                ok: true,
                stdout,
                stderr: String::new(),
            })
        };
        if joined.starts_with("pr view") {
            let commits = if self.many_commits { 100 } else { 1 };
            return ok(json!({
                "number": 1, "title": "t", "body": "b", "headRefOid": SHA,
                "files": [{"path": "src/a.rs", "additions": 3, "deletions": 1}],
                "commits": vec![json!({"messageHeadline": "m"}); commits]
            })
            .to_string());
        }
        if joined.starts_with("pr diff") {
            if self.diff_fails {
                return Ok(GhRun {
                    ok: false,
                    stdout: String::new(),
                    stderr: "the diff is too large".to_string(),
                });
            }
            return ok(self
                .diff_text
                .clone()
                .unwrap_or_else(|| "diff --git a/x b/x\n".to_string()));
        }
        if joined.ends_with("/files") {
            return match &self.files_answer {
                Some(text) => ok(text.clone()),
                None => Ok(GhRun {
                    ok: false,
                    stdout: String::new(),
                    stderr: "files refused".to_string(),
                }),
            };
        }
        if joined.contains("/comments") {
            // Two pages, printed back to back.
            return ok(format!(
                "{}{}",
                json!([{"id": 11, "pull_request_review_id": 7, "path": "src/a.rs", "body": "mine"}]),
                json!([{"id": 12, "in_reply_to_id": 11, "body": "reply"},
                       {"id": 13, "pull_request_review_id": 8, "body": "someone else"}])
            ));
        }
        if joined.contains("/reviews") {
            return ok(json!([
                {"id": 7, "user": {"login": "Me"}, "commit_id": "abc1234", "state": "commented",
                 "submitted_at": "2026-10-02T00:00:00Z"},
                {"id": 8, "user": {"login": "other"}, "commit_id": "abc1234", "state": "approved",
                 "submitted_at": "2026-10-03T00:00:00Z"}
            ])
            .to_string());
        }
        if joined.starts_with("api user") {
            return ok("me\n".to_string());
        }
        if joined.contains("/compare/") {
            return if self.compare_fails {
                Ok(GhRun {
                    ok: false,
                    stdout: String::new(),
                    stderr: "Not Found".to_string(),
                })
            } else {
                ok("since\n".to_string())
            };
        }
        Err(format!("unexpected gh call: {joined}"))
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

fn context(root: &Path, gh: &Gh, out: &Path) -> Result<ContextSummary, String> {
    read_context_with(root, &pr(1), out, None, &|args, _| gh.run(args))
}

/// The directory of the reader of `acme/widget#1`: the one place its material may go.
fn own(root: &Path) -> std::path::PathBuf {
    reader_dir(root, &pr(1))
}

fn reviewed_record(commit: &str) -> Record {
    let mut r = record(1);
    r.my_review = Some(MyReview {
        state: ReviewState::Commented,
        submitted_at: "20261002T000000Z".to_string(),
        commit: commit.to_string(),
    });
    r
}

#[test]
fn the_material_is_written_with_get_requests_only() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), reviewed_record("abc1234"));
    let out = root.path().join("review/acme~widget~1");
    let gh = Gh::new();
    let summary = context(root.path(), &gh, &out).unwrap();
    assert_eq!(summary.head_sha, SHA);
    assert!(summary.rereview);
    assert_eq!(summary.files[0].path, "src/a.rs");
    assert_eq!(summary.files[0].additions, 3);
    for name in [
        "pr.json",
        "diff.patch",
        "review-comments.json",
        "reviews.json",
        "my-last-review.json",
        "since-review.patch",
    ] {
        assert!(out.join(name).exists(), "{name}");
        assert!(summary.written.contains(&name.to_string()), "{name}");
    }
    // Nothing a call could write with.
    for call in gh.calls() {
        for arg in &call {
            assert!(
                !matches!(
                    arg.as_str(),
                    "-X" | "--method" | "-f" | "-F" | "--field" | "--raw-field" | "--input"
                ),
                "{call:?}"
            );
        }
        assert!(matches!(call[0].as_str(), "pr" | "api"), "{call:?}");
    }
    // The pages are one list, and what the person said is in the record's review.
    let comments: Value =
        serde_json::from_slice(&std::fs::read(out.join("review-comments.json")).unwrap()).unwrap();
    assert_eq!(comments.as_array().unwrap().len(), 3);
    let pr_json: Value =
        serde_json::from_slice(&std::fs::read(out.join("pr.json")).unwrap()).unwrap();
    assert_eq!(pr_json["myReview"]["commit"], "abc1234");
    // Their review's comments and the replies to them, and not somebody else's.
    let mine: Value =
        serde_json::from_slice(&std::fs::read(out.join("my-last-review.json")).unwrap()).unwrap();
    let ids: Vec<u64> = mine
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, [11, 12]);
    assert_eq!(
        std::fs::read_to_string(out.join("since-review.patch")).unwrap(),
        "since\n"
    );
}

#[test]
fn the_re_review_files_are_written_only_for_a_pr_that_moved_since_your_review() {
    let root = tempfile::tempdir().unwrap();
    for (what, record) in [
        ("no review", record(1)),
        ("same head", reviewed_record(SHA)),
    ] {
        put(root.path(), record);
        let out = own(root.path());
        let _ = std::fs::remove_dir_all(&out);
        let gh = Gh::new();
        let summary = context(root.path(), &gh, &out).unwrap();
        assert!(!summary.rereview, "{what}");
        assert!(!out.join("my-last-review.json").exists(), "{what}");
        assert!(!out.join("since-review.patch").exists(), "{what}");
        assert!(out.join("pr.json").exists(), "{what}");
        assert!(
            gh.calls().iter().all(|c| !c.join(" ").contains("compare")),
            "{what}"
        );
    }
}

#[test]
fn a_compare_that_fails_is_a_note_and_not_a_failure() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), reviewed_record("abc1234"));
    let out = own(root.path());
    let mut gh = Gh::new();
    gh.compare_fails = true;
    let summary = context(root.path(), &gh, &out).unwrap();
    assert!(!out.join("since-review.patch").exists());
    assert!(
        summary
            .notes
            .iter()
            .any(|n| n.contains("since-review.patch"))
    );
    assert!(out.join("my-last-review.json").exists());

    // A commit that is not a SHA is never put in a path.
    put(root.path(), reviewed_record("../../x"));
    let gh = Gh::new();
    let summary = context(root.path(), &gh, &own(root.path())).unwrap();
    assert!(summary.notes.iter().any(|n| n.contains("not a SHA")));
    assert!(gh.calls().iter().all(|c| !c.join(" ").contains("compare")));
}

#[test]
fn the_material_goes_only_to_the_directory_of_the_reader_of_that_pr() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), record(1));
    put(root.path(), record(2));
    let gh = Gh::new();
    let elsewhere = tempfile::tempdir().unwrap();
    for out in [
        elsewhere.path().to_path_buf(),
        root.path().to_path_buf(),
        root.path().join("review"),
        root.path().join("others"),
        root.path().join("review/../others"),
        root.path().join("review/x/../../others"),
        // Another PR's reader, and one that is not any.
        reader_dir(root.path(), &pr(2)),
        root.path().join("review/x"),
        own(root.path()).join("sub"),
    ] {
        let err = context(root.path(), &gh, &out).unwrap_err();
        assert!(
            err.contains("is not the directory of this pull request's reader"),
            "{}: {err}",
            out.display()
        );
    }
    // Nothing was asked of GitHub, and nothing written, for any of them.
    assert!(gh.calls().is_empty());
    assert!(!reader_dir(root.path(), &pr(2)).exists());
    // Its own is made when it is not there yet.
    context(root.path(), &gh, &own(root.path())).unwrap();
    assert!(own(root.path()).join("pr.json").exists());
    // A link out of the review directory does not make a place outside it the reader's own.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join("review/link")).unwrap();
        assert!(context(root.path(), &gh, &root.path().join("review/link")).is_err());
    }
}

#[test]
fn only_the_reader_the_record_names_may_fetch_for_its_pr() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    let gh = Gh::new();
    let fetch = |session: Option<&str>| {
        read_context_with(
            root.path(),
            &pr(1),
            &own(root.path()),
            session,
            &|args, _| gh.run(args),
        )
    };
    let err = fetch(Some("sid-old")).unwrap_err();
    assert!(err.contains("superseded"), "{err}");
    // Refused before anything was asked or written.
    assert!(gh.calls().is_empty());
    assert!(!own(root.path()).exists());
    fetch(Some("sid-a")).unwrap();
    assert!(own(root.path()).join("pr.json").exists());
    // A record whose read was asked for again names no session: the old reader is out.
    put(root.path(), asked(2, "queued", 0));
    let err = read_context_with(
        root.path(),
        &pr(2),
        &reader_dir(root.path(), &pr(2)),
        Some("sid-a"),
        &|a, _| gh.run(a),
    )
    .unwrap_err();
    assert!(err.contains("superseded"), "{err}");
}

#[test]
fn material_for_a_pr_with_no_record_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let gh = Gh::new();
    let err = context(root.path(), &gh, &own(root.path())).unwrap_err();
    assert!(err.contains("no record"), "{err}");
    assert!(gh.calls().is_empty());
}

#[test]
fn points_must_come_attention_then_check_then_info() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    let mut v = valid();
    // check, attention, info: the second is out of place, and the message says which and how to fix it.
    v["points"].as_array_mut().unwrap().swap(0, 1);
    let Err(SubmitError::Invalid(problems)) = submit(root.path(), 1, &v, None) else {
        panic!("accepted");
    };
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].starts_with("points[1]"), "{problems:?}");
    assert!(
        problems[0].contains("attention first, then check, then info"),
        "{problems:?}"
    );
    // Equal levels in a row are fine, and so are lists of one level and none.
    let mut ok = valid();
    ok["points"] = json!([
        {"level": "check", "path": "a", "title": "t", "why": "w"},
        {"level": "check", "path": "b", "title": "t", "why": "w"},
        {"level": "info", "path": "c", "title": "t", "why": "w"}
    ]);
    submit(root.path(), 1, &ok, None).unwrap();
}

#[test]
fn a_reader_that_cannot_finish_ends_its_read_and_frees_the_place() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    assert_eq!(
        fail_read(root.path(), &pr(1), Some("sid-b"), "no", NOW),
        Ok(false)
    );
    assert_eq!(status(root.path(), 1), "running");
    assert_eq!(
        fail_read(root.path(), &pr(1), Some("sid-a"), "could not read", NOW),
        Ok(true)
    );
    // Failed, so it can be asked for again, and the place is free for the next in line.
    assert_eq!(status(root.path(), 1), "failed");
    assert!(matches!(
        request_read(root.path(), &pr(1), NOW + 1, ReadMode::Read),
        Ok(Requested::Queued(_))
    ));
}

#[test]
fn a_running_read_is_never_replaced_by_asking_alone_and_a_hung_one_is_named() {
    let root = tempfile::tempdir().unwrap();
    let mut r = running(1, "sid-old");
    {
        let ai = r.ai.as_mut().unwrap();
        ai.pid = Some(4242);
        ai.ps_started = Some("Mon Oct  5 10:00:00 2026".to_string());
    }
    put(root.path(), r);
    let late = NOW + HUNG_READER_SECS + 1;
    // However long it has run, asking is `Already`: the old reader has not been looked at.
    assert!(matches!(
        request_read(root.path(), &pr(1), late, ReadMode::Read),
        Ok(Requested::Already(_))
    ));
    assert_eq!(
        load(root.path(), 1).ai.unwrap().session_id.as_deref(),
        Some("sid-old")
    );
    // Inside the limit nobody is named; past it, the reader is.
    assert_eq!(
        hung_reader(root.path(), &pr(1), NOW + HUNG_READER_SECS - 1),
        Ok(None)
    );
    let hung = hung_reader(root.path(), &pr(1), late).unwrap().unwrap();
    assert_eq!(hung.session_id, "sid-old");
    assert_eq!(hung.pid, Some(4242));
    assert_eq!(hung.ps_started.as_deref(), Some("Mon Oct  5 10:00:00 2026"));
    // A read that is not running, or has no launch time, names no one.
    put(root.path(), asked(2, "queued", 0));
    assert_eq!(hung_reader(root.path(), &pr(2), late), Ok(None));
    assert_eq!(hung_reader(root.path(), &pr(3), late), Ok(None));
}

#[test]
fn a_read_is_taken_over_only_from_the_reader_that_was_checked() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-old"));
    let late = NOW + HUNG_READER_SECS + 1;
    // The session was replaced since it was looked at: nothing changes.
    let Ok(Requested::Already(_)) =
        take_over_read(root.path(), &pr(1), late, "sid-other", ReadMode::Read)
    else {
        panic!("took over another reader's read");
    };
    assert_eq!(
        load(root.path(), 1).ai.unwrap().session_id.as_deref(),
        Some("sid-old")
    );
    // The one that was checked is replaced, and can no longer hand in.
    let Ok(Requested::Queued(r)) =
        take_over_read(root.path(), &pr(1), late, "sid-old", ReadMode::Read)
    else {
        panic!("not taken over");
    };
    let ai = r.ai.unwrap();
    assert_eq!(
        (ai.status.as_str(), ai.session_id, ai.pid),
        ("queued", None, None)
    );
    assert_eq!(
        submit(root.path(), 1, &valid(), Some("sid-old")).unwrap_err(),
        SubmitError::Superseded
    );
    // Taken over already: the second finds no running read under it.
    assert!(matches!(
        take_over_read(root.path(), &pr(1), late, "sid-old", ReadMode::Read),
        Ok(Requested::Already(_))
    ));
    // A done PR is not.
    let mut done = running(2, "sid-x");
    done.pr_state = PrOpenState::Merged;
    put(root.path(), done);
    assert_eq!(
        take_over_read(root.path(), &pr(2), late, "sid-x", ReadMode::Read),
        Err(RequestError::Done)
    );
}

#[test]
fn counts_keep_a_key_they_do_not_know_and_read_a_missing_one_as_zero() {
    let root = tempfile::tempdir().unwrap();
    let mut r = record(1);
    r.ai = Some(AiRead {
        status: "ready".to_string(),
        counts: Some(AiCounts {
            attention: 1,
            check: 2,
            drafts: 3,
            extra: Default::default(),
        }),
        ..blank_ai()
    });
    put(root.path(), r);
    // A newer binary's record: a count this one does not know, and `check` left out.
    let path = root.path().join("others/acme~widget~1.json");
    let mut raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    raw["ai"]["counts"] = json!({"attention": 4, "drafts": 5, "info": 6});
    std::fs::write(&path, raw.to_string()).unwrap();
    // It still loads, and is listed.
    let counts = load(root.path(), 1).ai.unwrap().counts.unwrap();
    assert_eq!((counts.attention, counts.check, counts.drafts), (4, 0, 5));
    assert_eq!(counts.extra["info"], 6);
    assert_eq!(store::list(root.path()).len(), 1);
    // And the unknown count survives a save.
    let mut again = load(root.path(), 1);
    store::save(root.path(), &mut again).unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["ai"]["counts"]["info"], 6);
}

#[test]
fn a_removed_record_takes_its_read_through_and_reader_directory_with_it() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    store::write_read(root.path(), &store::stem(&pr(1)), &json!({"id": "x"})).unwrap();
    let scratch = root.path().join("review/acme~widget~1");
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("read-through.json"), "{}").unwrap();
    store::remove(root.path(), &store::stem(&pr(1))).unwrap();
    assert!(store::load(root.path(), &pr(1)).unwrap().is_none());
    assert!(
        store::load_read(root.path(), &store::stem(&pr(1)))
            .unwrap()
            .is_none()
    );
    assert!(!scratch.exists());
    // Nothing to remove is not an error.
    store::remove(root.path(), &store::stem(&pr(1))).unwrap();
}

#[test]
fn the_context_removes_what_an_earlier_read_left_and_leaves_the_record_alone() {
    let root = tempfile::tempdir().unwrap();
    // Not reviewed, so no optional file is written this time.
    put(root.path(), record(1));
    let out = own(root.path());
    std::fs::create_dir_all(&out).unwrap();
    for name in ["my-last-review.json", "since-review.patch", "diff.patch"] {
        std::fs::write(out.join(name), "from an earlier read").unwrap();
    }
    let before = std::fs::read(root.path().join("others/acme~widget~1.json")).unwrap();
    let gh = Gh::new();
    context(root.path(), &gh, &out).unwrap();
    assert!(!out.join("my-last-review.json").exists());
    assert!(!out.join("since-review.patch").exists());
    // The diff is written again, not the old one.
    assert_eq!(
        std::fs::read_to_string(out.join("diff.patch")).unwrap(),
        "diff --git a/x b/x\n"
    );
    // The live head is not the record's `h1`, and the record is the sync's to bring up to date:
    // reading must not touch it, or the sync would lose the push it was to see.
    let r = store::load(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(r.head_sha, "h1");
    assert_eq!(r.head_seen_at, None);
    assert_eq!(
        std::fs::read(root.path().join("others/acme~widget~1.json")).unwrap(),
        before
    );

    // Nor a done one.
    let mut done = record(2);
    done.pr_state = PrOpenState::Merged;
    put(root.path(), done);
    let gh = Gh::new();
    read_context_with(
        root.path(),
        &pr(2),
        &reader_dir(root.path(), &pr(2)),
        None,
        &|args, _| gh.run(args),
    )
    .unwrap();
    assert_eq!(load(root.path(), 2).head_sha, "h1");
}

#[test]
fn a_diff_that_cannot_be_fetched_and_lists_that_may_be_cut_are_notes() {
    let root = tempfile::tempdir().unwrap();
    let mut r = record(1);
    r.changed_files = 5;
    put(root.path(), r);
    let out = own(root.path());
    let mut gh = Gh::new();
    gh.diff_fails = true;
    let summary = context(root.path(), &gh, &out).unwrap();
    assert!(!out.join("diff.patch").exists());
    assert!(!summary.written.contains(&"diff.patch".to_string()));
    assert!(
        summary
            .notes
            .iter()
            .any(|n| n.contains("diff.patch could not be fetched") && n.contains("do not guess"))
    );
    // One file came back of five.
    assert!(
        summary
            .notes
            .iter()
            .any(|n| n.contains("file list may be cut: 1 of 5"))
    );
    assert!(out.join("pr.json").exists() && out.join("reviews.json").exists());

    // A hundred commits is the limit of what is returned.
    let mut gh = Gh::new();
    gh.many_commits = true;
    let summary = context(root.path(), &gh, &own(root.path())).unwrap();
    assert!(
        summary
            .notes
            .iter()
            .any(|n| n.contains("commit list may be cut"))
    );
    // Nothing to say of a PR that came back whole.
    let mut whole = record(1);
    whole.changed_files = 1;
    put(root.path(), whole);
    let summary = context(root.path(), &Gh::new(), &own(root.path())).unwrap();
    assert!(
        summary.notes.iter().all(|n| !n.contains("may be cut")),
        "{:?}",
        summary.notes
    );
}

// ── a read of a head the sync has not seen yet ───────────────────────

/// A record the sync last saw at `head` at `seen_secs`, holding a read of `sha` whose material was
/// taken at `from_secs` (it was handed in later).
fn read_of(sha: &str, from_secs: i64, head: &str, seen_secs: i64) -> Record {
    let mut r = record(1);
    r.head_sha = head.to_string();
    r.head_seen_at = Some(utc_stamp(seen_secs));
    r.ai = Some(AiRead {
        status: "ready".to_string(),
        sha: Some(sha.to_string()),
        read_from: Some(utc_stamp(from_secs)),
        finished_at: Some(utc_stamp(from_secs + 600)),
        ..blank_ai()
    });
    r
}

#[test]
fn a_read_of_a_newer_head_than_the_sync_saw_is_not_stale_until_the_sync_sees_another() {
    // The sync saw B at NOW; a reader then read C (the live head) at NOW + 60.
    let read = read_of("hC", NOW + 60, "hB", NOW);
    assert!(!is_stale(&read));
    // The next sync brings C: the heads are equal, and it stays fresh.
    let mut synced = read.clone();
    synced.head_sha = "hC".to_string();
    synced.head_seen_at = Some(utc_stamp(NOW + 120));
    assert!(!is_stale(&synced));
    // Or it brings D, a push after the read: the sync saw it after the read finished, so it is stale.
    let mut pushed = read.clone();
    pushed.head_sha = "hD".to_string();
    pushed.head_seen_at = Some(utc_stamp(NOW + 120));
    assert!(is_stale(&pushed));
    // A read made at a head the sync saw before, and since replaced, is stale as it always was.
    assert!(is_stale(&read_of("hA", NOW, "hB", NOW + 30)));
    // The exact ordering that went wrong when it was judged by the hand-in: the reader took its
    // material at A (NOW), a push of B was stored by a sync (NOW + 20), and only then did the
    // reader hand in, its read still of A. It is stale, however late it was handed in.
    let mut late = read_of("hA", NOW, "hB", NOW + 20);
    late.ai.as_mut().unwrap().finished_at = Some(utc_stamp(NOW + 30));
    assert!(is_stale(&late));
    // The same read with the sync before the reader took its material is not (the reader read what
    // the sync had not yet seen).
    assert!(!is_stale(&read_of("hA", NOW + 40, "hB", NOW + 20)));
    // A record from before the stamp has nothing but the heads to go on.
    let mut old = read_of("hC", NOW + 60, "hB", NOW);
    old.head_seen_at = None;
    assert!(is_stale(&old));
    let mut unstamped = read_of("hC", NOW + 60, "hB", NOW);
    unstamped.ai.as_mut().unwrap().read_from = None;
    assert!(is_stale(&unstamped));
}

// ── what a re-review says it is about ────────────────────────────────

#[test]
fn since_review_must_name_the_commit_of_the_review_the_record_holds() {
    let root = tempfile::tempdir().unwrap();
    let since = |commit: &str| {
        json!({"commit": commit, "comments": [
        {"commentId": 11, "path": "src/a.rs", "status": "addressed", "note": "n"}], "newChanges": []})
    };
    let with = |commit: &str| {
        let mut v = valid();
        v["sinceReview"] = since(commit);
        v
    };
    // A PR never reviewed has nothing to compare with.
    put(root.path(), running(1, "sid-a"));
    let Err(SubmitError::Invalid(problems)) = submit(root.path(), 1, &with("abc1234"), None) else {
        panic!("accepted");
    };
    assert!(
        problems[0].contains("only for a PR you have already reviewed"),
        "{problems:?}"
    );
    nothing_changed(root.path(), 1);

    // One reviewed on abc1234: another commit is refused, and says which is right.
    let mut r = running(2, "sid-b");
    r.my_review = Some(MyReview {
        state: ReviewState::Commented,
        submitted_at: "20261002T000000Z".to_string(),
        commit: "abc1234".to_string(),
    });
    put(root.path(), r);
    let Err(SubmitError::Invalid(problems)) = submit(root.path(), 2, &with("fff9999"), None) else {
        panic!("accepted");
    };
    assert!(
        problems[0].contains("fff9999") && problems[0].contains("abc1234"),
        "{problems:?}"
    );
    nothing_changed(root.path(), 2);
    // The right one is taken, and null is always fine.
    submit(root.path(), 2, &with("abc1234"), Some("sid-b")).unwrap();
    let stored = read_through(root.path(), &pr(2)).unwrap().unwrap();
    assert_eq!(stored["sinceReview"]["commit"], "abc1234");
}

#[test]
fn the_reader_directory_has_one_name() {
    let root = std::path::Path::new("/state");
    assert_eq!(review_home(root), std::path::Path::new("/state/review"));
    assert_eq!(reader_stem(&pr(7)), "acme~widget~7");
    assert_eq!(pr_id(&pr(7)), "acme/widget#7");
    assert_eq!(
        reader_dir(root, &pr(7)),
        std::path::Path::new("/state/review/acme~widget~7")
    );
    // A claim carries the same directory, and a removal takes that one.
    let tmp = tempfile::tempdir().unwrap();
    put(tmp.path(), asked(7, "queued", 0));
    let claims = claim_reads(tmp.path(), NOW, &sessions()).unwrap();
    assert_eq!(claims[0].dir, reader_dir(tmp.path(), &pr(7)));
    assert_eq!(claims[0].stem, reader_stem(&pr(7)));
}

#[test]
fn a_hand_in_records_when_its_material_was_taken() {
    let root = tempfile::tempdir().unwrap();
    // A reader: the launch of it.
    put(root.path(), running(1, "sid-a"));
    submit(root.path(), 1, &valid(), Some("sid-a")).unwrap();
    assert_eq!(
        load(root.path(), 1).ai.unwrap().read_from,
        Some(utc_stamp(NOW))
    );
    // A person by hand, with no reader launched: the hand-in itself.
    let mut by_hand = record(2);
    by_hand.ai = Some(AiRead {
        status: "queued".to_string(),
        ..blank_ai()
    });
    put(root.path(), by_hand);
    submit(root.path(), 2, &valid(), None).unwrap();
    assert_eq!(
        load(root.path(), 2).ai.unwrap().read_from,
        Some(utc_stamp(NOW + 50))
    );
}

fn reviewed_on(number: u64, commit: &str) -> Record {
    let mut r = running(number, "sid-a");
    r.my_review = Some(MyReview {
        state: ReviewState::Commented,
        submitted_at: "20261002T000000Z".to_string(),
        commit: commit.to_string(),
    });
    r
}

#[test]
fn a_re_review_must_say_what_became_of_the_comments() {
    let root = tempfile::tempdir().unwrap();
    // Reviewed on abc1234, and the reader read another head: sinceReview null is refused.
    put(root.path(), reviewed_on(1, "abc1234"));
    let Err(SubmitError::Invalid(problems)) = submit(root.path(), 1, &valid(), Some("sid-a"))
    else {
        panic!("accepted");
    };
    assert!(
        problems[0].contains("re-review") && problems[0].contains("sinceReview is required"),
        "{problems:?}"
    );
    nothing_changed(root.path(), 1);
    // Reviewed on the head the reader read: there is nothing since, and null is right.
    put(root.path(), reviewed_on(2, SHA));
    submit(root.path(), 2, &valid(), Some("sid-a")).unwrap();
}

#[test]
fn the_comments_of_a_re_review_must_be_those_the_reader_was_given() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), reviewed_on(1, "abc1234"));
    let with = |ids: &[u64]| {
        let mut v = valid();
        v["sinceReview"] = json!({"commit": "abc1234", "newChanges": [],
            "comments": ids.iter().map(|id| json!({"commentId": id, "path": "a", "status": "addressed", "note": "n"})).collect::<Vec<_>>()});
        v
    };
    // With no file (a person by hand) nothing is checked against.
    submit(root.path(), 1, &with(&[999]), None).unwrap();

    put(root.path(), reviewed_on(2, "abc1234"));
    let dir = reader_dir(root.path(), &pr(2));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("my-last-review.json"),
        json!([{"id": 11}, {"id": 12}]).to_string(),
    )
    .unwrap();
    let Err(SubmitError::Invalid(problems)) =
        submit(root.path(), 2, &with(&[11, 13]), Some("sid-a"))
    else {
        panic!("accepted");
    };
    assert!(
        problems[0].contains("commentId 13") && problems[0].contains("my-last-review.json"),
        "{problems:?}"
    );
    nothing_changed(root.path(), 2);
    submit(root.path(), 2, &with(&[11, 12]), Some("sid-a")).unwrap();
}

#[test]
fn a_queued_read_of_a_pr_that_went_done_is_failed_and_takes_no_place() {
    let root = tempfile::tempdir().unwrap();
    for n in 1..=4 {
        put(root.path(), asked(n, "queued", n as i64));
    }
    let mut gone = load(root.path(), 1);
    gone.pr_state = PrOpenState::Merged;
    store::save(root.path(), &mut gone).unwrap();
    let claims = claim_reads(root.path(), NOW, &sessions()).unwrap();
    // Three places, and the merged PR took none of them.
    let ids: Vec<&str> = claims.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["acme/widget#2", "acme/widget#3", "acme/widget#4"]);
    let ai = load(root.path(), 1).ai.unwrap();
    assert_eq!(ai.status, "failed");
    assert_eq!(ai.error.as_deref(), Some("the PR is no longer open"));
}

#[test]
fn a_reader_must_hand_in_the_head_its_pr_json_says() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    let dir = reader_dir(root.path(), &pr(1));
    std::fs::create_dir_all(&dir).unwrap();
    let other = "f".repeat(40);
    std::fs::write(
        dir.join("pr.json"),
        json!({"headRefOid": other}).to_string(),
    )
    .unwrap();
    let Err(SubmitError::Invalid(problems)) = submit(root.path(), 1, &valid(), Some("sid-a"))
    else {
        panic!("accepted");
    };
    assert!(
        problems[0].contains("headRefOid of pr.json") && problems[0].contains(&other),
        "{problems:?}"
    );
    nothing_changed(root.path(), 1);
    // The one it says is taken; a person by hand, with no session, is not held to the file.
    std::fs::write(dir.join("pr.json"), json!({"headRefOid": SHA}).to_string()).unwrap();
    submit(root.path(), 1, &valid(), Some("sid-a")).unwrap();
    put(root.path(), running(2, "sid-b"));
    let dir2 = reader_dir(root.path(), &pr(2));
    std::fs::create_dir_all(&dir2).unwrap();
    std::fs::write(
        dir2.join("pr.json"),
        json!({"headRefOid": other}).to_string(),
    )
    .unwrap();
    submit(root.path(), 2, &valid(), None).unwrap();
}

#[test]
fn every_comment_of_the_last_review_has_exactly_one_entry_and_replies_are_not_entries() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), reviewed_on(1, "abc1234"));
    let dir = reader_dir(root.path(), &pr(1));
    std::fs::create_dir_all(&dir).unwrap();
    // 11 and 13 are the review's comments, 12 answers 11.
    std::fs::write(
        dir.join("my-last-review.json"),
        json!([{"id": 11, "in_reply_to_id": null}, {"id": 12, "in_reply_to_id": 11}, {"id": 13}])
            .to_string(),
    )
    .unwrap();
    let with = |ids: &[u64]| {
        let mut v = valid();
        v["sinceReview"] = json!({"commit": "abc1234", "newChanges": [],
            "comments": ids.iter().map(|id| json!({"commentId": id, "path": "a", "status": "addressed", "note": "n"})).collect::<Vec<_>>()});
        v
    };
    let refused = |ids: &[u64]| match submit(root.path(), 1, &with(ids), Some("sid-a")) {
        Err(SubmitError::Invalid(problems)) => problems,
        other => panic!("{ids:?}: {other:?}"),
    };
    // 13 is missing.
    let problems = refused(&[11]);
    assert!(
        problems[0].contains("no entry for the comment(s) 13"),
        "{problems:?}"
    );
    // 11 twice.
    let problems = refused(&[11, 11, 13]);
    assert!(
        problems[0].contains("more than one entry for the comment(s) 11"),
        "{problems:?}"
    );
    // Nothing written by any of them.
    nothing_changed(root.path(), 1);
    // The top-level ones once each: taken, with or without an entry for the reply.
    submit(root.path(), 1, &with(&[11, 13]), Some("sid-a")).unwrap();
    submit(root.path(), 1, &with(&[11, 12, 13]), Some("sid-a")).unwrap();
}

#[test]
fn the_reason_a_read_failed_is_kept_as_one_short_line() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    let long = format!("first line\n\nsecond   line\n{}", "x".repeat(2000));
    assert_eq!(
        fail_read(root.path(), &pr(1), Some("sid-a"), &long, NOW),
        Ok(true)
    );
    let error = load(root.path(), 1).ai.unwrap().error.unwrap();
    assert!(!error.contains('\n'));
    assert!(error.starts_with("first line second line xxx"), "{error}");
    assert_eq!(error.chars().count(), 300);
    assert!(error.ends_with('…'));
    // A short one is kept as it is, apart from its whitespace.
    put(root.path(), running(2, "sid-b"));
    fail_read(
        root.path(),
        &pr(2),
        Some("sid-b"),
        "  could not\nread  ",
        NOW,
    )
    .unwrap();
    assert_eq!(
        load(root.path(), 2).ai.unwrap().error.as_deref(),
        Some("could not read")
    );
}

// ── the person's drafts ──────────────────────────────────────────────

const STAMP: &str = "20261011T101500Z";

fn drafts_input() -> Value {
    json!({
        "version": 1,
        "fromSha": SHA,
        "drafts": [
            {"key": "r1", "source": "reader", "path": "src/a.rs", "line": 4, "body": "b1"},
            {"key": "p1", "source": "person", "path": "src/a.rs", "body": "a file"},
            {"key": "p2", "source": "person", "body": ""}
        ],
        "overall": {"body": "fine"}
    })
}

fn save(root: &Path, number: u64, input: &Value) -> Result<Value, SaveDraftsError> {
    save_drafts(root, &pr(number), input, STAMP)
}

fn no_drafts_written(root: &Path) {
    assert!(!root.join("others/drafts").exists());
    assert!(!root.join("others/acme~widget~1.lock").exists());
}

#[test]
fn drafts_that_break_the_contract_are_refused_whole_and_leave_nothing() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), record(1));
    let cases: Vec<(&str, Change)> = vec![
        ("an unknown key", Box::new(|v| v["extra"] = json!(1))),
        (
            "an unknown key in a draft",
            Box::new(|v| v["drafts"][0]["note"] = json!("x")),
        ),
        (
            "the server's own keys",
            Box::new(|v| v["id"] = json!("acme/widget#1")),
        ),
        (
            "savedAt",
            Box::new(|v| v["savedAt"] = json!("20261011T101500Z")),
        ),
        ("a wrong version", Box::new(|v| v["version"] = json!(2))),
        ("a short fromSha", Box::new(|v| v["fromSha"] = json!("abc"))),
        (
            "an unknown fromMode",
            Box::new(|v| v["fromMode"] = json!("summary")),
        ),
        (
            "a bad source",
            Box::new(|v| v["drafts"][0]["source"] = json!("ai")),
        ),
        (
            "a bad key",
            Box::new(|v| v["drafts"][0]["key"] = json!("a b")),
        ),
        (
            "an empty key",
            Box::new(|v| v["drafts"][0]["key"] = json!("")),
        ),
        (
            "a long key",
            Box::new(|v| v["drafts"][0]["key"] = json!("k".repeat(65))),
        ),
        (
            "a repeated key",
            Box::new(|v| v["drafts"][1]["key"] = json!("r1")),
        ),
        (
            "a line without a path",
            Box::new(|v| v["drafts"][2]["line"] = json!(3)),
        ),
        (
            "a startLine without a line",
            Box::new(|v| v["drafts"][1]["startLine"] = json!(1)),
        ),
        (
            "a startLine after its line",
            Box::new(|v| v["drafts"][0]["startLine"] = json!(9)),
        ),
        ("line 0", Box::new(|v| v["drafts"][0]["line"] = json!(0))),
        (
            "an absolute path",
            Box::new(|v| v["drafts"][0]["path"] = json!("/src/a.rs")),
        ),
        (
            "a blank path",
            Box::new(|v| v["drafts"][0]["path"] = json!(" ")),
        ),
        (
            "a long path",
            Box::new(|v| v["drafts"][0]["path"] = json!("p".repeat(1025))),
        ),
        (
            "a bad side",
            Box::new(|v| v["drafts"][0]["side"] = json!("RIGHT")),
        ),
        (
            "an over-long body",
            Box::new(|v| v["drafts"][0]["body"] = json!("x".repeat(20_001))),
        ),
        (
            "an over-long overall",
            Box::new(|v| v["overall"]["body"] = json!("x".repeat(20_001))),
        ),
        (
            "301 drafts",
            Box::new(|v| {
                v["drafts"] = json!(
                    (0..301)
                        .map(|i| json!({"key": format!("k{i}"), "source": "person", "body": ""}))
                        .collect::<Vec<_>>()
                )
            }),
        ),
        (
            "too big together",
            Box::new(|v| {
                v["drafts"] = json!(
                    (0..30)
                        .map(|i| json!({"key": format!("k{i}"), "source": "person",
                                        "body": "x".repeat(20_000)}))
                        .collect::<Vec<_>>()
                )
            }),
        ),
        ("not an object", Box::new(|v| *v = json!([]))),
    ];
    for (what, change) in &cases {
        let mut v = drafts_input();
        change(&mut v);
        let err = save(root.path(), 1, &v).unwrap_err();
        assert!(
            matches!(err, SaveDraftsError::Invalid(ref p) if !p.is_empty()),
            "{what}: {err:?}"
        );
        no_drafts_written(root.path());
    }
}

#[test]
fn every_problem_with_the_drafts_is_reported_in_one_answer() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), record(1));
    let mut v = drafts_input();
    v["drafts"][0]["line"] = json!(0);
    v["drafts"][1]["key"] = json!("r1");
    v["overall"]["body"] = json!("x".repeat(20_001));
    let Err(SaveDraftsError::Invalid(problems)) = save(root.path(), 1, &v) else {
        panic!("accepted");
    };
    assert_eq!(problems.len(), 3, "{problems:?}");
}

#[test]
fn drafts_of_a_pr_with_no_record_are_refused() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        save(root.path(), 1, &drafts_input()),
        Err(SaveDraftsError::NoRecord)
    );
    no_drafts_written(root.path());
}

#[test]
fn drafts_are_saved_whole_and_read_back_with_the_servers_keys() {
    let root = tempfile::tempdir().unwrap();
    // A done record may still be saved.
    let mut done = record(1);
    done.pr_state = PrOpenState::Merged;
    put(root.path(), done);
    assert_eq!(drafts(root.path(), &pr(1)).unwrap(), None);
    let saved = save(root.path(), 1, &drafts_input()).unwrap();
    assert_eq!(saved["id"], "acme/widget#1");
    assert_eq!(saved["savedAt"], STAMP);
    assert_eq!(saved["fromSha"], SHA);
    assert!(saved.get("fromMode").is_none());
    assert_eq!(saved["drafts"][0]["side"], "right");
    assert_eq!(saved["drafts"][2]["body"], "");
    assert_eq!(saved["overall"], json!({"body": "fine"}));
    assert_eq!(drafts(root.path(), &pr(1)).unwrap(), Some(saved));
    // A save replaces the file, and the absent version is 1.
    let saved = save(root.path(), 1, &json!({"drafts": []})).unwrap();
    assert_eq!(
        (saved["version"].clone(), saved["drafts"].clone()),
        (json!(1), json!([]))
    );
    assert_eq!(saved["overall"], Value::Null);
    assert_eq!(drafts(root.path(), &pr(1)).unwrap(), Some(saved));
}

#[test]
fn a_key_a_newer_binary_added_to_the_drafts_survives_a_read() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), record(1));
    let path = root.path().join("others/drafts/acme~widget~1.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        json!({"id": "acme/widget#1", "savedAt": STAMP, "version": 1, "drafts": [],
               "pending": {"reviewId": 77}})
        .to_string(),
    )
    .unwrap();
    let read = drafts(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(read["pending"], json!({"reviewId": 77}));
}

#[test]
fn drafts_that_cannot_be_read_are_an_error_and_not_none() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), record(1));
    let path = root.path().join("others/drafts/acme~widget~1.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{not json").unwrap();
    assert!(drafts(root.path(), &pr(1)).is_err());
}

#[test]
fn a_removed_record_takes_its_drafts_with_it_and_leaves_the_others() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), record(1));
    put(root.path(), record(2));
    save(root.path(), 1, &drafts_input()).unwrap();
    save(root.path(), 2, &drafts_input()).unwrap();
    store::remove(root.path(), &store::stem(&pr(1))).unwrap();
    assert_eq!(drafts(root.path(), &pr(1)).unwrap(), None);
    assert!(drafts(root.path(), &pr(2)).unwrap().is_some());
    // The directory of drafts is not a record to the list.
    assert_eq!(list(root.path()).len(), 1);
}

#[test]
fn one_record_is_read_apart_from_one_that_is_missing_or_broken() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(super::record(root.path(), &pr(1)), Ok(None));
    put(root.path(), record(1));
    assert_eq!(
        super::record(root.path(), &pr(1)).unwrap().unwrap().number,
        1
    );
    std::fs::write(root.path().join("others/acme~widget~1.json"), "{not json").unwrap();
    assert!(super::record(root.path(), &pr(1)).is_err());
}

#[test]
fn a_key_an_older_page_does_not_send_survives_a_save() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), record(1));
    let path = root.path().join("others/drafts/acme~widget~1.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        json!({"id": "acme/widget#1", "savedAt": "old", "version": 1, "drafts": [],
               "pending": {"reviewId": 77}})
        .to_string(),
    )
    .unwrap();
    let saved = save(root.path(), 1, &drafts_input()).unwrap();
    assert_eq!(saved["pending"], json!({"reviewId": 77}));
    assert_eq!(saved["savedAt"], STAMP);
    assert_eq!(saved["drafts"].as_array().unwrap().len(), 3);
    let read = drafts(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(read["pending"], json!({"reviewId": 77}));
    // A file that cannot be read fails the save and is left as it is.
    std::fs::write(&path, "{not json").unwrap();
    assert!(matches!(
        save(root.path(), 1, &drafts_input()),
        Err(SaveDraftsError::Failed(_))
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{not json");
}

// ── the diff kept with a read-through ────────────────────────────────

const PATCH: &str =
    "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+b\n";

/// The reader's material for PR `number`: a `pr.json` of `head`, and a `diff.patch` when given.
fn material(root: &Path, number: u64, head: &str, patch: Option<&[u8]>) {
    let dir = reader_dir(root, &pr(number));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pr.json"), json!({"headRefOid": head}).to_string()).unwrap();
    if let Some(patch) = patch {
        std::fs::write(dir.join("diff.patch"), patch).unwrap();
    }
}

fn diff_text(root: &Path, number: u64) -> Option<String> {
    store::load_diff(root, &store::stem(&pr(number))).unwrap()
}

#[test]
fn a_diff_is_kept_when_pr_json_says_the_head_that_was_read() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    material(root.path(), 1, SHA, Some(PATCH.as_bytes()));
    submit(root.path(), 1, &valid(), Some("sid-a")).unwrap();
    let diff = stored_diff(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(diff["id"], "acme/widget#1");
    assert_eq!(diff["sha"], SHA);
    assert_eq!(diff["bytes"], PATCH.len());
    assert_eq!(diff["diff"], PATCH);
}

#[test]
fn an_empty_diff_is_an_answer_and_not_a_missing_one() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    material(root.path(), 1, SHA, Some(b""));
    submit(root.path(), 1, &valid(), Some("sid-a")).unwrap();
    let diff = stored_diff(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(
        (diff["bytes"].clone(), diff["diff"].clone()),
        (json!(0), json!(""))
    );
}

#[test]
fn a_diff_over_the_cap_keeps_its_size_and_no_text() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    let big = vec![b'x'; MAX_DIFF_BYTES + 1];
    material(root.path(), 1, SHA, Some(&big));
    submit(root.path(), 1, &valid(), Some("sid-a")).unwrap();
    let diff = stored_diff(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(diff["bytes"], MAX_DIFF_BYTES + 1);
    assert_eq!(diff["diff"], Value::Null);
    // Exactly at the cap is kept.
    let at = vec![b'x'; MAX_DIFF_BYTES];
    material(root.path(), 1, SHA, Some(&at));
    put(root.path(), running(1, "sid-a"));
    submit(root.path(), 1, &valid(), Some("sid-a")).unwrap();
    let diff = stored_diff(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(diff["diff"].as_str().map(str::len), Some(MAX_DIFF_BYTES));
}

#[test]
fn no_diff_is_kept_and_an_old_one_goes_when_the_head_is_not_known_to_be_the_one_read() {
    let root = tempfile::tempdir().unwrap();
    let other = "f".repeat(40);
    // A diff of an earlier hand-in is there for each case.
    let earlier = |number: u64| {
        put(root.path(), running(number, "sid-a"));
        material(root.path(), number, SHA, Some(PATCH.as_bytes()));
        submit(root.path(), number, &valid(), Some("sid-a")).unwrap();
        assert!(diff_text(root.path(), number).is_some());
        put(root.path(), running(number, "sid-a"));
    };
    // A person's hand-in whose pr.json is of another head.
    earlier(1);
    material(root.path(), 1, &other, Some(PATCH.as_bytes()));
    submit(root.path(), 1, &valid(), None).unwrap();
    assert_eq!(diff_text(root.path(), 1), None);
    assert_eq!(stored_diff(root.path(), &pr(1)).unwrap(), None);
    // No pr.json at all.
    earlier(2);
    std::fs::remove_file(reader_dir(root.path(), &pr(2)).join("pr.json")).unwrap();
    submit(root.path(), 2, &valid(), Some("sid-a")).unwrap();
    assert_eq!(diff_text(root.path(), 2), None);
    // No diff.patch: GitHub would not give it.
    earlier(3);
    std::fs::remove_file(reader_dir(root.path(), &pr(3)).join("diff.patch")).unwrap();
    submit(root.path(), 3, &valid(), Some("sid-a")).unwrap();
    assert_eq!(diff_text(root.path(), 3), None);
    assert_eq!(stored_diff(root.path(), &pr(3)).unwrap(), None);
}

#[test]
fn a_diff_patch_that_cannot_be_read_refuses_the_hand_in_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    material(root.path(), 1, SHA, None);
    // A directory where the file should be: it is there, and cannot be read.
    std::fs::create_dir(reader_dir(root.path(), &pr(1)).join("diff.patch")).unwrap();
    let err = submit(root.path(), 1, &valid(), Some("sid-a")).unwrap_err();
    assert!(matches!(err, SubmitError::Failed(_)), "{err:?}");
    nothing_changed(root.path(), 1);
    assert_eq!(diff_text(root.path(), 1), None);
}

#[cfg(unix)]
#[test]
fn a_diff_patch_that_is_a_link_refuses_the_hand_in_like_one_that_cannot_be_read() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    material(root.path(), 1, SHA, None);
    let dir = reader_dir(root.path(), &pr(1));
    let elsewhere = root.path().join("elsewhere.patch");
    std::fs::write(&elsewhere, PATCH).unwrap();
    // To a file that is there, and to one that is not.
    for target in [elsewhere.clone(), root.path().join("nothing")] {
        let _ = std::fs::remove_file(dir.join("diff.patch"));
        std::os::unix::fs::symlink(&target, dir.join("diff.patch")).unwrap();
        let err = submit(root.path(), 1, &valid(), Some("sid-a")).unwrap_err();
        assert!(matches!(err, SubmitError::Failed(_)), "{err:?}");
        nothing_changed(root.path(), 1);
        assert_eq!(diff_text(root.path(), 1), None);
    }
}

#[test]
fn a_diff_that_is_not_utf8_is_kept_with_its_size() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    material(root.path(), 1, SHA, Some(b"+caf\xe9\n"));
    submit(root.path(), 1, &valid(), Some("sid-a")).unwrap();
    let diff = stored_diff(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(diff["bytes"], 6);
    assert_eq!(diff["diff"], "+caf\u{fffd}\n");
}

#[test]
fn a_record_that_cannot_be_saved_puts_back_the_previous_read_and_diff() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    material(root.path(), 1, SHA, Some(PATCH.as_bytes()));
    let failing = |_: &mut Record| Err("disk full".to_string());
    // With nothing before: nothing is left.
    let err = submit_read_with(
        root.path(),
        &pr(1),
        &valid().to_string(),
        Some("sid-a"),
        NOW,
        &failing,
    )
    .unwrap_err();
    assert_eq!(err, SubmitError::Failed("disk full".to_string()));
    nothing_changed(root.path(), 1);
    assert_eq!(diff_text(root.path(), 1), None);

    // With a read and diff before: both come back as they were, though the new hand-in has none.
    submit(root.path(), 1, &valid(), Some("sid-a")).unwrap();
    let (read_before, diff_before) = (
        store::load_read(root.path(), &store::stem(&pr(1))).unwrap(),
        diff_text(root.path(), 1),
    );
    assert!(read_before.is_some() && diff_before.is_some());
    std::fs::remove_file(reader_dir(root.path(), &pr(1)).join("diff.patch")).unwrap();
    let mut other = valid();
    other["three"]["essence"] = json!("another");
    let err = submit_read_with(
        root.path(),
        &pr(1),
        &other.to_string(),
        Some("sid-a"),
        NOW + 9,
        &failing,
    )
    .unwrap_err();
    assert!(matches!(err, SubmitError::Failed(_)));
    assert_eq!(
        store::load_read(root.path(), &store::stem(&pr(1))).unwrap(),
        read_before
    );
    assert_eq!(diff_text(root.path(), 1), diff_before);
}

#[test]
fn a_diff_of_another_head_than_the_read_is_not_served() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    material(root.path(), 1, SHA, Some(PATCH.as_bytes()));
    submit(root.path(), 1, &valid(), Some("sid-a")).unwrap();
    let path = root.path().join("others/diffs/acme~widget~1.json");
    let mut stored: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    stored["sha"] = json!("f".repeat(40));
    std::fs::write(&path, stored.to_string()).unwrap();
    assert_eq!(stored_diff(root.path(), &pr(1)).unwrap(), None);
    // A file that cannot be read is an error, not "no diff".
    std::fs::write(&path, "{not json").unwrap();
    assert!(stored_diff(root.path(), &pr(1)).is_err());
}

#[test]
fn a_removed_record_takes_its_diff_with_it_and_a_list_does_not_read_diffs_as_records() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), running(1, "sid-a"));
    material(root.path(), 1, SHA, Some(PATCH.as_bytes()));
    submit(root.path(), 1, &valid(), Some("sid-a")).unwrap();
    assert!(diff_text(root.path(), 1).is_some());
    let listed = store::list(root.path());
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "acme/widget#1");
    store::remove(root.path(), &store::stem(&pr(1))).unwrap();
    assert_eq!(diff_text(root.path(), 1), None);
    assert!(store::list(root.path()).is_empty());
}

// ── a split proposal ─────────────────────────────────────────────────

fn split_asked(number: u64, session: &str) -> Record {
    let mut r = running(number, session);
    r.ai.as_mut().unwrap().mode = Some("split".to_string());
    r
}

#[test]
fn asking_for_a_split_sets_the_mode_a_read_clears_it_and_a_takeover_takes_the_new_one() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), record(1));
    let Requested::Queued(r) = request_read(root.path(), &pr(1), NOW, ReadMode::Split).unwrap()
    else {
        panic!("not queued");
    };
    assert_eq!(r.ai.as_ref().unwrap().mode.as_deref(), Some("split"));
    assert_eq!(
        load(root.path(), 1).ai.unwrap().mode.as_deref(),
        Some("split")
    );
    // Asking while it is queued answers Already and changes nothing, the mode included.
    let Requested::Already(r) = request_read(root.path(), &pr(1), NOW + 5, ReadMode::Read).unwrap()
    else {
        panic!("not already");
    };
    assert_eq!(r.ai.unwrap().mode.as_deref(), Some("split"));
    // A read asked for after the proposal is over clears the mode.
    let mut ready = load(root.path(), 1);
    ready.ai.as_mut().unwrap().status = "ready".to_string();
    put(root.path(), ready);
    let Requested::Queued(r) = request_read(root.path(), &pr(1), NOW + 9, ReadMode::Read).unwrap()
    else {
        panic!("not queued");
    };
    assert_eq!(r.ai.unwrap().mode, None);
    // A takeover of a hung reader queues the mode of the new request.
    put(root.path(), running(2, "sid-old"));
    let late = NOW + HUNG_READER_SECS + 10;
    let Ok(Requested::Queued(r)) =
        take_over_read(root.path(), &pr(2), late, "sid-old", ReadMode::Split)
    else {
        panic!("not taken over");
    };
    assert_eq!(r.ai.unwrap().mode.as_deref(), Some("split"));
    put(root.path(), split_asked(3, "sid-old"));
    let Ok(Requested::Queued(r)) =
        take_over_read(root.path(), &pr(3), late, "sid-old", ReadMode::Read)
    else {
        panic!("not taken over");
    };
    assert_eq!(r.ai.unwrap().mode, None);
}

#[test]
fn the_mode_survives_what_a_reader_does_to_its_read_and_is_read_from_the_record() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), split_asked(1, "sid-a"));
    note_reader(root.path(), &pr(1), "sid-a", 42, None, None).unwrap();
    assert_eq!(
        load(root.path(), 1).ai.unwrap().mode.as_deref(),
        Some("split")
    );
    fail_read(root.path(), &pr(1), Some("sid-a"), "boom", NOW).unwrap();
    assert_eq!(
        load(root.path(), 1).ai.unwrap().mode.as_deref(),
        Some("split")
    );
    // A mode this binary does not know is a read-through, and a record with one keeps it.
    let path = root.path().join("others/acme~widget~1.json");
    let mut raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    raw["ai"]["mode"] = json!("summary");
    raw["ai"]["futureKey"] = json!(7);
    std::fs::write(&path, raw.to_string()).unwrap();
    let r = load(root.path(), 1);
    assert_eq!(ReadMode::of(r.ai.as_ref()), ReadMode::Read);
    put(root.path(), r);
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["ai"]["mode"], "summary");
    assert_eq!(raw["ai"]["futureKey"], 7);
}

#[test]
fn a_mode_on_the_wire_is_read_split_or_absent_and_nothing_else() {
    assert_eq!(ReadMode::from_wire(None), Some(ReadMode::Read));
    assert_eq!(ReadMode::from_wire(Some("read")), Some(ReadMode::Read));
    assert_eq!(ReadMode::from_wire(Some("split")), Some(ReadMode::Split));
    for bad in ["", "Split", "summary", " split"] {
        assert_eq!(ReadMode::from_wire(Some(bad)), None, "{bad:?}");
    }
}

const BIG_DIFF: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,2 @@ fn a() {
 keep
-old
+new
@@ -40 +40,2 @@ impl B {
-gone
+x
+y
\\ No newline at end of file
diff --git a/src/b.rs b/src/b.rs
new file mode 100644
index 000..333
--- /dev/null
+++ b/src/b.rs
@@ -0,0 +1,2 @@
+first
+second
diff --git a/old.bin b/new.bin
similarity index 90%
rename from old.bin
rename to new.bin
Binary files a/old.bin and b/new.bin differ
";

#[test]
fn hunk_headers_keeps_the_headers_and_none_of_the_lines() {
    let kept = hunk_headers(BIG_DIFF);
    assert_eq!(
        kept,
        "\
diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,2 @@ fn a() {
@@ -40 +40,2 @@ impl B {
diff --git a/src/b.rs b/src/b.rs
new file mode 100644
index 000..333
--- /dev/null
+++ b/src/b.rs
@@ -0,0 +1,2 @@
diff --git a/old.bin b/new.bin
similarity index 90%
rename from old.bin
rename to new.bin
Binary files a/old.bin and b/new.bin differ
"
    );
}

#[test]
fn hunk_headers_reads_a_hunk_by_its_counts_not_by_what_a_line_looks_like() {
    // A deleted line that starts `-- ` shows as `--- `, an added `++ ` as `+++ `, and an added line
    // can start `diff --git`: all are lines of the hunk, and none is a header.
    let diff = "\
diff --git a/x.sql b/x.sql
--- a/x.sql
+++ b/x.sql
@@ -1,2 +1,3 @@
--- a comment
+++ b comment
+diff --git a/fake b/fake
 context
@@ -9,2 +9,0 @@
-a
-b
";
    let kept = hunk_headers(diff);
    assert!(!kept.contains("a comment"), "{kept}");
    assert!(!kept.contains("b comment"), "{kept}");
    assert!(!kept.contains("fake"), "{kept}");
    assert_eq!(
        kept,
        "diff --git a/x.sql b/x.sql\n--- a/x.sql\n+++ b/x.sql\n@@ -1,2 +1,3 @@\n@@ -9,2 +9,0 @@\n"
    );
    // A missing count is one line, a stripped blank context line is still one, and nothing panics on a cut diff.
    let one = "@@ -5 +5 @@\n-a\n+b\n@@ -7,2 +7,2 @@\n\n x\n--- file after\n";
    assert_eq!(
        hunk_headers(one),
        "@@ -5 +5 @@\n@@ -7,2 +7,2 @@\n--- file after\n"
    );
    // A diff that ends inside a hunk says so.
    assert_eq!(
        hunk_headers("@@ -1,5 +1,5 @@\n a\n"),
        "@@ -1,5 +1,5 @@\n# note: the diff was cut inside a hunk; hunks.patch may be incomplete\n"
    );
    assert_eq!(hunk_headers(""), "");
}

fn split_material_for(
    number: u64,
    gh: &Gh,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Result<ContextSummary, String>,
) {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), split_asked(number, "sid-a"));
    let out = reader_dir(root.path(), &pr(number));
    let got = read_context_with(root.path(), &pr(number), &out, Some("sid-a"), &|args, _| {
        gh.run(args)
    });
    (root, out, got)
}

#[test]
fn the_material_of_a_split_is_pr_json_and_hunks_and_nothing_of_the_diff_or_the_comments() {
    let mut gh = Gh::new();
    gh.diff_text = Some(BIG_DIFF.to_string());
    let root = tempfile::tempdir().unwrap();
    put(root.path(), reviewed_record("abc1234"));
    let mut r = load(root.path(), 1);
    r.ai = split_asked(1, "sid-a").ai;
    put(root.path(), r);
    // What an earlier read left behind is gone.
    let out = own(root.path());
    std::fs::create_dir_all(&out).unwrap();
    for name in [
        "diff.patch",
        "review-comments.json",
        "reviews.json",
        "my-last-review.json",
        "since-review.patch",
    ] {
        std::fs::write(out.join(name), "old").unwrap();
    }
    let summary = read_context_with(root.path(), &pr(1), &out, Some("sid-a"), &|args, _| {
        gh.run(args)
    })
    .unwrap();
    assert_eq!(summary.mode, "split");
    assert!(!summary.rereview);
    assert_eq!(summary.written, ["pr.json", "hunks.patch"]);
    assert_eq!(
        std::fs::read_to_string(out.join("hunks.patch")).unwrap(),
        hunk_headers(BIG_DIFF)
    );
    for name in [
        "diff.patch",
        "review-comments.json",
        "reviews.json",
        "my-last-review.json",
        "since-review.patch",
    ] {
        assert!(!out.join(name).exists(), "{name}");
    }
    // The comments and the reviews are not even asked for.
    assert!(
        gh.calls()
            .iter()
            .all(|c| !c.join(" ").contains("/comments") && !c.join(" ").contains("/reviews"))
    );
    // pr.json is the usual one, with the head the hand-in is checked against.
    let pr_json: Value =
        serde_json::from_slice(&std::fs::read(out.join("pr.json")).unwrap()).unwrap();
    assert_eq!(pr_json["headRefOid"], SHA);
}

#[test]
fn a_read_removes_the_hunks_of_an_earlier_split_and_says_read() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), record(1));
    let out = own(root.path());
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("hunks.patch"), "old").unwrap();
    let summary = context(root.path(), &Gh::new(), &out).unwrap();
    assert_eq!(summary.mode, "read");
    assert!(!out.join("hunks.patch").exists());
}

#[test]
fn when_gh_will_not_give_the_diff_the_hunks_come_from_the_files_list() {
    let mut gh = Gh::new();
    gh.diff_fails = true;
    gh.files_answer = Some(
        json!([{"filename": "src/a.rs", "status": "modified", "additions": 3, "deletions": 1,
                "patch": "@@ -1,3 +1,2 @@ fn a()\n-x\n+y\n ctx\n--- not a header\n"},
               {"filename": "src/new.rs", "previous_filename": "src/old.rs", "status": "renamed",
                "additions": 0, "deletions": 0}])
        .to_string()
            + &json!([{"filename": "src/huge.rs", "status": "added", "additions": 9000, "deletions": 0}])
                .to_string(),
    );
    let (_root, out, got) = split_material_for(1, &gh);
    let summary = got.unwrap();
    let hunks = std::fs::read_to_string(out.join("hunks.patch")).unwrap();
    assert_eq!(
        hunks,
        "diff --git a/src/a.rs b/src/a.rs\n# status: modified +3 -1\n@@ -1,3 +1,2 @@ fn a()\n\
         diff --git a/src/old.rs b/src/new.rs\nrename from src/old.rs\nrename to src/new.rs\n\
         # status: renamed +0 -0\n\
         diff --git a/src/huge.rs b/src/huge.rs\n# status: added +9000 -0\n"
    );
    assert!(
        summary
            .notes
            .iter()
            .any(|n| n == "hunk headers of src/huge.rs are not available")
    );
    assert!(
        summary
            .notes
            .iter()
            .any(|n| n.contains("made from the files list"))
    );
    // Only GET requests, as everywhere else.
    assert!(gh.calls().iter().all(|c| !c.contains(&"-X".to_string())));
    assert!(
        gh.calls()
            .iter()
            .any(|c| c.join(" ").contains("--paginate") && c.join(" ").ends_with("/files"))
    );
}

#[test]
fn when_neither_the_diff_nor_the_files_list_comes_the_split_has_a_note_and_no_hunks() {
    let mut gh = Gh::new();
    gh.diff_fails = true;
    let (_root, out, got) = split_material_for(1, &gh);
    let summary = got.unwrap();
    assert!(!out.join("hunks.patch").exists());
    assert_eq!(summary.written, ["pr.json"]);
    assert!(
        summary
            .notes
            .iter()
            .any(|n| n.contains("hunks.patch could not be made"))
    );
}

fn split_read() -> Value {
    json!({
        "version": 1,
        "sha": SHA,
        "three": {"purpose": "p", "approach": "a", "impact": "i", "essence": "e"},
        "overall": {"body": "PR 1: the schema\nPR 2: the code"}
    })
}

#[test]
fn a_split_proposal_is_accepted_stored_as_split_and_keeps_no_diff() {
    let root = tempfile::tempdir().unwrap();
    // The record has an earlier read-through and its diff, and a review of the person's on an older head.
    let mut r = split_asked(1, "sid-a");
    r.my_review = Some(MyReview {
        state: ReviewState::Commented,
        submitted_at: "20261002T000000Z".to_string(),
        commit: "abc1234".to_string(),
    });
    put(root.path(), r);
    store::write_diff(
        root.path(),
        &store::stem(&pr(1)),
        &StoredDiff {
            sha: SHA.to_string(),
            saved_at: "s".to_string(),
            bytes: 1,
            diff: Some("old".to_string()),
        },
    )
    .unwrap();
    material(root.path(), 1, SHA, None);
    // No sinceReview needed: a proposal compares nothing with the earlier review.
    submit(root.path(), 1, &split_read(), Some("sid-a")).unwrap();
    let stored = read_through(root.path(), &pr(1)).unwrap().unwrap();
    assert_eq!(stored["mode"], "split");
    assert_eq!(
        stored["overall"]["body"],
        "PR 1: the schema\nPR 2: the code"
    );
    assert_eq!(diff_text(root.path(), 1), None);
    let ai = load(root.path(), 1).ai.unwrap();
    assert_eq!(ai.status, "ready");
    assert_eq!(ai.mode.as_deref(), Some("split"));
    assert_eq!(
        ai.counts.map(|c| (c.attention, c.check, c.drafts)),
        Some((0, 0, 0))
    );
}

#[test]
fn a_split_proposal_that_is_not_one_is_refused_whole() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), split_asked(1, "sid-a"));
    let cases: Vec<(&str, Change, &str)> = vec![
        (
            "no overall",
            Box::new(|v| {
                v.as_object_mut().unwrap().remove("overall");
            }),
            "overall.body",
        ),
        (
            "blank overall",
            Box::new(|v| v["overall"]["body"] = json!("  \n")),
            "must not be blank",
        ),
        (
            "a draft",
            Box::new(|v| v["drafts"] = json!([{"path": "a.rs", "line": 1, "body": "b"}])),
            "drafts must be []",
        ),
        (
            "an annotation",
            Box::new(|v| {
                v["annotations"] =
                    json!([{"path": "a.rs", "anchor": "x", "kind": "tr", "text": "t"}])
            }),
            "annotations must be []",
        ),
        (
            "a sinceReview",
            Box::new(|v| v["sinceReview"] = json!({"commit": "abc1234"})),
            "sinceReview must be null",
        ),
        (
            "a point",
            Box::new(|v| {
                v["points"] = json!([{"level": "info", "path": "a.rs", "title": "t", "why": "w"}])
            }),
            "points must be empty",
        ),
        (
            "an order",
            Box::new(|v| v["order"] = json!([{"path": "a.rs", "category": "logic"}])),
            "order must be empty",
        ),
        (
            "a deleted file",
            Box::new(|v| v["deleted"] = json!([{"path": "a.rs", "reason": "r", "question": null}])),
            "deleted must be empty",
        ),
        (
            "a decision",
            Box::new(|v| v["decisions"] = json!([{"decision": "d", "gains": "g", "costs": "c"}])),
            "decisions must be empty",
        ),
        (
            "an unwritten",
            Box::new(|v| v["unwritten"] = json!(["q"])),
            "unwritten must be empty",
        ),
        (
            "a test",
            Box::new(|v| v["tests"] = json!({"covered": ["x"], "notCovered": []})),
            "tests must be empty",
        ),
        (
            "a suggestion",
            Box::new(|v| v["overall"]["suggestion"] = json!("Request changes")),
            "overall.suggestion must be left out",
        ),
    ];
    for (what, change, says) in cases {
        let mut v = split_read();
        change(&mut v);
        let Err(SubmitError::Invalid(problems)) = submit(root.path(), 1, &v, Some("sid-a")) else {
            panic!("{what} accepted");
        };
        assert!(
            problems.iter().any(|p| p.contains(says)),
            "{what}: {problems:?}"
        );
        nothing_changed(root.path(), 1);
    }
}

#[test]
fn a_read_through_is_not_held_to_the_proposal_and_a_proposal_is_what_the_record_says_it_is() {
    let root = tempfile::tempdir().unwrap();
    // The record asked for a read: a file with drafts and no overall is fine.
    put(root.path(), running(1, "sid-a"));
    let mut v = valid();
    v["overall"] = json!(null);
    submit(root.path(), 1, &v, Some("sid-a")).unwrap();
    let stored = read_through(root.path(), &pr(1)).unwrap().unwrap();
    assert!(stored.get("mode").is_none());
}

#[test]
fn the_mode_of_the_read_the_drafts_came_from_is_kept_when_given() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), record(1));
    let mut input = drafts_input();
    input["fromMode"] = json!("split");
    let saved = save(root.path(), 1, &input).unwrap();
    assert_eq!(saved["fromMode"], "split");
    assert_eq!(drafts(root.path(), &pr(1)).unwrap(), Some(saved));
    // A save without it is a file of a read-through, and the old key does not linger.
    let saved = save(root.path(), 1, &drafts_input()).unwrap();
    assert!(saved.get("fromMode").is_none());
}
