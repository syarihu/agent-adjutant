use super::dispatch::dispatch_with;
use super::launch_reader::launch_reader_with;
use super::request::request_with;
use super::*;
use crate::infra::terminal::Performed;
use crate::others::{self, Claim, ReadMode, Requested};
use crate::registry::{self, Context, Liveness};
use crate::testing::Sandbox;

use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::Path;

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
        dir: Path::new(&ctx.repo.main).join("acme~widget~7"),
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
    assert_eq!(cwd, format!("{}/acme~widget~7", ctx.repo.main));
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
        dir: Path::new(&ctx.repo.main).join("acme~widget~7"),
        session_id: "sid-new".to_string(),
    };
    let scratch = Path::new(&ctx.repo.main).join(&claim.stem);
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
