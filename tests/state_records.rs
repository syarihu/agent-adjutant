//! What older hubs and workers left on disk, read by this binary.
//!
//! Hubs and workers keep running across `adj server restart`, so the state directory is
//! shared by binaries of different ages. These tests write records in the shape a build wrote
//! them (`src/fixtures/state/`, see its README), read each one through the commands that
//! read it, and, where a command writes the record back, check that a key this binary does
//! not know is still there afterwards.

mod common;

use common::*;

type Json = serde_json::Value;

const WORKER: &str = include_str!("../src/fixtures/state/worker.json");
const HUB_RECORD: &str = include_str!("../src/fixtures/state/hub.json");
const REPORT: &str = include_str!("../src/fixtures/state/inbox-report.md");
const HELD: &str = include_str!("../src/fixtures/state/inbox-held.md");
const HUB_SESSION: &str = include_str!("../src/fixtures/state/session-hub.json");
const WORKER_SESSION: &str = include_str!("../src/fixtures/state/session-worker.json");
const GATE: &str = include_str!("../src/fixtures/state/gate.json");
const TASK: &str = include_str!("../src/fixtures/state/task.json");

const REPORT_NAME: &str = "20260908T112233Z-report.md";
const HELD_NAME: &str = "20260908T112300Z-done.md";
const GATE_ID: &str = "20260908T041500Z-plan";
const TASK_ID: &str = "20260908T041500Z-retry-login-on-timeout";

fn parse(text: &str) -> Json {
    serde_json::from_str(text).expect(text)
}

fn read_json(path: &Path) -> Json {
    parse(&std::fs::read_to_string(path).unwrap())
}

/// Put `text` at `path`, making the directory it lives in.
fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn worker_record(fixture: &Fixture) -> PathBuf {
    fixture.repo.join(".claude").join("adjutant-worker.json")
}

fn worker_session(fixture: &Fixture) -> PathBuf {
    fixture.repo.join(".claude").join("adjutant-session.json")
}

fn hub_record(fixture: &Fixture) -> PathBuf {
    fixture.state.join("hubs").join(format!("{SLUG}.json"))
}

fn hub_session(fixture: &Fixture) -> PathBuf {
    fixture.state.join("sessions").join(format!("{SLUG}.json"))
}

fn inbox(fixture: &Fixture) -> PathBuf {
    fixture.state.join("inbox").join(SLUG)
}

fn gate_file(fixture: &Fixture, dir: &str) -> PathBuf {
    let base = fixture.state.join("gates").join(SLUG);
    let base = if dir.is_empty() { base } else { base.join(dir) };
    base.join(format!("{GATE_ID}.json"))
}

fn task_file(fixture: &Fixture) -> PathBuf {
    fixture
        .state
        .join("tasks")
        .join(SLUG)
        .join(format!("{TASK_ID}.json"))
}

fn tool_result(response: &Json) -> Json {
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    parse(text)
}

/// Every key `before` has is in `after` with the same value, except those a command was
/// expected to change. A key the binary does not know is the point, but so is every other:
/// a write that dropped `psStarted` would be as much of a loss.
fn keeps_every_key(before: &Json, after: &Json, changed: &[&str]) {
    for (key, value) in before.as_object().unwrap() {
        if changed.contains(&key.as_str()) {
            continue;
        }
        assert_eq!(after.get(key), Some(value), "{key} in {after}");
    }
}

#[test]
fn a_worker_record_is_read_and_a_phase_change_keeps_keys_it_does_not_know() {
    let fixture = Fixture::new(QUIET);
    let record = worker_record(&fixture);
    put(&record, WORKER);
    let before = parse(WORKER);
    let worktree = fixture.repo.to_str().unwrap();

    assert_eq!(
        fixture.ok(&["phase", "--worktree", worktree]).trim(),
        "implement"
    );
    fixture.ok(&["phase", "--set", "review", "--worktree", worktree]);

    let after = read_json(&record);
    assert_eq!(after["phase"], "review");
    assert_eq!(after["x-unknown"], before["x-unknown"]);
    assert_eq!(
        after["phases"].as_array().unwrap().len(),
        before["phases"].as_array().unwrap().len() + 1,
        "{after}"
    );
    keeps_every_key(&before, &after, &["phase", "phaseAt", "phases"]);
}

#[test]
fn a_worker_record_still_addresses_the_hub_it_names() {
    let fixture = Fixture::new(QUIET);
    put(&worker_record(&fixture), WORKER);

    let info = fixture.json(&["hub-name", "--json"]);
    assert_eq!(info["hub"], FEATURE);
    assert_eq!(info["hubName"], FEATURE_HUB);
    assert_eq!(info["slug"], FEATURE_SLUG);
}

#[test]
fn a_hub_record_is_read_as_it_is_written_and_left_alone() {
    let fixture = Fixture::new(QUIET);
    let record = hub_record(&fixture);
    put(&record, HUB_RECORD);

    let replies = mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_hub_status", "arguments": {}}),
        )],
    );
    let status = tool_result(&replies[0]);
    assert_eq!(status["hubName"], HUB);
    assert_eq!(status["pid"], 4242);
    assert_eq!(status["cwd"], "/path/to/widget");
    assert_eq!(status["startedAt"], "20260908T100000Z");
    // No process of that name started then runs as pid 4242, so the hub is gone.
    assert_eq!(status["present"], false);
    assert_eq!(std::fs::read_to_string(&record).unwrap(), HUB_RECORD);
}

#[test]
fn an_inbox_entry_is_listed_read_and_filed_byte_for_byte() {
    let fixture = Fixture::new(QUIET);
    put(&inbox(&fixture).join(REPORT_NAME), REPORT);

    let list = fixture.json(&["pending", "--json"]);
    assert_eq!(list["count"], 1, "{list}");
    let entries = list["messages"].as_array().unwrap();
    assert_eq!(entries[0]["name"], REPORT_NAME);
    assert_eq!(entries[0]["from"], "wid-957-worker");
    assert_eq!(entries[0]["worktree"], "/path/to/widget-wid-957");
    assert_eq!(entries[0]["kind"], "report");
    assert_eq!(entries[0]["subject"], "Retry added");

    assert_eq!(fixture.ok(&["pending", "--read", REPORT_NAME]), REPORT);

    fixture.ok(&["pending", "--ack", REPORT_NAME]);
    let filed = inbox(&fixture).join("read").join(REPORT_NAME);
    assert_eq!(std::fs::read_to_string(filed).unwrap(), REPORT);
    assert!(!inbox(&fixture).join(REPORT_NAME).exists());
}

/// Written the way `hold` names it, `.acking-<pid>-<attempt>-<name>`. No command leaves one
/// behind for good: it is what an `ack` that died between its two moves left.
fn leave_held(fixture: &Fixture, age_secs: u64) -> PathBuf {
    let held = inbox(fixture).join(format!(".acking-4242-0-{HELD_NAME}"));
    put(&held, HELD);
    let then = std::time::SystemTime::now() - std::time::Duration::from_secs(age_secs);
    std::fs::File::options()
        .write(true)
        .open(&held)
        .unwrap()
        .set_modified(then)
        .unwrap();
    held
}

#[test]
fn a_held_inbox_entry_is_put_back_once_it_is_stale() {
    let fixture = Fixture::new(QUIET);
    let held = leave_held(&fixture, 120);

    let list = fixture.json(&["pending", "--json"]);
    assert_eq!(list["count"], 1, "{list}");
    assert_eq!(list["messages"][0]["name"], HELD_NAME);
    assert!(!held.exists());
    assert_eq!(fixture.ok(&["pending", "--read", HELD_NAME]), HELD);
}

#[test]
fn a_held_inbox_entry_is_left_alone_while_in_flight() {
    let fixture = Fixture::new(QUIET);
    let held = leave_held(&fixture, 0);

    let list = fixture.json(&["pending", "--json"]);
    assert_eq!(list["count"], 0, "{list}");
    assert_eq!(std::fs::read_to_string(&held).unwrap(), HELD);
}

fn resumable_config(fixture: &Fixture) {
    std::fs::write(
        &fixture.config,
        serde_json::json!({
            "notification": "true",
            "hubRunner": "true {name} {sessionId} {prompt}",
            "agentRunner": "true {sessionId} {prompt}",
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget"}},
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn a_saved_hub_session_is_resumed_from() {
    let fixture = Fixture::new(QUIET);
    resumable_config(&fixture);
    let saved = hub_session(&fixture);
    put(&saved, HUB_SESSION);
    let sid = parse(HUB_SESSION)["sessionId"]
        .as_str()
        .unwrap()
        .to_string();

    let resumed = fixture.ok(&["hub", "--resume", "--dry-run"]);
    assert!(
        resumed.contains(&format!("claude -n {HUB} --resume {sid} ")),
        "{resumed}"
    );
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), HUB_SESSION);
}

#[test]
fn a_saved_worker_session_is_resumed_from() {
    let fixture = Fixture::new(QUIET);
    resumable_config(&fixture);
    let saved = worker_session(&fixture);
    put(&saved, WORKER_SESSION);
    let sid = parse(WORKER_SESSION)["sessionId"]
        .as_str()
        .unwrap()
        .to_string();

    // Typed from inside the worktree with nothing else said: the session comes from the file.
    let resumed = fixture.ok(&["worker", "--resume", "--dry-run"]);
    assert!(
        resumed.contains(&format!("claude --resume {sid} --permission-mode auto ")),
        "{resumed}"
    );
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), WORKER_SESSION);
}

#[test]
fn a_gate_record_is_listed_shown_and_closed_with_keys_it_does_not_know() {
    let fixture = Fixture::new(QUIET);
    put(&gate_file(&fixture, ""), GATE);
    let before = parse(GATE);

    let list = fixture.json(&["gate", "list", "--json"]);
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list[0]["id"], GATE_ID);
    assert_eq!(list[0]["x-unknown"], before["x-unknown"]);

    let shown = fixture.json(&["gate", "show", "--id", GATE_ID]);
    assert_eq!(shown["x-unknown"], before["x-unknown"], "{shown}");

    let closed = fixture.json(&[
        "gate",
        "close",
        "--id",
        GATE_ID,
        "--comment",
        "dealt with in tab",
        "--json",
    ]);
    assert_eq!(closed["gate"]["x-unknown"], before["x-unknown"]);

    assert!(!gate_file(&fixture, "").exists());
    let archived = read_json(&gate_file(&fixture, "answered"));
    assert_eq!(archived["decision"], "closed");
    assert_eq!(archived["comment"], "dealt with in tab");
    assert!(archived["answeredAt"].is_string(), "{archived}");
    keeps_every_key(&before, &archived, &[]);
}

#[test]
fn a_task_record_is_shown_and_updated_with_keys_it_does_not_know() {
    let fixture = Fixture::new(QUIET);
    let path = task_file(&fixture);
    put(&path, TASK);
    let before = parse(TASK);

    let shown = fixture.json(&["task", "show", "--id", TASK_ID]);
    assert_eq!(shown["x-unknown"], before["x-unknown"], "{shown}");
    let list = fixture.json(&["task", "list", "--json"]);
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list[0]["x-unknown"], before["x-unknown"]);

    fixture.json(&[
        "task", "update", "--id", TASK_ID, "--note", "again", "--json",
    ]);

    let after = read_json(&path);
    assert_eq!(after["note"], "again");
    assert_ne!(after["updatedAt"], before["updatedAt"], "{after}");
    keeps_every_key(&before, &after, &["note", "updatedAt"]);
}

#[test]
fn closing_a_gate_writes_onto_its_task_without_losing_keys() {
    let fixture = Fixture::new(QUIET);
    put(&gate_file(&fixture, ""), GATE);
    let path = task_file(&fixture);
    put(&path, TASK);
    let before = parse(TASK);
    assert_eq!(parse(GATE)["task"], TASK_ID);

    fixture.ok(&["gate", "close", "--id", GATE_ID, "--comment", "c", "--json"]);

    let after = read_json(&path);
    assert!(after["gateAnsweredAt"].is_string(), "{after}");
    keeps_every_key(&before, &after, &["gateAnsweredAt", "updatedAt"]);
}
