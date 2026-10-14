use super::close_ended_pr_readers::close_ended_pr_readers_with;
use super::close_reader::close_reader_with;
use super::close_readers::{close_finished_readers_with, frees_a_place, has_finished_reader};
use super::close_task_readers::{close_task_readers_with, has_task_reader};
use super::dispatch::dispatch_with;
use super::launch_reader::launch_reader_with;
use super::request::request_with;
use super::request_for_task::request_for_task_with;
use super::*;
use crate::infra::terminal::Performed;
use crate::others::{self, Claim, ReadMode, Requested};
use crate::registry::{self, Context, Liveness};
use crate::testing::Sandbox;

use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

const NOW: i64 = 1_791_600_000;

fn put(root: &Path, number: u64, status: Option<&str>, asked: &str) {
    let mut record = json!({
        "id": format!("acme/widget#{number}"), "repo": "Acme/Widget", "number": number,
        "url": format!("https://github.com/acme/widget/pull/{number}"),
        "registered": true, "title": "t", "base": "main", "head": "f", "headSha": "h1",
        "prState": "open", "additions": 1, "deletions": 1, "changedFiles": 1,
        "requested": true, "requestedAt": "20261001T000000Z",
        "firstSeenAt": "20261001T000000Z", "readAt": "20261001T000000Z"
    });
    if let Some(status) = status {
        record["ai"] = json!({"status": status, "startedAt": asked});
    }
    let dir = root.join("others");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("acme~widget~{number}.json")),
        record.to_string(),
    )
    .unwrap();
}

fn status_of(root: &Path, number: u64) -> String {
    let path = root.join(format!("others/acme~widget~{number}.json"));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["ai"]["status"].as_str().unwrap().to_string()
}

fn error_of(root: &Path, number: u64) -> Option<String> {
    let path = root.join(format!("others/acme~widget~{number}.json"));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["ai"]["error"].as_str().map(str::to_string)
}

fn setup() -> (Sandbox, Context) {
    let sandbox = Sandbox::empty();
    let ctx = registry::review_context(&sandbox.state()).unwrap();
    (sandbox, ctx)
}

fn gone(_: u32, _: Option<&str>) -> Liveness {
    Liveness::Gone
}

fn sessions() -> impl Fn() -> Result<String, String> {
    let n = std::cell::Cell::new(0);
    move || {
        n.set(n.get() + 1);
        Ok(format!("sid-{}", n.get()))
    }
}

#[test]
fn nothing_starts_while_the_hub_is_not_up() {
    let (sandbox, ctx) = setup();
    put(&sandbox.state(), 1, Some("queued"), "20261001T000001Z");
    let launched = RefCell::new(Vec::new());
    let done = dispatch_with(
        &ctx,
        NOW,
        &|| false,
        &gone,
        &sessions(),
        &|_, claim: &Claim| {
            launched.borrow_mut().push(claim.id.clone());
            Ok(())
        },
    );
    assert_eq!(done, Default::default());
    assert!(launched.borrow().is_empty());
    assert_eq!(status_of(&sandbox.state(), 1), "queued");
}

#[test]
fn with_nothing_queued_or_running_the_hub_is_not_even_asked() {
    let (sandbox, ctx) = setup();
    put(&sandbox.state(), 1, None, "");
    put(&sandbox.state(), 2, Some("ready"), "20261001T000001Z");
    let done = dispatch_with(
        &ctx,
        NOW,
        &|| panic!("a ps for nothing"),
        &gone,
        &sessions(),
        &|_, _| panic!("nothing to start"),
    );
    assert_eq!(done, Default::default());
}

#[test]
fn at_most_three_start_in_order_and_each_is_launched() {
    let (sandbox, ctx) = setup();
    let root = sandbox.state();
    for (n, at) in [
        (1, "20261001T000004Z"),
        (2, "20261001T000001Z"),
        (3, "20261001T000003Z"),
        (4, "20261001T000002Z"),
    ] {
        put(&root, n, Some("queued"), at);
    }
    let launched = RefCell::new(Vec::new());
    let done = dispatch_with(
        &ctx,
        NOW,
        &|| true,
        &gone,
        &sessions(),
        &|_, claim: &Claim| {
            launched
                .borrow_mut()
                .push((claim.id.clone(), claim.session_id.clone()));
            Ok(())
        },
    );
    assert_eq!(
        done.started,
        ["acme/widget#2", "acme/widget#4", "acme/widget#3"]
    );
    assert_eq!(launched.borrow().len(), 3);
    assert_eq!(
        launched.borrow()[0],
        ("acme/widget#2".to_string(), "sid-1".to_string())
    );
    assert_eq!(status_of(&root, 1), "queued");
    assert_eq!(status_of(&root, 2), "running");
}

#[test]
fn a_reader_that_cannot_be_started_fails_its_read_under_its_session() {
    let (sandbox, ctx) = setup();
    let root = sandbox.state();
    put(&root, 1, Some("queued"), "20261001T000001Z");
    put(&root, 2, Some("queued"), "20261001T000002Z");
    let done = dispatch_with(
        &ctx,
        NOW,
        &|| true,
        &gone,
        &sessions(),
        &|_, claim: &Claim| {
            if claim.id.ends_with("#1") {
                Err("no window".to_string())
            } else {
                Ok(())
            }
        },
    );
    assert_eq!(done.failed, ["acme/widget#1"]);
    assert_eq!(done.started, ["acme/widget#2"]);
    assert_eq!(status_of(&root, 1), "failed");
    assert_eq!(error_of(&root, 1).as_deref(), Some("no window"));
    assert_eq!(status_of(&root, 2), "running");
}

#[test]
fn a_reader_is_started_with_no_lock_held() {
    let (sandbox, ctx) = setup();
    let root = sandbox.state();
    put(&root, 1, Some("queued"), "20261001T000001Z");
    let got_both = RefCell::new(false);
    dispatch_with(
        &ctx,
        NOW,
        &|| true,
        &gone,
        &sessions(),
        &|_, claim: &Claim| {
            // The queue's lock and the record's own, both free while a reader is being started.
            let queue = crate::infra::fs::try_lock(&root.join("others/review.lock")).unwrap();
            let record =
                crate::infra::fs::try_lock(&root.join(format!("others/{}.lock", claim.stem)))
                    .unwrap();
            *got_both.borrow_mut() = queue.is_some() && record.is_some();
            Ok(())
        },
    );
    assert!(*got_both.borrow());
}

#[test]
fn a_reader_that_died_frees_its_place_in_the_same_pass() {
    let (sandbox, ctx) = setup();
    let root = sandbox.state();
    for n in 1..=3 {
        put(&root, n, Some("running"), "20261001T000001Z");
        let path = root.join(format!("others/acme~widget~{n}.json"));
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["ai"]["pid"] = json!(100 + n);
        v["ai"]["sessionId"] = json!(format!("old-{n}"));
        std::fs::write(&path, v.to_string()).unwrap();
    }
    put(&root, 4, Some("queued"), "20261001T000009Z");
    let alive = |pid: u32, _: Option<&str>| {
        if pid == 101 {
            Liveness::Gone
        } else {
            Liveness::Alive
        }
    };
    let done = dispatch_with(&ctx, NOW, &|| true, &alive, &sessions(), &|_, _| Ok(()));
    assert_eq!(done.reaped, ["acme/widget#1"]);
    assert_eq!(done.started, ["acme/widget#4"]);
    assert_eq!(status_of(&root, 1), "failed");
}

#[test]
fn asking_is_refused_while_the_hub_is_down_or_no_reader_can_be_opened() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    put(&root, 1, None, "");
    let pr = others::pr_of("acme/widget#1").unwrap();
    assert_eq!(
        request_with(&root, &pr, NOW, ReadMode::Read, false, true),
        Err(AskError::HubStopped)
    );
    assert_eq!(
        request_with(&root, &pr, NOW, ReadMode::Read, true, false),
        Err(AskError::NotStartable)
    );
    // A refusal wrote nothing.
    assert!(
        !std::fs::read_to_string(root.join("others/acme~widget~1.json"))
            .unwrap()
            .contains("\"ai\"")
    );
    assert!(matches!(
        request_with(&root, &pr, NOW, ReadMode::Read, true, true),
        Ok(Requested::Queued(_))
    ));
    assert_eq!(status_of(&root, 1), "queued");
    assert!(matches!(
        request_with(&root, &pr, NOW, ReadMode::Read, true, true),
        Ok(Requested::Already(_))
    ));
    let missing = others::pr_of("acme/widget#9").unwrap();
    assert_eq!(
        request_with(&root, &missing, NOW, ReadMode::Read, true, true),
        Err(AskError::NoRecord)
    );
}

#[test]
fn asking_passes_the_mode_to_the_queue_and_a_takeover_takes_the_new_one() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    put(&root, 1, None, "");
    let pr = others::pr_of("acme/widget#1").unwrap();
    let mode_of = || {
        others::list(&root)
            .into_iter()
            .find(|r| r.id == "acme/widget#1")
            .and_then(|r| r.ai)
            .and_then(|ai| ai.mode)
    };
    assert!(matches!(
        request_with(&root, &pr, NOW, ReadMode::Split, true, true),
        Ok(Requested::Queued(_))
    ));
    assert_eq!(mode_of().as_deref(), Some("split"));
    // Asking for a read while it is queued is Already, and the mode stays.
    assert!(matches!(
        request_with(&root, &pr, NOW, ReadMode::Read, true, true),
        Ok(Requested::Already(_))
    ));
    assert_eq!(mode_of().as_deref(), Some("split"));
}

#[test]
fn the_reader_window_runs_review_run_in_its_own_directory() {
    let (_sandbox, ctx) = setup();
    let claim = Claim {
        id: "acme/widget#7".to_string(),
        url: "https://github.com/acme/widget/pull/7".to_string(),
        stem: "acme~widget~7".to_string(),
        dir: Path::new(&ctx.repo.main).join("acme+widget+7"),
        session_id: "sid-x".to_string(),
    };
    let seen = RefCell::new(None);
    launch_reader_with(&ctx, &claim, &|req| {
        *seen.borrow_mut() = Some((
            req.cwd.to_string(),
            req.title.to_string(),
            req.command.to_string(),
        ));
        Ok((
            Performed {
                description: String::new(),
                script: String::new(),
                ran: true,
                screen: false,
            },
            None,
        ))
    })
    .unwrap();
    let (cwd, title, command) = seen.borrow().clone().unwrap();
    assert_eq!(cwd, format!("{}/acme+widget+7", ctx.repo.main));
    assert!(Path::new(&cwd).is_dir());
    assert_eq!(title, "adj-read widget#7");
    assert!(
        command.contains("review run --pr 'acme/widget#7' --session sid-x"),
        "{command}"
    );

    let err = launch_reader_with(&ctx, &claim, &|_| Err("no tmux".to_string())).unwrap_err();
    assert_eq!(err, "no tmux");
}

#[test]
fn a_dead_reader_is_failed_while_the_hub_is_down_but_nothing_is_started() {
    let (sandbox, ctx) = setup();
    let root = sandbox.state();
    put(&root, 1, Some("running"), "20261001T000001Z");
    let path = root.join("others/acme~widget~1.json");
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["ai"]["pid"] = json!(101);
    v["ai"]["sessionId"] = json!("sid-old");
    std::fs::write(&path, v.to_string()).unwrap();
    put(&root, 2, Some("queued"), "20261001T000002Z");
    let done = dispatch_with(&ctx, NOW, &|| false, &gone, &sessions(), &|_, _| {
        panic!("a reader started with the hub down")
    });
    assert_eq!(done.reaped, ["acme/widget#1"]);
    assert!(done.started.is_empty());
    assert_eq!(status_of(&root, 1), "failed");
    assert_eq!(status_of(&root, 2), "queued");
}

#[test]
fn a_reader_that_cannot_finish_fails_its_read_only_under_its_own_session() {
    let (sandbox, _ctx) = setup();
    let root = sandbox.state();
    put(&root, 1, Some("running"), "20261001T000001Z");
    let path = root.join("others/acme~widget~1.json");
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["ai"]["sessionId"] = json!("sid-a");
    std::fs::write(&path, v.to_string()).unwrap();
    let pr = others::pr_of("acme/widget#1").unwrap();
    assert_eq!(
        fail(&root, &pr, Some("sid-b"), "no"),
        Err(FailError::NotCurrent)
    );
    assert_eq!(status_of(&root, 1), "running");
    fail(&root, &pr, Some("sid-a"), "could not read the diff").unwrap();
    assert_eq!(status_of(&root, 1), "failed");
    assert_eq!(
        error_of(&root, 1).as_deref(),
        Some("could not read the diff")
    );
    // Over already: nothing more to fail.
    assert_eq!(
        fail(&root, &pr, Some("sid-a"), "again"),
        Err(FailError::NotCurrent)
    );
}

#[test]
fn a_fresh_reader_starts_with_no_read_through_left_by_an_earlier_one() {
    let (_sandbox, ctx) = setup();
    let claim = Claim {
        id: "acme/widget#7".to_string(),
        url: "https://github.com/acme/widget/pull/7".to_string(),
        stem: "acme~widget~7".to_string(),
        dir: Path::new(&ctx.repo.main).join("acme+widget+7"),
        session_id: "sid-new".to_string(),
    };
    let scratch = claim.dir.clone();
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("read-through.json"), "{\"old\": true}").unwrap();
    std::fs::write(scratch.join("pr.json"), "{}").unwrap();
    launch_reader_with(&ctx, &claim, &|_| {
        assert!(!scratch.join("read-through.json").exists());
        Ok((
            Performed {
                description: String::new(),
                script: String::new(),
                ran: true,
                screen: false,
            },
            None,
        ))
    })
    .unwrap();
    assert!(!scratch.join("read-through.json").exists());
}

#[test]
fn a_queue_that_cannot_be_worked_is_a_warning_beside_the_result_and_not_an_error() {
    // A config that does not parse: the settings of the hub cannot be read.
    let sandbox = Sandbox::new("{ not json");
    let root = sandbox.state();
    put(&root, 1, Some("running"), "20261001T000001Z");
    let path = root.join("others/acme~widget~1.json");
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["ai"]["sessionId"] = json!("sid-a");
    std::fs::write(&path, v.to_string()).unwrap();
    // Something else is waiting, so the pass has work and meets the config.
    put(&root, 2, Some("queued"), "20261001T000002Z");
    let pr = others::pr_of("acme/widget#1").unwrap();

    let done = dispatch(&root);
    assert!(done.error.is_some());
    // The bare reason: the wording is the transports'.
    assert!(!done.error.clone().unwrap().contains("the queue of readers"));

    // The fail went through, and says what the queue could not do.
    let warning = fail(&root, &pr, Some("sid-a"), "no diff").unwrap();
    assert!(warning.is_some());
    assert_eq!(status_of(&root, 1), "failed");
}

// ── a reader that has run too long ───────────────────────────────────

/// A read running under `sid-old` since long before `NOW`, with the process it recorded.
fn hung(root: &Path, number: u64, pid: Option<u32>, started: Option<&str>) {
    put(root, number, Some("running"), "20260101T000000Z");
    let path = root.join(format!("others/acme~widget~{number}.json"));
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["ai"]["launchedAt"] = json!("20260101T000000Z");
    v["ai"]["sessionId"] = json!("sid-old");
    if let Some(pid) = pid {
        v["ai"]["pid"] = json!(pid);
    }
    if let Some(started) = started {
        v["ai"]["psStarted"] = json!(started);
    }
    std::fs::write(&path, v.to_string()).unwrap();
}

fn session_of(root: &Path, number: u64) -> Option<String> {
    let path = root.join(format!("others/acme~widget~{number}.json"));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["ai"]["sessionId"].as_str().map(str::to_string)
}

/// Ask again with the old reader's liveness and the way it is stopped handed in; the stop is
/// counted.
fn ask_again(
    root: &Path,
    alive: Liveness,
    stop: Result<(), String>,
) -> (Result<Requested, AskError>, usize) {
    let stopped = std::cell::Cell::new(0usize);
    let pr = others::pr_of("acme/widget#1").unwrap();
    let asked = request_taking_over(
        root,
        &pr,
        NOW,
        ReadMode::Read,
        true,
        true,
        &|_, _| alive,
        &|_| {
            stopped.set(stopped.get() + 1);
            stop.clone()
        },
    );
    (asked, stopped.get())
}

#[test]
fn a_hung_reader_whose_process_is_gone_is_taken_over() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    hung(&root, 1, Some(4242), Some("started"));
    let (asked, stops) = ask_again(&root, Liveness::Gone, Ok(()));
    assert!(matches!(asked, Ok(Requested::Queued(_))));
    assert_eq!(stops, 0, "nothing to stop");
    assert_eq!(status_of(&root, 1), "queued");
    assert_eq!(session_of(&root, 1), None);
}

#[test]
fn a_hung_reader_that_was_never_noted_is_taken_over_with_nothing_to_stop() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    hung(&root, 1, None, None);
    let (asked, stops) = ask_again(&root, Liveness::Alive, Err("must not be called".into()));
    assert!(matches!(asked, Ok(Requested::Queued(_))));
    assert_eq!(stops, 0);
}

#[test]
fn a_hung_reader_that_is_alive_is_stopped_and_then_taken_over() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    hung(&root, 1, Some(4242), Some("started"));
    let (asked, stops) = ask_again(&root, Liveness::Alive, Ok(()));
    assert!(matches!(asked, Ok(Requested::Queued(_))));
    assert_eq!(stops, 1);
    assert_eq!(status_of(&root, 1), "queued");
}

#[test]
fn a_hung_reader_that_cannot_be_stopped_or_told_is_left_running() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    hung(&root, 1, Some(4242), Some("started"));
    // Alive, and the stop fails: nothing is taken over, so no second reader starts beside it.
    let (asked, stops) = ask_again(&root, Liveness::Alive, Err("not in a tmux pane".into()));
    assert!(matches!(asked, Ok(Requested::Already(_))));
    assert_eq!(stops, 1);
    assert_eq!(status_of(&root, 1), "running");
    assert_eq!(session_of(&root, 1).as_deref(), Some("sid-old"));
    // Liveness cannot be told: the reader is not touched at all.
    let (asked, stops) = ask_again(&root, Liveness::CannotTell, Ok(()));
    assert!(matches!(asked, Ok(Requested::Already(_))));
    assert_eq!(stops, 0);
    assert_eq!(status_of(&root, 1), "running");
}

#[test]
fn a_reader_that_has_not_run_too_long_is_not_looked_at() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    // Launched at NOW: running, and young.
    put(&root, 1, Some("running"), "20261001T000000Z");
    let path = root.join("others/acme~widget~1.json");
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["ai"]["launchedAt"] = json!(crate::infra::clock::utc_stamp(NOW));
    v["ai"]["sessionId"] = json!("sid-old");
    v["ai"]["pid"] = json!(4242);
    std::fs::write(&path, v.to_string()).unwrap();
    let pr = others::pr_of("acme/widget#1").unwrap();
    let asked = request_taking_over(
        &root,
        &pr,
        NOW + 60,
        ReadMode::Read,
        true,
        true,
        &|_, _| panic!("a reader that is young was looked at"),
        &|_| panic!("a reader that is young was stopped"),
    );
    assert!(matches!(asked, Ok(Requested::Already(_))));
}

// ── closing a reader ─────────────────────────────────────────────────

/// A read in `status` whose reader `sid-old` recorded a process, on a PR in `pr_state`, with a
/// read-through and drafts kept for it.
fn open_reader(root: &Path, number: u64, status: &str, pr_state: &str) {
    put(root, number, Some(status), "20261001T000001Z");
    let path = root.join(format!("others/acme~widget~{number}.json"));
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["prState"] = json!(pr_state);
    v["ai"]["sessionId"] = json!("sid-old");
    v["ai"]["pid"] = json!(4242);
    v["ai"]["psStarted"] = json!("started");
    std::fs::write(&path, v.to_string()).unwrap();
    for dir in ["reads", "drafts"] {
        let dir = root.join("others").join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("acme~widget~{number}.json")),
            "{\"kept\": true}",
        )
        .unwrap();
    }
}

fn record_text(root: &Path, number: u64) -> String {
    std::fs::read_to_string(root.join(format!("others/acme~widget~{number}.json"))).unwrap()
}

fn closed_at(root: &Path, number: u64) -> Option<String> {
    let v: Value = serde_json::from_str(&record_text(root, number)).unwrap();
    v["ai"]["readerClosedAt"].as_str().map(str::to_string)
}

fn close(
    root: &Path,
    number: u64,
    alive: Liveness,
    stop: Result<(), String>,
) -> (Result<super::ClosedReader, super::CloseError>, usize) {
    let stopped = std::cell::Cell::new(0usize);
    let pr = others::pr_of(&format!("acme/widget#{number}")).unwrap();
    let closed = close_reader_with(
        root,
        &pr,
        NOW,
        "the reader was closed",
        &|_, _| alive,
        &|_| {
            stopped.set(stopped.get() + 1);
            stop.clone()
        },
    );
    (closed, stopped.get())
}

fn kept(root: &Path, number: u64) -> bool {
    ["reads", "drafts"].iter().all(|dir| {
        root.join(format!("others/{dir}/acme~widget~{number}.json"))
            .exists()
    })
}

#[test]
fn a_reader_whose_process_is_gone_is_marked_closed_with_nothing_to_stop() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "ready", "open");
    let (closed, stops) = close(&root, 1, Liveness::Gone, Err("must not be called".into()));
    let closed = closed.unwrap();
    assert!(!closed.was_running);
    assert_eq!(stops, 0);
    assert!(closed.record.ai.unwrap().reader_closed_at.is_some());
    assert!(closed_at(&root, 1).is_some());
}

#[test]
fn a_reader_that_is_alive_is_stopped_before_the_read_is_marked() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "ready", "open");
    let pr = others::pr_of("acme/widget#1").unwrap();
    let marked_when_stopped = std::cell::Cell::new(true);
    let closed = close_reader_with(
        &root,
        &pr,
        NOW,
        "the reader was closed",
        &|_, _| Liveness::Alive,
        &|reader| {
            assert_eq!(reader.session_id, "sid-old");
            marked_when_stopped.set(closed_at(&root, 1).is_some());
            Ok(())
        },
    );
    assert!(closed.is_ok());
    assert!(!marked_when_stopped.get(), "marked before it was stopped");
    assert!(closed_at(&root, 1).is_some());
}

#[test]
fn a_ready_read_stays_ready_and_its_read_through_and_drafts_stay() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "ready", "open");
    close(&root, 1, Liveness::Gone, Ok(())).0.unwrap();
    assert_eq!(status_of(&root, 1), "ready");
    assert_eq!(error_of(&root, 1), None);
    assert!(kept(&root, 1));
}

#[test]
fn a_running_read_fails_under_its_own_session_and_the_next_in_line_starts() {
    let (sandbox, ctx) = setup();
    let root = sandbox.state();
    open_reader(&root, 1, "running", "open");
    put(&root, 2, Some("queued"), "20261001T000002Z");
    let closed = close(&root, 1, Liveness::Alive, Ok(())).0.unwrap();
    assert!(closed.was_running);
    assert_eq!(status_of(&root, 1), "failed");
    assert_eq!(error_of(&root, 1).as_deref(), Some("the reader was closed"));
    assert!(closed_at(&root, 1).is_some());
    assert!(kept(&root, 1));
    // The place it held is free: the queue moves on.
    let launched = RefCell::new(Vec::new());
    let done = dispatch_with(
        &ctx,
        NOW,
        &|| true,
        &gone,
        &sessions(),
        &|_, claim: &Claim| {
            launched.borrow_mut().push(claim.id.clone());
            Ok(())
        },
    );
    assert_eq!(done.started, vec!["acme/widget#2".to_string()]);
}

#[test]
fn a_reader_that_cannot_be_stopped_or_told_leaves_the_record_as_it_was() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "running", "open");
    let before = record_text(&root, 1);
    let (closed, stops) = close(&root, 1, Liveness::Alive, Err("not in a tmux pane".into()));
    assert_eq!(
        closed,
        Err(super::CloseError::Failed("not in a tmux pane".to_string()))
    );
    assert_eq!(stops, 1);
    assert_eq!(record_text(&root, 1), before);
    let (closed, stops) = close(&root, 1, Liveness::CannotTell, Ok(()));
    assert!(matches!(closed, Err(super::CloseError::Failed(_))));
    assert_eq!(stops, 0);
    assert_eq!(record_text(&root, 1), before);
}

#[test]
fn there_is_no_reader_to_close_without_a_record_a_process_or_an_open_reader() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let (closed, _) = close(&root, 1, Liveness::Gone, Ok(()));
    assert_eq!(closed, Err(super::CloseError::NoRecord));
    // Still being started: no process recorded yet.
    hung(&root, 1, None, None);
    let (closed, _) = close(&root, 1, Liveness::Gone, Ok(()));
    assert_eq!(closed, Err(super::CloseError::NoReader));
    // Failed, queued and closed reads have none either.
    for status in ["failed", "queued"] {
        open_reader(&root, 1, status, "open");
        assert_eq!(
            close(&root, 1, Liveness::Gone, Ok(())).0,
            Err(super::CloseError::NoReader),
            "{status}"
        );
    }
    open_reader(&root, 1, "ready", "open");
    close(&root, 1, Liveness::Gone, Ok(())).0.unwrap();
    assert_eq!(
        close(&root, 1, Liveness::Gone, Ok(())).0,
        Err(super::CloseError::NoReader)
    );
}

fn sweep(
    root: &Path,
    alive: Liveness,
    stop: impl Fn(&others::HungReader) -> Result<(), String>,
) -> Vec<(String, String)> {
    close_finished_readers_with(root, NOW, &|_, _| alive, &stop)
}

#[test]
fn the_sweep_closes_the_readers_of_merged_and_closed_prs_only() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "ready", "merged");
    open_reader(&root, 2, "running", "closed");
    open_reader(&root, 3, "ready", "open");
    // Approved, or the request withdrawn: done, but the PR is open, so the reader stays.
    for (number, review) in [
        (4, json!(null)),
        (
            5,
            json!({"state": "approved", "submittedAt": "20261002T000000Z", "commit": "h1"}),
        ),
    ] {
        open_reader(&root, number, "ready", "open");
        let path = root.join(format!("others/acme~widget~{number}.json"));
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["requested"] = json!(false);
        v["myReview"] = review;
        std::fs::write(&path, v.to_string()).unwrap();
        let pr = others::pr_of(&format!("acme/widget#{number}")).unwrap();
        let state = others::record(&root, &pr).unwrap().unwrap().state;
        assert_eq!(state, others::State::Done, "{number}");
    }
    let failures = sweep(&root, Liveness::Gone, |_| panic!("nothing to stop"));
    assert!(failures.is_empty());
    assert!(closed_at(&root, 1).is_some());
    assert!(closed_at(&root, 2).is_some());
    assert_eq!(status_of(&root, 2), "failed");
    assert_eq!(
        error_of(&root, 2).as_deref(),
        Some("the PR is no longer open")
    );
    assert_eq!(status_of(&root, 1), "ready");
    assert!(closed_at(&root, 3).is_none());
    assert!(closed_at(&root, 4).is_none());
    assert!(closed_at(&root, 5).is_none());
}

#[test]
fn the_sweep_skips_a_reader_already_closed_and_goes_on_past_one_that_fails() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "ready", "merged");
    close(&root, 1, Liveness::Gone, Ok(())).0.unwrap();
    let before = record_text(&root, 1);
    open_reader(&root, 2, "ready", "merged");
    open_reader(&root, 3, "ready", "closed");
    let failures = sweep(&root, Liveness::Alive, |_| Ok(()));
    assert!(failures.is_empty());
    assert_eq!(record_text(&root, 1), before, "closed again");
    assert!(closed_at(&root, 2).is_some());
    assert!(closed_at(&root, 3).is_some());

    // One that cannot be stopped is reported, left open, and does not stop the rest.
    open_reader(&root, 2, "ready", "merged");
    open_reader(&root, 3, "ready", "closed");
    let failures = sweep(&root, Liveness::Alive, |reader| {
        let _ = reader;
        Err("not in a tmux pane".to_string())
    });
    assert_eq!(failures.len(), 2);
    assert_eq!(
        failures[0],
        (
            "acme/widget#2".to_string(),
            "not in a tmux pane".to_string()
        )
    );
    assert!(closed_at(&root, 2).is_none());
    let stopped = std::cell::RefCell::new(Vec::new());
    let failures = sweep(&root, Liveness::Alive, |reader| {
        stopped.borrow_mut().push(reader.session_id.clone());
        if stopped.borrow().len() == 1 {
            Err("stuck".to_string())
        } else {
            Ok(())
        }
    });
    assert_eq!(failures.len(), 1);
    assert!(closed_at(&root, 2).is_none());
    assert!(closed_at(&root, 3).is_some());
}

#[test]
fn only_a_running_read_of_a_finished_pr_frees_a_place_for_the_next() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    assert!(!frees_a_place(&root));
    open_reader(&root, 1, "ready", "merged");
    open_reader(&root, 2, "running", "open");
    assert!(!frees_a_place(&root), "handed in, or the PR still open");
    open_reader(&root, 3, "running", "closed");
    assert!(frees_a_place(&root));
    close(&root, 3, Liveness::Gone, Ok(())).0.unwrap();
    assert!(!frees_a_place(&root), "already closed");
}

#[test]
fn a_reader_whose_pr_was_approved_is_closed_once_a_sync_has_seen_it_merged() {
    // What the sync leaves after it reads an approved (done) PR that was merged since.
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "ready", "open");
    let path = root.join("others/acme~widget~1.json");
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["requested"] = json!(false);
    v["myReview"] = json!({"state": "approved", "submittedAt": "20261002T000000Z", "commit": "h1"});
    std::fs::write(&path, v.to_string()).unwrap();
    assert!(sweep(&root, Liveness::Gone, |_| Ok(())).is_empty());
    assert!(closed_at(&root, 1).is_none(), "open PR: kept");
    v["prState"] = json!("merged");
    std::fs::write(&path, v.to_string()).unwrap();
    assert!(sweep(&root, Liveness::Gone, |_| Ok(())).is_empty());
    assert!(closed_at(&root, 1).is_some());
}

#[test]
fn the_sweep_has_nothing_to_do_without_a_finished_pr_with_an_open_reader() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    assert!(!has_finished_reader(&root));
    open_reader(&root, 1, "ready", "open");
    assert!(!has_finished_reader(&root));
    open_reader(&root, 2, "ready", "merged");
    assert!(has_finished_reader(&root));
    close(&root, 2, Liveness::Gone, Ok(())).0.unwrap();
    assert!(!has_finished_reader(&root));
}

#[test]
fn a_stop_that_fails_because_another_closer_got_there_first_still_closes() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "ready", "open");
    let pr = others::pr_of("acme/widget#1").unwrap();
    let asked = std::cell::Cell::new(0);
    // Alive when looked at, gone when asked again after the stop failed.
    let alive = |_: u32, _: Option<&str>| {
        asked.set(asked.get() + 1);
        if asked.get() == 1 {
            Liveness::Alive
        } else {
            Liveness::Gone
        }
    };
    let closed = close_reader_with(
        &root,
        &pr,
        NOW,
        "x",
        &alive,
        &|_| Err("no such pane".into()),
    );
    assert!(closed.is_ok());
    assert!(closed_at(&root, 1).is_some());
    // Still alive after a failed stop: the failure stands and nothing is written.
    open_reader(&root, 2, "ready", "open");
    let pr2 = others::pr_of("acme/widget#2").unwrap();
    let still = close_reader_with(&root, &pr2, NOW, "x", &|_, _| Liveness::Alive, &|_| {
        Err("no pane".into())
    });
    assert_eq!(still, Err(super::CloseError::Failed("no pane".to_string())));
    assert!(closed_at(&root, 2).is_none());
}

// ── a task's PR ──────────────────────────────────────────────────────

/// The context of the hub `acme-widget`, whose tasks are the ones a read is asked for.
fn task_hub() -> (Sandbox, Context) {
    let sandbox = Sandbox::empty();
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = registry::context_at(repo, sandbox.state()).unwrap();
    (sandbox, ctx)
}

/// Task `id` of the hub `acme-widget`, with `pr` when given.
fn put_task(root: &Path, id: &str, pr: Option<&str>) {
    put_task_status(root, id, pr, "backlog");
}

/// `put_task`, in `status`.
fn put_task_status(root: &Path, id: &str, pr: Option<&str>, status: &str) {
    let dir = root.join("tasks/acme-widget");
    std::fs::create_dir_all(&dir).unwrap();
    let mut task = json!({
        "id": id, "kind": "investigate", "title": "t", "doneWhen": "report-only",
        "status": status, "autoStart": true, "createdAt": "20261001T000000Z", "updatedAt": "20261001T000000Z"
    });
    if let Some(pr) = pr {
        task["pr"] = json!(pr);
    }
    std::fs::write(dir.join(format!("{id}.json")), task.to_string()).unwrap();
}

/// What the details query answers for PR 7 at `head`, which nobody asked you to review.
fn pr_seven(head: &str) -> Value {
    let up = |word: &str| word.to_ascii_uppercase();
    json!({
        "number": 7, "url": "https://github.com/acme/widget/pull/7", "title": "Add a thing",
        "state": up("open"), "isDraft": false, "author": {"login": "alice"},
        "baseRefName": "main", "headRefName": "feature", "headRefOid": head,
        "additions": 1, "deletions": 1, "changedFiles": 1,
        "files": {
            "pageInfo": {"hasNextPage": false, "endCursor": null},
            "nodes": [{"path": "a.rs", "additions": 1, "deletions": 1, "changeType": up("modified")}]
        },
        "reviewRequests": {"nodes": []}, "latestReviews": {"nodes": []},
        "myReviews": {"nodes": []}, "requests": {"nodes": []},
        "commits": {"nodes": [{"commit": {"oid": head, "committedDate": "2026-09-30T00:00:00Z"}}]},
        "lastCommit": {"nodes": [{"commit": {"statusCheckRollup": null}}]}
    })
}

/// A `gh` that knows who you are and PR 7, and counts what it was asked.
fn github(
    asked: &AtomicUsize,
) -> impl Fn(&[&str], std::time::Instant) -> Result<crate::infra::gh::GhRun, String> + Sync + '_ {
    move |args: &[&str], _| {
        asked.fetch_add(1, Ordering::SeqCst);
        let said = |stdout: String| {
            Ok(crate::infra::gh::GhRun {
                ok: true,
                stdout,
                stderr: String::new(),
            })
        };
        match args {
            ["api", "user", ..] => said("me".to_string()),
            ["api", "graphql", ..] => {
                said(json!({"data": {"p0": {"pullRequest": pr_seven("h1")}}}).to_string())
            }
            other => panic!("unexpected gh call {other:?}"),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn ask_for_task(
    ctx: &Context,
    id: &str,
    now: i64,
    hub_up: bool,
    startable: bool,
    asked: &AtomicUsize,
) -> Result<Requested, TaskAskError> {
    request_for_task_with(
        ctx,
        id,
        ReadMode::Read,
        now,
        hub_up,
        startable,
        &gone,
        &|_| Ok(()),
        &github(asked),
    )
}

#[test]
fn asking_for_a_task_s_read_is_refused_before_github_is_asked_or_anything_written() {
    let (sandbox, ctx) = task_hub();
    let root = sandbox.state();
    put_task(&root, "none", None);
    put_task(&root, "bare", Some("#7"));
    put_task(
        &root,
        "elsewhere",
        Some("https://ghe.example.com/acme/widget/pull/7"),
    );
    put_task(&root, "ok", Some("https://github.com/acme/widget/pull/7"));
    let asked = AtomicUsize::new(0);
    let ask = |id: &str, hub_up, startable| ask_for_task(&ctx, id, NOW, hub_up, startable, &asked);

    assert!(matches!(
        ask("missing", true, true),
        Err(TaskAskError::NoTask)
    ));
    assert_eq!(ask("none", true, true), Err(TaskAskError::NoPr));
    // A bare number has no remote to be read against here.
    assert_eq!(
        ask("bare", true, true),
        Err(TaskAskError::NotReadable("#7".to_string()))
    );
    assert!(matches!(
        ask("elsewhere", true, true),
        Err(TaskAskError::NotReadable(_))
    ));
    assert_eq!(
        ask("ok", false, true),
        Err(TaskAskError::Refused(AskError::HubStopped))
    );
    assert_eq!(
        ask("ok", true, false),
        Err(TaskAskError::Refused(AskError::NotStartable))
    );
    assert_eq!(asked.load(Ordering::SeqCst), 0);
    assert!(!root.join("others").exists());
}

#[test]
fn asking_for_a_task_s_read_queues_it_and_asking_again_while_running_writes_nothing() {
    let (sandbox, ctx) = task_hub();
    let root = sandbox.state();
    put_task(&root, "ok", Some("https://github.com/acme/widget/pull/7"));
    let asked = AtomicUsize::new(0);

    let Ok(Requested::Queued(record)) = ask_for_task(&ctx, "ok", NOW, true, true, &asked) else {
        panic!("not queued");
    };
    assert_eq!(
        record
            .task
            .as_ref()
            .map(|t| (t.board.as_str(), t.id.as_str())),
        Some(("acme-widget", "ok"))
    );
    assert!(!record.requested && !record.on_review_list());
    assert_eq!(record.ai.as_ref().unwrap().status, "queued");
    assert_eq!(status_of(&root, 7), "queued");

    // Once it runs, asking again keeps the place and the time it was asked.
    let claims = others::claim_reads(&root, NOW, &sessions()).unwrap();
    assert_eq!(claims.len(), 1);
    let before = record_text(&root, 7);
    let Ok(Requested::Already(again)) = ask_for_task(&ctx, "ok", NOW + 60, true, true, &asked)
    else {
        panic!("not already");
    };
    assert_eq!(again.ai.as_ref().unwrap().status, "running");
    let after: Value = serde_json::from_str(&record_text(&root, 7)).unwrap();
    let before: Value = serde_json::from_str(&before).unwrap();
    assert_eq!(after["ai"], before["ai"]);
}

#[test]
fn the_sweep_closes_the_reader_of_a_merged_task_pr_and_keeps_its_read() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "ready", "merged");
    open_reader(&root, 2, "ready", "open");
    for number in [1, 2] {
        let path = root.join(format!("others/acme~widget~{number}.json"));
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["requested"] = json!(false);
        v["task"] = json!({"board": "acme-widget", "id": "t1"});
        std::fs::write(&path, v.to_string()).unwrap();
    }
    let failures = sweep(&root, Liveness::Gone, |_| panic!("nothing to stop"));
    assert!(failures.is_empty());
    assert!(closed_at(&root, 1).is_some());
    assert_eq!(status_of(&root, 1), "ready");
    assert!(kept(&root, 1));
    // An open task PR with no request is not done, and its reader stays.
    assert!(closed_at(&root, 2).is_none());
}

#[test]
fn a_task_record_that_cannot_be_read_is_not_a_missing_task() {
    let (sandbox, ctx) = task_hub();
    let root = sandbox.state();
    let dir = root.join("tasks/acme-widget");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("broken.json"), "{ not json").unwrap();
    let asked = AtomicUsize::new(0);
    assert!(matches!(
        ask_for_task(&ctx, "broken", NOW, true, true, &asked),
        Err(TaskAskError::TaskUnreadable(_))
    ));
    assert_eq!(
        ask_for_task(&ctx, "missing", NOW, true, true, &asked),
        Err(TaskAskError::NoTask)
    );
    assert_eq!(asked.load(Ordering::SeqCst), 0);
}

#[test]
fn a_task_given_another_pr_while_github_is_read_is_not_queued() {
    let (sandbox, ctx) = task_hub();
    let root = sandbox.state();
    put_task(&root, "ok", Some("https://github.com/acme/widget/pull/7"));
    let asked = AtomicUsize::new(0);
    let real = github(&asked);
    // The task's PR is rewritten while GitHub answers.
    let rewriting = |args: &[&str], deadline| {
        put_task(&root, "ok", Some("https://github.com/acme/widget/pull/8"));
        real(args, deadline)
    };
    let result = request_for_task_with(
        &ctx,
        "ok",
        ReadMode::Read,
        NOW,
        true,
        true,
        &gone,
        &|_| Ok(()),
        &rewriting,
    );
    assert_eq!(result, Err(TaskAskError::PrChanged));
    // Read, but not queued.
    let pr = others::pr_of("acme/widget#7").unwrap();
    let record = others::record(&root, &pr).unwrap().unwrap();
    assert!(record.ai.is_none());
    // A task that is gone, or has lost its PR, meanwhile is said so.
    put_task(&root, "ok", Some("https://github.com/acme/widget/pull/7"));
    let dropping = |args: &[&str], deadline| {
        put_task(&root, "ok", None);
        real(args, deadline)
    };
    let result = request_for_task_with(
        &ctx,
        "ok",
        ReadMode::Read,
        NOW,
        true,
        true,
        &gone,
        &|_| Ok(()),
        &dropping,
    );
    assert_eq!(result, Err(TaskAskError::NoPr));
}

/// PR `number`, read for task `task_id` of the hub `acme-widget` and requested of nobody.
fn link(root: &Path, number: u64, task_id: &str) {
    let path = root.join(format!("others/acme~widget~{number}.json"));
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["requested"] = json!(false);
    v["task"] = json!({"board": "acme-widget", "id": task_id});
    std::fs::write(&path, v.to_string()).unwrap();
}

fn sweep_tasks(
    root: &Path,
    alive: Liveness,
    stop: impl Fn(&others::HungReader) -> Result<(), String>,
) -> Vec<(String, String)> {
    close_task_readers_with(root, NOW, &|_, _| alive, &stop)
}

#[test]
fn a_reader_of_a_done_or_cancelled_task_is_closed_and_its_read_kept() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    put_task_status(&root, "t1", None, "done");
    put_task_status(&root, "t2", None, "cancelled");
    open_reader(&root, 1, "ready", "open");
    open_reader(&root, 2, "running", "open");
    link(&root, 1, "t1");
    link(&root, 2, "t2");
    let failures = sweep_tasks(&root, Liveness::Gone, |_| panic!("nothing to stop"));
    assert!(failures.is_empty());
    assert!(closed_at(&root, 1).is_some() && closed_at(&root, 2).is_some());
    assert_eq!(status_of(&root, 1), "ready");
    assert_eq!(status_of(&root, 2), "failed");
    assert_eq!(error_of(&root, 2).as_deref(), Some("the task is finished"));
    assert!(kept(&root, 1) && kept(&root, 2));
}

#[test]
fn a_reader_of_a_task_whose_record_is_gone_is_closed() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "ready", "open");
    link(&root, 1, "deleted");
    let failures = sweep_tasks(&root, Liveness::Gone, |_| panic!("nothing to stop"));
    assert!(failures.is_empty());
    assert!(closed_at(&root, 1).is_some());
    assert!(kept(&root, 1));
}

#[test]
fn a_reader_of_a_task_still_in_play_is_kept() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    for (number, status) in [(1, "backlog"), (2, "queued"), (3, "dispatched"), (4, "pr")] {
        let id = format!("t{number}");
        put_task_status(&root, &id, None, status);
        open_reader(&root, number, "running", "open");
        link(&root, number, &id);
    }
    let failures = sweep_tasks(&root, Liveness::Alive, |_| {
        panic!("a reader in play was stopped")
    });
    assert!(failures.is_empty());
    for number in 1..=4 {
        assert!(closed_at(&root, number).is_none());
        assert_eq!(status_of(&root, number), "running");
    }
}

#[test]
fn a_task_record_that_cannot_be_read_keeps_its_reader() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let dir = root.join("tasks/acme-widget");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t1.json"), "{").unwrap();
    open_reader(&root, 1, "ready", "open");
    link(&root, 1, "t1");
    let failures = sweep_tasks(&root, Liveness::Gone, |_| panic!("nothing to stop"));
    assert!(failures.is_empty());
    assert!(closed_at(&root, 1).is_none());
}

#[test]
fn a_reader_not_read_for_a_task_or_also_requested_follows_the_request_rule() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    // 1: no task link at all. 2: read for a finished task, and also requested of you.
    put_task_status(&root, "t2", None, "done");
    open_reader(&root, 1, "ready", "open");
    open_reader(&root, 2, "ready", "open");
    link(&root, 2, "t2");
    let path = root.join("others/acme~widget~2.json");
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["requested"] = json!(true);
    std::fs::write(&path, v.to_string()).unwrap();
    assert!(!has_task_reader(&root));
    let failures = sweep_tasks(&root, Liveness::Gone, |_| panic!("nothing to stop"));
    assert!(failures.is_empty());
    assert!(closed_at(&root, 1).is_none() && closed_at(&root, 2).is_none());
}

#[test]
fn closing_a_finished_task_s_reader_again_writes_nothing() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    put_task_status(&root, "t1", None, "done");
    open_reader(&root, 1, "ready", "open");
    link(&root, 1, "t1");
    sweep_tasks(&root, Liveness::Gone, |_| panic!("nothing to stop"));
    let first = record_text(&root, 1);
    assert!(closed_at(&root, 1).is_some());
    let failures = sweep_tasks(&root, Liveness::Gone, |_| panic!("nothing to stop"));
    assert!(failures.is_empty());
    assert_eq!(record_text(&root, 1), first);
}

#[test]
fn a_task_reader_that_cannot_be_told_alive_is_left_and_reported() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    put_task_status(&root, "t1", None, "done");
    for number in [1, 2] {
        open_reader(&root, number, "ready", "open");
        link(&root, number, "t1");
    }
    let before = record_text(&root, 1);
    let alive = |pid: u32, _: Option<&str>| {
        if pid == 4242 {
            Liveness::CannotTell
        } else {
            Liveness::Gone
        }
    };
    // Both readers carry pid 4242: make the second one's different, so only the first fails.
    let path = root.join("others/acme~widget~2.json");
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["ai"]["pid"] = json!(4243);
    std::fs::write(&path, v.to_string()).unwrap();
    let failures = close_task_readers_with(&root, NOW, &alive, &|_| panic!("nothing to stop"));
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, "acme/widget#1");
    assert!(failures[0].1.contains("cannot tell"));
    assert_eq!(record_text(&root, 1), before);
    assert!(closed_at(&root, 2).is_some());
}

#[test]
fn has_task_reader_needs_a_linked_open_reader() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    assert!(!has_task_reader(&root));
    // An open reader that is read for no task.
    open_reader(&root, 1, "ready", "open");
    assert!(!has_task_reader(&root));
    // A task's PR with no read.
    put(&root, 2, None, "20261001T000000Z");
    link(&root, 2, "t1");
    assert!(!has_task_reader(&root));
    // A linked reader, then closed.
    open_reader(&root, 3, "ready", "open");
    link(&root, 3, "t1");
    assert!(has_task_reader(&root));
    let (closed, _) = close(&root, 3, Liveness::Gone, Ok(()));
    closed.unwrap();
    assert!(!has_task_reader(&root));
}

#[test]
fn the_poll_closes_the_readers_of_ended_prs_it_names() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "running", "open");
    open_reader(&root, 2, "ready", "open");
    open_reader(&root, 3, "ready", "open");
    let named: Vec<_> = [1, 3, 4]
        .iter()
        .map(|n| others::pr_of(&format!("acme/widget#{n}")).unwrap())
        .collect();
    // 3 is closed already: nothing to say. 4 has no record.
    close(&root, 3, Liveness::Gone, Ok(())).0.unwrap();
    let failures = close_ended_pr_readers_with(&root, &named, NOW, &|_, _| Liveness::Gone, &|_| {
        panic!("nothing to stop")
    });
    assert!(failures.is_empty());
    assert!(closed_at(&root, 1).is_some());
    assert_eq!(status_of(&root, 1), "failed");
    assert_eq!(
        error_of(&root, 1).as_deref(),
        Some("the PR is no longer open")
    );
    assert!(kept(&root, 1));
    // Not named: untouched.
    assert!(closed_at(&root, 2).is_none());
}

#[test]
fn a_finished_task_s_live_reader_is_stopped_once_and_closed() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    put_task_status(&root, "t1", None, "done");
    open_reader(&root, 1, "ready", "open");
    link(&root, 1, "t1");
    // Alive until it is stopped, as a pane is.
    let stopped = std::cell::Cell::new(0usize);
    let alive = |_: u32, _: Option<&str>| {
        if stopped.get() == 0 {
            Liveness::Alive
        } else {
            Liveness::Gone
        }
    };
    let failures = close_task_readers_with(&root, NOW, &alive, &|_| {
        stopped.set(stopped.get() + 1);
        Ok(())
    });
    assert!(failures.is_empty());
    assert_eq!(stopped.get(), 1);
    assert!(closed_at(&root, 1).is_some());
}

#[test]
fn a_finished_task_s_reader_that_cannot_be_stopped_is_left_and_reported() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    put_task_status(&root, "t1", None, "done");
    open_reader(&root, 1, "ready", "open");
    link(&root, 1, "t1");
    let before = record_text(&root, 1);
    let failures = sweep_tasks(&root, Liveness::Alive, |_| Err("tmux refused".to_string()));
    assert_eq!(
        failures,
        [("acme/widget#1".to_string(), "tmux refused".to_string())]
    );
    assert_eq!(record_text(&root, 1), before);
}

#[test]
fn the_poll_stops_a_live_reader_of_an_ended_pr_once_and_closes_it() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    open_reader(&root, 1, "ready", "open");
    let named = [others::pr_of("acme/widget#1").unwrap()];
    let stopped = std::cell::Cell::new(0usize);
    let alive = |_: u32, _: Option<&str>| {
        if stopped.get() == 0 {
            Liveness::Alive
        } else {
            Liveness::Gone
        }
    };
    let failures = close_ended_pr_readers_with(&root, &named, NOW, &alive, &|_| {
        stopped.set(stopped.get() + 1);
        Ok(())
    });
    assert!(failures.is_empty());
    assert_eq!(stopped.get(), 1);
    assert!(closed_at(&root, 1).is_some());
}
