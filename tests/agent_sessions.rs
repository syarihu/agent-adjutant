//! The agent session ledger, through the commands: payloads fed to `adj hook claude` on stdin
//! (`src/fixtures/hooks/claude/`, see its README) and the rows read back with `adj agent-sessions`.

mod common;

use common::*;

type Json = serde_json::Value;

const SESSION: &str = "0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d";

fn payload(name: &str) -> Json {
    let text = match name {
        "session-start" => include_str!("../src/fixtures/hooks/claude/session-start.json"),
        "user-prompt-submit" => {
            include_str!("../src/fixtures/hooks/claude/user-prompt-submit.json")
        }
        "post-tool-use" => include_str!("../src/fixtures/hooks/claude/post-tool-use.json"),
        "post-tool-use-failure" => {
            include_str!("../src/fixtures/hooks/claude/post-tool-use-failure.json")
        }
        "permission-request" => {
            include_str!("../src/fixtures/hooks/claude/permission-request.json")
        }
        "notification-permission" => {
            include_str!("../src/fixtures/hooks/claude/notification-permission.json")
        }
        "notification-idle" => include_str!("../src/fixtures/hooks/claude/notification-idle.json"),
        "subagent-start" => include_str!("../src/fixtures/hooks/claude/subagent-start.json"),
        "subagent-post-tool-use" => {
            include_str!("../src/fixtures/hooks/claude/subagent-post-tool-use.json")
        }
        "subagent-permission-request" => {
            include_str!("../src/fixtures/hooks/claude/subagent-permission-request.json")
        }
        "subagent-stop" => include_str!("../src/fixtures/hooks/claude/subagent-stop.json"),
        "stop" => include_str!("../src/fixtures/hooks/claude/stop.json"),
        "stop-failure" => include_str!("../src/fixtures/hooks/claude/stop-failure.json"),
        "session-end" => include_str!("../src/fixtures/hooks/claude/session-end.json"),
        "status-line" => include_str!("../src/fixtures/hooks/claude/status-line.json"),
        other => panic!("no fixture called {other}"),
    };
    serde_json::from_str(text).unwrap()
}

const ALL: [&str; 14] = [
    "session-start",
    "user-prompt-submit",
    "post-tool-use",
    "post-tool-use-failure",
    "permission-request",
    "notification-permission",
    "notification-idle",
    "subagent-start",
    "subagent-post-tool-use",
    "subagent-permission-request",
    "subagent-stop",
    "stop",
    "stop-failure",
    "session-end",
];

fn fixture() -> Fixture {
    Fixture::new(QUIET)
}

/// The fixture's payload as a session in the fixture repository sends it.
fn sent(fixture: &Fixture, name: &str) -> Json {
    let mut payload = payload(name);
    payload["cwd"] = fixture.repo.to_string_lossy().to_string().into();
    payload
}

/// `adj hook claude` as the agent `pid` runs it, not yet waited for.
fn start(command: &mut Command, input: &[u8], pid: u32) -> std::process::Child {
    let mut child = command
        .env("CLAUDE_PID", pid.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child
}

fn hook_as(fixture: &Fixture, payload: &Json, pid: u32) -> std::process::Output {
    let mut command = fixture.command(["hook", "claude"]);
    start(&mut command, payload.to_string().as_bytes(), pid)
        .wait_with_output()
        .unwrap()
}

/// As this test process, which is alive for as long as the test runs.
fn hook(fixture: &Fixture, payload: &Json) -> std::process::Output {
    hook_as(fixture, payload, std::process::id())
}

fn quiet_success(out: &std::process::Output) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn rows(fixture: &Fixture) -> Vec<Json> {
    fixture
        .json(&["agent-sessions", "--json"])
        .as_array()
        .unwrap()
        .clone()
}

fn row_path(fixture: &Fixture, id: &str) -> PathBuf {
    fixture
        .state
        .join("agent-sessions")
        .join(format!("{id}.json"))
}

/// `adj hook claude --status-line` fed `input`, as a status line script runs it.
fn relay(fixture: &Fixture, input: &[u8]) -> std::process::Output {
    let mut command = fixture.command(["hook", "claude", "--status-line"]);
    start(&mut command, input, std::process::id())
        .wait_with_output()
        .unwrap()
}

#[test]
fn a_turn_ends_done_with_the_worktree_the_pid_and_its_start_time() {
    let fx = fixture();
    for name in ["session-start", "user-prompt-submit", "post-tool-use"] {
        quiet_success(&hook(&fx, &sent(&fx, name)));
    }
    let running = rows(&fx);
    assert_eq!(running[0]["status"], "running");
    assert_eq!(running[0]["activity"], "Edit: /home/user/repo/src/lib.rs");

    quiet_success(&hook(&fx, &sent(&fx, "stop")));
    let rows = rows(&fx);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["sessionId"], SESSION);
    assert_eq!(row["agent"], "claude");
    assert_eq!(row["status"], "done");
    assert_eq!(row["worktree"], fx.repo.to_string_lossy().as_ref());
    assert_eq!(row["cwd"], fx.repo.to_string_lossy().as_ref());
    assert_eq!(row["pid"], std::process::id());
    assert!(!row["psStarted"].as_str().unwrap().trim().is_empty());
    assert!(row["configDir"].as_str().unwrap().ends_with(".claude"));
    assert!(row.get("activity").is_none() && row.get("request").is_none());
}

#[test]
fn only_a_permission_request_prints_anything_and_every_event_exits_zero() {
    let fx = fixture();
    for name in ALL {
        let out = hook(&fx, &sent(&fx, name));
        quiet_success(&out);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        match name {
            "permission-request" | "subagent-permission-request" => {
                assert_eq!(stdout, "{}\n", "{name}")
            }
            _ => assert_eq!(stdout, "", "{name}"),
        }
    }
    // The entries `adj setup` writes into a global file say so; the receiver acts the same.
    let mut command = fx.command(["hook", "claude", "--global"]);
    let global = start(
        &mut command,
        sent(&fx, "session-start").to_string().as_bytes(),
        1,
    )
    .wait_with_output()
    .unwrap();
    quiet_success(&global);
}

#[test]
fn what_cannot_be_recorded_is_said_on_stderr_and_still_exits_zero() {
    let fx = fixture();
    let mut bad_id = sent(&fx, "session-start");
    bad_id["session_id"] = "../x".into();
    let mut permission = sent(&fx, "permission-request");
    permission["session_id"] = "".into();
    let std::process::Output {
        status,
        stdout,
        stderr,
    } = {
        let mut command = fx.command(["hook", "claude"]);
        start(&mut command, b"this is not json", 1)
            .wait_with_output()
            .unwrap()
    };
    assert!(status.success() && stdout.is_empty() && !stderr.is_empty());

    for (input, said) in [(bad_id, ""), (permission, "{}\n")] {
        let out = hook(&fx, &input);
        quiet_success(&out);
        assert!(!out.stderr.is_empty());
        // A refused permission request is still answered: the agent is waiting for the line.
        assert_eq!(String::from_utf8_lossy(&out.stdout), said);
    }
    let mut command = fx.command(["hook", "agy"]);
    let out = start(
        &mut command,
        sent(&fx, "session-start").to_string().as_bytes(),
        1,
    )
    .wait_with_output()
    .unwrap();
    quiet_success(&out);
    assert!(!out.stderr.is_empty());
    assert!(!fx.state.join("agent-sessions").exists());
}

#[test]
fn overlapping_events_of_one_session_are_all_kept() {
    let fx = fixture();
    let mut children = Vec::new();
    let mut tool = sent(&fx, "post-tool-use");
    tool["tool_input"] = serde_json::json!({ "file_path": "src/lib.rs" });
    let mut command = fx.command(["hook", "claude"]);
    children.push(start(
        &mut command,
        tool.to_string().as_bytes(),
        std::process::id(),
    ));
    for n in 0..6 {
        let mut sub = sent(&fx, "subagent-start");
        sub["agent_id"] = format!("agent-{n}").into();
        let mut command = fx.command(["hook", "claude"]);
        children.push(start(
            &mut command,
            sub.to_string().as_bytes(),
            std::process::id(),
        ));
    }
    for child in children {
        quiet_success(&child.wait_with_output().unwrap());
    }
    let rows = rows(&fx);
    assert_eq!(rows.len(), 1);
    let mut ids: Vec<&str> = rows[0]["subagents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sub| sub["id"].as_str().unwrap())
        .collect();
    ids.sort();
    assert_eq!(
        ids,
        [
            "agent-0", "agent-1", "agent-2", "agent-3", "agent-4", "agent-5"
        ]
    );
    assert_eq!(rows[0]["activity"], "Edit: src/lib.rs");
}

#[test]
fn a_done_that_comes_while_a_subagent_runs_is_held_until_it_stops() {
    let fx = fixture();
    hook(&fx, &sent(&fx, "subagent-start"));
    hook(&fx, &sent(&fx, "stop"));
    let held = rows(&fx);
    assert_eq!(held[0]["status"], "running");
    assert_eq!(held[0]["pendingStatus"], "done");
    assert_eq!(held[0]["subagents"].as_array().unwrap().len(), 1);

    hook(&fx, &sent(&fx, "subagent-stop"));
    let done = rows(&fx);
    assert_eq!(done[0]["status"], "done");
    assert!(done[0].get("pendingStatus").is_none());
    assert!(done[0].get("subagents").is_none());
}

#[test]
fn a_session_whose_process_has_gone_is_left_out_and_removed_by_another_sessions_start() {
    let fx = fixture();
    let mut exited = std::process::Command::new("true").spawn().unwrap();
    let gone = exited.id();
    exited.wait().unwrap();
    let mut closed = sent(&fx, "user-prompt-submit");
    closed["session_id"] = "closed-tab".into();
    quiet_success(&hook_as(&fx, &closed, gone));
    assert!(row_path(&fx, "closed-tab").exists());
    assert!(rows(&fx).is_empty());
    assert!(row_path(&fx, "closed-tab").exists());

    quiet_success(&hook(&fx, &sent(&fx, "session-start")));
    assert!(!row_path(&fx, "closed-tab").exists());
    assert!(
        !fx.state
            .join("agent-sessions")
            .join("closed-tab.lock")
            .exists()
    );
    let rows = rows(&fx);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["sessionId"], SESSION);
}

#[test]
fn a_relative_state_dir_is_taken_against_the_main_checkout_of_the_payloads_cwd() {
    let fx = fixture();
    let elsewhere = tempfile::tempdir().unwrap();
    let mut command = fx.command(["hook", "claude"]);
    command
        .env("ADJUTANT_STATE_DIR", "rel")
        .current_dir(elsewhere.path());
    let out = start(
        &mut command,
        sent(&fx, "session-start").to_string().as_bytes(),
        std::process::id(),
    )
    .wait_with_output()
    .unwrap();
    quiet_success(&out);
    assert!(
        fx.repo
            .join("rel")
            .join("agent-sessions")
            .join(format!("{SESSION}.json"))
            .exists()
    );
    assert!(!elsewhere.path().join("rel").exists());
    assert!(!fx.state.join("agent-sessions").exists());
}

#[test]
fn the_list_names_the_status_and_the_session_and_says_when_there_is_none() {
    let fx = fixture();
    assert_eq!(fx.ok(&["agent-sessions"]), "No agent sessions.\n");
    assert_eq!(fx.ok(&["agent-sessions", "--json"]), "[]\n");
    hook(&fx, &sent(&fx, "session-start"));
    hook(&fx, &sent(&fx, "permission-request"));
    let text = fx.ok(&["agent-sessions"]);
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.starts_with("waiting"), "{text}");
    assert!(text.contains(SESSION), "{text}");
    assert!(text.contains("Bash: rm -rf target"), "{text}");
}

#[test]
fn the_receiver_is_hidden_from_the_help_and_the_listing_is_not() {
    let fx = fixture();
    let help = fx.ok(&["--help"]);
    assert!(
        help.lines()
            .any(|line| line.trim_start().starts_with("agent-sessions")),
        "{help}"
    );
    assert!(
        !help
            .lines()
            .any(|line| line.trim_start().starts_with("hook")),
        "{help}"
    );
}

#[test]
fn the_status_line_relay_fills_an_existing_row_and_prints_nothing() {
    let fx = fixture();
    let draw = sent(&fx, "status-line").to_string();

    let out = relay(&fx, draw.as_bytes());
    quiet_success(&out);
    assert!(out.stdout.is_empty());
    assert!(rows(&fx).is_empty());
    assert!(!row_path(&fx, SESSION).exists());

    quiet_success(&hook(&fx, &sent(&fx, "session-start")));
    let out = relay(&fx, draw.as_bytes());
    quiet_success(&out);
    assert!(out.stdout.is_empty());
    let rows = rows(&fx);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["model"], "Opus");
    assert_eq!(rows[0]["contextPercent"], 43.0);
    assert_eq!(rows[0]["rateLimits"]["fiveHour"]["usedPercent"], 23.5);
    assert_eq!(rows[0]["status"], "idle");
}

#[test]
fn the_status_line_relay_never_fails_the_status_line() {
    let fx = fixture();
    let mut bad_id = sent(&fx, "status-line");
    bad_id["session_id"] = "../x".into();
    for input in [b"not json".to_vec(), bad_id.to_string().into_bytes()] {
        let out = relay(&fx, &input);
        assert!(out.status.success());
        assert!(out.stdout.is_empty());
        assert!(!out.stderr.is_empty());
    }
}

/// A Codex payload as a session in the fixture repository sends it.
fn codex_sent(fixture: &Fixture, name: &str) -> Json {
    let text = match name {
        "session-start" => include_str!("../src/fixtures/hooks/codex/session-start.json"),
        "permission-request" => include_str!("../src/fixtures/hooks/codex/permission-request.json"),
        "interrupt" => include_str!("../src/fixtures/hooks/codex/interrupt.json"),
        other => panic!("no fixture called {other}"),
    };
    let mut payload: Json = serde_json::from_str(text).unwrap();
    payload["cwd"] = fixture.repo.to_string_lossy().to_string().into();
    payload
}

fn codex_hook(fixture: &Fixture, payload: &Json) -> std::process::Output {
    let mut command = fixture.command(["hook", "codex"]);
    // A live pid in `CLAUDE_PID`, which Codex's row must not pick up.
    start(
        &mut command,
        payload.to_string().as_bytes(),
        std::process::id(),
    )
    .wait_with_output()
    .unwrap()
}

#[test]
fn a_codex_hook_records_a_row_without_a_pid_and_prints_nothing() {
    let fx = fixture();
    for name in ["session-start", "permission-request"] {
        let out = codex_hook(&fx, &codex_sent(&fx, name));
        quiet_success(&out);
        // Codex reads stdout as a decision, so even a permission request is answered with nothing.
        assert!(out.stdout.is_empty(), "{name}");
    }
    let rows = rows(&fx);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["agent"], "codex");
    assert_eq!(rows[0]["sessionId"], SESSION);
    assert_eq!(rows[0]["status"], "waiting");
    assert!(rows[0].get("pid").is_none());

    quiet_success(&codex_hook(&fx, &codex_sent(&fx, "interrupt")));
    assert_eq!(self::rows(&fx)[0]["status"], "done");
}
