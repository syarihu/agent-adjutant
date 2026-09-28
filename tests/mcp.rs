//! The MCP server, spoken to over stdio the way an agent would.

mod common;

use common::*;

/// The same driver as `common::mcp`, but the input is bytes — a line that is not valid UTF-8
/// cannot be written any other way, and that is the whole point of the test below.
fn mcp_raw(fixture: &Fixture, lines: &[&[u8]]) -> Vec<serde_json::Value> {
    let mut child = fixture
        .command(["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for line in lines {
            stdin.write_all(line).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect(l))
        .collect()
}

fn tool_result(response: &serde_json::Value) -> serde_json::Value {
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).expect(text)
}

#[test]
fn the_server_handshakes_serves_the_procedures_and_answers_about_the_repo() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp(
        &fixture,
        &[
            request(1, "initialize", serde_json::json!({})),
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            request(2, "prompts/list", serde_json::json!({})),
            request(
                3,
                "prompts/get",
                serde_json::json!({"name": "adj-report", "arguments": {"arguments": "画像が潰れる"}}),
            ),
            request(4, "tools/list", serde_json::json!({})),
            request(
                5,
                "tools/call",
                serde_json::json!({"name": "adjutant_hub_status", "arguments": {}}),
            ),
        ],
    );
    // Five requests, one notification: the notification must not produce a sixth line.
    assert_eq!(replies.len(), 5, "{replies:#?}");

    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "adjutant");
    assert_eq!(replies[1]["result"]["prompts"].as_array().unwrap().len(), 3);
    let procedure = replies[2]["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(procedure.contains("画像が潰れる"));
    assert!(
        !procedure.starts_with("---"),
        "frontmatter leaked into the procedure"
    );
    assert_eq!(replies[3]["result"]["tools"].as_array().unwrap().len(), 10);

    let hub = tool_result(&replies[4]);
    assert_eq!(hub["hubName"], HUB);
    assert_eq!(hub["present"], false);
    assert_eq!(hub["waiting"], 0);
}

#[test]
fn a_report_sent_through_the_server_lands_where_the_cli_looks_for_it() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_send", "arguments": {
                "from": "wid-1-worker",
                "subject": "検索結果の画像が縦に潰れる",
                "body": "## Symptom\nthe image is squashed",
            }}),
        )],
    );
    let sent = tool_result(&replies[0]);
    assert_eq!(sent["present"], false);
    assert_eq!(sent["hubName"], HUB);

    // Same inbox from both directions — that is the whole point of the file-based channel.
    let listed = fixture.json(&["pending", "--json"]);
    assert_eq!(listed["count"], 1);
    assert_eq!(
        listed["messages"][0]["subject"],
        "検索結果の画像が縦に潰れる"
    );
    // With no `cwd` given, the sender is where the server is standing.
    assert_eq!(
        listed["messages"][0]["worktree"],
        fixture.repo.to_string_lossy().to_string()
    );

    // And with one, it is where the *caller* is standing. This is the case that matters:
    // one server is started per session and then asked about whichever worktree the worker
    // is working in, so the server's own directory is nobody's address.
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-2");
    let added = Command::new("git")
        .hermetic()
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "wid-2",
            worktree.to_str().unwrap(),
        ])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_send", "arguments": {
                "from": "wid-2-worker",
                "kind": "done",
                "subject": "Done",
                "body": "b",
                "cwd": worktree.to_string_lossy(),
            }}),
        )],
    );
    let listed = fixture.json(&["pending", "--json"]);
    let done = listed["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "done")
        .unwrap();
    assert_eq!(done["worktree"], worktree.to_string_lossy().to_string());
}

#[test]
fn the_config_tool_and_the_config_subcommand_agree() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_config", "arguments": {}}),
        )],
    );
    assert_eq!(tool_result(&replies[0]), fixture.json(&["config"]));
}

/// One byte used to end the session. `lines()` reports a line that is not valid UTF-8 as an
/// error, that was read as end-of-input, and every later call in that session failed with
/// nothing said about why.
#[test]
fn one_undecodable_byte_does_not_take_the_server_down_with_it() {
    let fixture = Fixture::new(QUIET);
    let good = request(1, "ping", serde_json::json!({})).to_string();
    let after = request(2, "ping", serde_json::json!({})).to_string();
    let replies = mcp_raw(
        &fixture,
        &[
            good.as_bytes(),
            // A lone continuation byte: valid JSON is impossible here, and so is UTF-8.
            b"{\"jsonrpc\": \"2.0\", \"id\": 9, \"method\": \"\xff\"}",
            after.as_bytes(),
        ],
    );
    assert_eq!(replies.len(), 3, "{replies:#?}");
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[1]["error"]["code"], -32700);
    // The session is still there afterwards, which is the part that was broken.
    assert_eq!(replies[2]["id"], 2);
    assert!(replies[2]["result"].is_object(), "{replies:#?}");
}

/// Three ways an id can arrive, and they mean three different things.
#[test]
fn an_id_that_is_null_is_a_request_and_an_absent_one_is_not() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp_raw(
        &fixture,
        &[
            // Present and null: a request. Its answer carries `"id": null`.
            br#"{"jsonrpc": "2.0", "id": null, "method": "ping", "params": {}}"#,
            // Absent: a notification, and answering one is a protocol error.
            br#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#,
            // Unparseable: the error has to carry an id too, and null is the one the
            // specification names for "cannot be matched to a request".
            br#"{not json at all"#,
        ],
    );
    assert_eq!(replies.len(), 2, "{replies:#?}");
    assert!(replies[0]["result"].is_object(), "{replies:#?}");
    for reply in &replies {
        // Present and null — not missing. A client matching replies to requests by id has
        // nothing to work with when the field is simply absent.
        assert!(reply.as_object().unwrap().contains_key("id"), "{reply:#?}");
        assert!(reply["id"].is_null(), "{reply:#?}");
    }
    assert_eq!(replies[1]["error"]["code"], -32700);
}

/// Well-formed JSON is not the same thing as a well-formed request, and answering the
/// second kind of mistake with a *parse* error told the client to look in the wrong place.
#[test]
fn a_request_that_is_not_a_request_is_told_which_of_the_two_it_got_wrong() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp_raw(
        &fixture,
        &[
            // Parses; has no method. Silence would be the answer to a notification, and
            // this is not one — a client that gets silence waits for ever.
            br#"{}"#,
            // serde will read a struct out of a sequence, so this used to be accepted and
            // answered as a call to `ping`.
            br#"["2.0", 1, "ping", {}]"#,
            // An id has to be something a reply can be matched by.
            br#"{"jsonrpc": "2.0", "id": true, "method": "ping"}"#,
            // The version is wrong, but this one *is* a request, so its id comes back.
            br#"{"jsonrpc": "1.0", "id": 8, "method": "ping"}"#,
        ],
    );
    assert_eq!(replies.len(), 4, "{replies:#?}");
    for reply in &replies {
        // The structure was wrong, not the syntax.
        assert_eq!(reply["error"]["code"], -32600, "{reply:#?}");
    }
    for reply in &replies[..3] {
        assert!(reply["id"].is_null(), "{reply:#?}");
    }
    assert_eq!(replies[3]["id"], 8);
}

#[test]
fn install_mcp_rejects_unknown_target() {
    let fixture = Fixture::new(QUIET);
    let out = fixture
        .command(["install-mcp", "--target", "invalid-target"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("target must be claude-code, agy, or json"));
}

#[test]
fn a_record_kept_through_the_server_is_one_the_cli_can_show() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_gate_open", "arguments": {
                "kind": "verify",
                "wait": false,
                "title": "cargo test が通った",
                "commands": [{ "command": "cargo test", "result": "pass", "time": "42s" }],
                "manual": ["画面の文言を見る"],
                // What a client that fills in every property sends for one it has no value
                // for. Taken as absent, not as an address.
                "worktree": "",
                "cwd": fixture.repo.to_str().unwrap(),
            }}),
        )],
    );
    let opened = tool_result(&replies[0]);
    assert_eq!(opened["wait"], false, "{opened}");
    let id = opened["gate"]["id"].as_str().unwrap();
    // The answer's address is where the caller stands, not where the server does.
    assert_eq!(
        opened["gate"]["worktree"],
        fixture.repo.to_string_lossy().to_string()
    );

    let shown = fixture.json(&["gate", "show", "--id", id]);
    assert_eq!(shown["commands"][0]["result"], "pass", "{shown}");
    assert_eq!(shown["manual"][0], "画面の文言を見る", "{shown}");
    assert!(
        fixture
            .json(&["gate", "list", "--json"])
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_waiting_gate_opened_and_closed_through_the_server() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({
                "name": "adjutant_gate_open",
                "arguments": {
                    "kind": "plan",
                    "title": "計画の承認",
                    "problem": "問題",
                    "goal": "目標",
                    "cwd": fixture.repo.to_str().unwrap(),
                }
            }),
        )],
    );
    let opened = tool_result(&replies[0]);
    assert_eq!(opened["server"], "down");
    assert!(
        opened["wakeLine"]
            .as_str()
            .unwrap()
            .contains("adjutant_outbox")
    );
    let id = opened["gate"]["id"].as_str().unwrap();

    let replies = mcp(
        &fixture,
        &[request(
            2,
            "tools/call",
            serde_json::json!({
                "name": "adjutant_gate_close",
                "arguments": {
                    "id": id,
                    "comment": "answered in terminal",
                    "cwd": fixture.repo.to_str().unwrap(),
                }
            }),
        )],
    );
    let closed = tool_result(&replies[0]);
    assert_eq!(closed["closed"], true);
    assert_eq!(closed["alreadyAnswered"], false);
    assert_eq!(closed["gate"]["decision"], "closed");
    assert_eq!(closed["gate"]["comment"], "answered in terminal");

    // Calling close again on an already archived gate succeeds idempotently
    let replies = mcp(
        &fixture,
        &[request(
            3,
            "tools/call",
            serde_json::json!({
                "name": "adjutant_gate_close",
                "arguments": {
                    "id": id,
                    "cwd": fixture.repo.to_str().unwrap(),
                }
            }),
        )],
    );
    let closed_again = tool_result(&replies[0]);
    assert_eq!(closed_again["closed"], true);
    assert_eq!(closed_again["alreadyAnswered"], false);

    assert!(
        fixture
            .json(&["gate", "list", "--json"])
            .as_array()
            .unwrap()
            .is_empty()
    );

    // When a gate was already answered on the board, closing it reports alreadyAnswered
    let replies = mcp(
        &fixture,
        &[request(
            4,
            "tools/call",
            serde_json::json!({
                "name": "adjutant_gate_open",
                "arguments": {
                    "kind": "question",
                    "title": "Board question",
                    "cwd": fixture.repo.to_str().unwrap(),
                }
            }),
        )],
    );
    let opened2 = tool_result(&replies[0]);
    let id2 = opened2["gate"]["id"].as_str().unwrap();
    fixture.ok(&["gate", "answer", "--id", id2, "--decision", "approve"]);

    let replies = mcp(
        &fixture,
        &[request(
            5,
            "tools/call",
            serde_json::json!({
                "name": "adjutant_gate_close",
                "arguments": {
                    "id": id2,
                    "cwd": fixture.repo.to_str().unwrap(),
                }
            }),
        )],
    );
    let close_answered = tool_result(&replies[0]);
    assert_eq!(close_answered["closed"], false);
    assert_eq!(close_answered["alreadyAnswered"], true);
    assert_eq!(close_answered["gate"]["decision"], "approve");

    // An MCP process running outside the repository (e.g. in state directory)
    // resolves worktree from the passed cwd.
    let replies = mcp_in(
        &fixture,
        &fixture.state,
        &[request(
            6,
            "tools/call",
            serde_json::json!({
                "name": "adjutant_outbox",
                "arguments": {
                    "action": "read",
                    "cwd": fixture.repo.to_str().unwrap(),
                }
            }),
        )],
    );
    let outbox = tool_result(&replies[0]);
    assert!(outbox["content"].as_str().unwrap().contains("approve"));

    // An empty worktree string also falls back to cwd
    let replies = mcp_in(
        &fixture,
        &fixture.state,
        &[request(
            7,
            "tools/call",
            serde_json::json!({
                "name": "adjutant_outbox",
                "arguments": {
                    "action": "read",
                    "worktree": "",
                    "cwd": fixture.repo.to_str().unwrap(),
                }
            }),
        )],
    );
    let outbox_empty_wt = tool_result(&replies[0]);
    assert!(
        outbox_empty_wt["content"]
            .as_str()
            .unwrap()
            .contains("approve")
    );
}

#[test]
fn a_gate_opened_with_waking_off_omits_the_wake_line() {
    let fixture = Fixture::new(
        r#"{"workerWake": false,
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
                      "issueKeys": {"acme/widget": "WID"}, "ide": "code"}}}"#,
    );
    let replies = mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({
                "name": "adjutant_gate_open",
                "arguments": {
                    "kind": "plan",
                    "title": "計画の承認",
                    "problem": "問題",
                    "goal": "目標",
                    "cwd": fixture.repo.to_str().unwrap(),
                }
            }),
        )],
    );
    let opened = tool_result(&replies[0]);
    assert!(opened.get("wakeLine").is_none());
}

/// An MCP server started with a hub's line, talked to until it has said where the board is.
/// Returns the child, still running, and what `adjutant_config` answered.
fn hub_mcp(fixture: &Fixture, slug: &str) -> (std::process::Child, serde_json::Value) {
    hub_mcp_as(fixture, None, slug)
}

/// The same, for a hub named `hub` — `adj hub --hub` puts it on the line as `ADJUTANT_HUB`.
fn hub_mcp_as(
    fixture: &Fixture,
    hub: Option<&str>,
    slug: &str,
) -> (std::process::Child, serde_json::Value) {
    let mut command = fixture.command(["mcp"]);
    if let Some(hub) = hub {
        command.env("ADJUTANT_HUB", hub);
    }
    let mut child = command
        .env("ADJUTANT_HUB_SERVE", slug)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let call = request(
        1,
        "tools/call",
        serde_json::json!({"name": "adjutant_config", "arguments": {}}),
    );
    writeln!(child.stdin.as_mut().unwrap(), "{call}").unwrap();
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    let reply: serde_json::Value = serde_json::from_str(&line).expect(&line);
    (child, tool_result(&reply))
}

/// `GET` the board's page, and answer with the status line.
fn fetch(url: &str) -> String {
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_once('/').unwrap();
    let mut stream = std::net::TcpStream::connect(host).unwrap();
    write!(
        stream,
        "GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut status = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut status)
        .unwrap();
    status
}

/// `GET /api/state` from the board, whole.
fn fetch_state(url: &str) -> serde_json::Value {
    let rest = url.strip_prefix("http://").unwrap();
    let (host, query) = rest.split_once('/').unwrap();
    let token = query.split("token=").nth(1).unwrap();
    let mut stream = std::net::TcpStream::connect(host).unwrap();
    write!(
        stream,
        "GET /api/state?token={token} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut answer = String::new();
    std::io::Read::read_to_string(&mut stream, &mut answer).unwrap();
    let body = answer.split_once("\r\n\r\n").unwrap().1;
    serde_json::from_str(body).unwrap()
}

#[test]
fn the_board_state_reports_hubs_and_sessions_alongside_hub_and_workers() {
    let fixture = Fixture::new(QUIET);
    let (mut board, url) = start_board(&fixture);

    let state = fetch_state(&url);
    // Legacy fields preserved for existing board callers
    assert!(state.get("hub").is_some());
    assert!(state.get("workers").is_some());

    // New fields: hubs and sessions
    let hubs = state["hubs"].as_array().expect("hubs array");
    assert!(!hubs.is_empty());
    let repo_hub = &hubs[0];
    assert_eq!(repo_hub["id"], "hub");
    assert!(repo_hub["name"].as_str().unwrap().contains("adjutant-"));
    assert!(repo_hub.get("state").is_some());
    assert!(repo_hub["state"].get("present").is_some());
    assert!(repo_hub["state"].get("stale").is_some());
    assert!(repo_hub.get("slug").is_some());
    assert!(repo_hub.get("inboxCount").is_some());

    let sessions = state["sessions"].as_array().expect("sessions array");
    assert!(
        sessions
            .iter()
            .any(|s| s["kind"] == "hub" && s["id"] == "hub")
    );
    for session in sessions {
        assert!(session.get("terminal").is_some());
        assert!(session.get("agent").is_some());
        assert!(session.get("present").is_some());
        assert!(session.get("stale").is_some());
    }

    board.kill().unwrap();
    board.wait().unwrap();
}

#[test]
fn the_board_state_reports_worker_session_with_metadata() {
    let fixture = Fixture::new(QUIET);
    let worktree_dir = fixture.repo.join("worktree-wid-1");
    let out = Command::new("git")
        .hermetic()
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "wid-1",
            worktree_dir.to_str().unwrap(),
        ])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let pid = std::process::id();
    let claude_dir = worktree_dir.join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(
        claude_dir.join("adjutant-worker.json"),
        serde_json::json!({
            "pid": pid,
            "title": "WID-1 Fix widget",
            "task": "task-wid-1",
            "hub": "parent-1",
            "phase": "implement",
            "phaseAt": 1700000000,
            "psStarted": ps_started(pid),
        })
        .to_string(),
    )
    .unwrap();

    let (mut board, url) = start_board(&fixture);
    let state = fetch_state(&url);

    // hubs must discover parent-1 from the worktree's worker record
    let hubs = state["hubs"].as_array().expect("hubs array");
    assert!(hubs.iter().any(|h| h["key"] == "parent-1"));

    // sessions must contain the worker session
    let sessions = state["sessions"].as_array().expect("sessions array");
    let worker_sess = sessions
        .iter()
        .find(|s| s["kind"] == "worker")
        .expect("worker session found");
    assert_eq!(
        worker_sess["worktree"],
        worktree_dir
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(worker_sess["task"], "task-wid-1");
    assert_eq!(worker_sess["hub"], "hub-parent-1");
    assert_eq!(worker_sess["phase"], "implement");
    assert_eq!(worker_sess["phaseAt"], 1700000000);
    assert_eq!(worker_sess["present"], true);

    board.kill().unwrap();
    board.wait().unwrap();
}

#[test]
fn the_board_state_reports_main_worker_from_saved_session_when_unregistered() {
    let fixture = Fixture::new(QUIET);
    let claude_dir = fixture.repo.join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(
        claude_dir.join("adjutant-session.json"),
        serde_json::json!({
            "sessionId": "sid-main-worker",
            "title": "Main checkout previous worker",
            "hub": "parent-main-saved",
            "savedAt": "20260928T120000Z",
        })
        .to_string(),
    )
    .unwrap();

    let (mut board, url) = start_board(&fixture);
    let state = fetch_state(&url);

    // hubs must discover parent-main-saved from the main checkout saved session
    let hubs = state["hubs"].as_array().expect("hubs array");
    assert!(hubs.iter().any(|h| h["key"] == "parent-main-saved"));

    // sessions must contain worker-main as an inactive worker
    let sessions = state["sessions"].as_array().expect("sessions array");
    let main_worker = sessions
        .iter()
        .find(|s| s["id"] == "worker-main")
        .expect("worker-main session found");
    assert_eq!(main_worker["kind"], "worker");
    assert_eq!(main_worker["title"], "Main checkout previous worker");
    assert_eq!(main_worker["hub"], "hub-parent-main-saved");
    assert_eq!(main_worker["present"], false);
    assert_eq!(main_worker["stale"], false);
    assert!(main_worker.get("startedAt").is_none());
    assert!(main_worker.get("task").is_none());

    board.kill().unwrap();
    board.wait().unwrap();
}

#[test]
fn the_board_state_reports_linked_worker_from_saved_session_when_unregistered() {
    let fixture = Fixture::new(QUIET);
    let worktree_dir = fixture.repo.join("worktree-saved-1");
    let out = Command::new("git")
        .hermetic()
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "saved-1",
            worktree_dir.to_str().unwrap(),
        ])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(out.status.success());

    let claude_dir = worktree_dir.join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(
        claude_dir.join("adjutant-session.json"),
        serde_json::json!({
            "sessionId": "sid-linked-saved",
            "title": "Saved linked worker task",
            "hub": "parent-linked-saved",
            "savedAt": "20260928T120000Z",
        })
        .to_string(),
    )
    .unwrap();

    let (mut board, url) = start_board(&fixture);
    let state = fetch_state(&url);

    let hubs = state["hubs"].as_array().expect("hubs array");
    assert!(hubs.iter().any(|h| h["key"] == "parent-linked-saved"));

    let sessions = state["sessions"].as_array().expect("sessions array");
    let worker_sess = sessions
        .iter()
        .find(|s| s["id"] == "worker-worktree-saved-1")
        .expect("worker session found");
    assert_eq!(worker_sess["kind"], "worker");
    assert_eq!(worker_sess["title"], "Saved linked worker task");
    assert_eq!(worker_sess["hub"], "hub-parent-linked-saved");
    assert_eq!(worker_sess["present"], false);

    board.kill().unwrap();
    board.wait().unwrap();
}

#[test]
fn a_hub_s_mcp_server_serves_its_board_for_as_long_as_it_runs() {
    let fixture = Fixture::new(QUIET);
    let (mut child, config) = hub_mcp(&fixture, SLUG);
    let url = config["board"]["url"]
        .as_str()
        .expect("no board")
        .to_string();
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert!(url.contains("?token="), "{url}");
    assert!(fetch(&url).contains(" 200 "), "{url}");

    // The session ends: the pipe closes, the server exits, and the board goes with it.
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(fixture.json(&["config"])["board"].is_null());
    assert!(std::net::TcpStream::connect(url.split('/').nth(2).unwrap()).is_err());
}

#[test]
fn only_a_server_started_for_this_hub_serves_a_board() {
    // No marker: every other session on the machine.
    let fixture = Fixture::new(QUIET);
    let replies = mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_config", "arguments": {}}),
        )],
    );
    assert!(tool_result(&replies[0])["board"].is_null());
    assert!(!fixture.state.join("dashboards").exists());

    // A marker naming a hub this server does not resolve to.
    let (mut child, config) = hub_mcp(&fixture, FEATURE_SLUG);
    assert!(config["board"].is_null(), "{config}");
    drop(child.stdin.take());
    child.wait().unwrap();
    assert!(!fixture.state.join("dashboards").exists());
}

#[test]
fn a_hub_for_a_parent_task_serves_a_board_of_its_own() {
    let fixture = Fixture::new(QUIET);
    let (mut child, config) = hub_mcp_as(&fixture, Some(FEATURE), FEATURE_SLUG);
    let url = config["board"]["url"]
        .as_str()
        .expect("no board")
        .to_string();
    assert!(fetch(&url).contains(" 200 "), "{url}");
    // Recorded under that hub's slug, and the repository's own hub has none.
    assert!(
        fixture
            .state
            .join("dashboards")
            .join(format!("{FEATURE_SLUG}.json"))
            .exists()
    );
    assert_eq!(
        fixture.json(&["config", "--hub", FEATURE])["board"]["url"],
        url.as_str()
    );
    assert!(fixture.json(&["config"])["board"].is_null());
    drop(child.stdin.take());
    child.wait().unwrap();
}

fn start_board(fixture: &Fixture) -> (std::process::Child, String) {
    let mut by_hand = fixture
        .command(["serve", "--port", "0", "--no-open"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut said = String::new();
    std::io::BufReader::new(by_hand.stdout.as_mut().unwrap())
        .read_line(&mut said)
        .unwrap();
    let url = said.split(" — ").nth(1).unwrap().trim().to_string();
    (by_hand, url)
}

#[test]
fn a_board_already_running_for_the_hub_is_left_to_serve() {
    let fixture = Fixture::new(QUIET);
    let (mut by_hand, theirs) = start_board(&fixture);

    let (mut child, config) = hub_mcp(&fixture, SLUG);
    assert_eq!(config["board"]["url"], theirs.as_str());
    drop(child.stdin.take());
    child.wait().unwrap();
    // Still the hand-started one, still answering.
    assert_eq!(fixture.json(&["config"])["board"]["url"], theirs.as_str());
    assert!(fetch(&theirs).contains(" 200 "));
    by_hand.kill().unwrap();
    by_hand.wait().unwrap();
}

/// A board started while another serves the same hub — a worker checking a UI change in its
/// worktree does exactly this — must not take the record: once it stops, the record would name
/// a dead process and the board still serving would read as absent.
#[test]
fn a_second_board_does_not_take_the_record_from_the_one_still_serving() {
    let fixture = Fixture::new(QUIET);
    let (mut first, first_url) = start_board(&fixture);
    let (mut second, _) = start_board(&fixture);
    assert_eq!(
        fixture.json(&["config"])["board"]["url"],
        first_url.as_str()
    );

    second.kill().unwrap();
    second.wait().unwrap();
    assert_eq!(
        fixture.json(&["config"])["board"]["url"],
        first_url.as_str()
    );
    assert!(fetch(&first_url).contains(" 200 "));

    first.kill().unwrap();
    first.wait().unwrap();
}

#[test]
fn a_board_takes_the_record_from_one_that_has_stopped() {
    let fixture = Fixture::new(QUIET);
    let (mut first, _) = start_board(&fixture);
    first.kill().unwrap();
    first.wait().unwrap();

    let (mut second, second_url) = start_board(&fixture);
    assert_eq!(
        fixture.json(&["config"])["board"]["url"],
        second_url.as_str()
    );

    second.kill().unwrap();
    second.wait().unwrap();
}

#[test]
fn adjutant_tell_and_send_wake_heuristics_and_overrides() {
    let fixture = Fixture::new(
        r#"{"notification": "true", "workerWake": "true", "hubWake": "true",
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
                      "issueKeys": {"acme/widget": "WID"}, "ide": "code"}}}"#,
    );
    let worktree = fixture.repo.to_str().unwrap().to_string();

    // Register this process as the worker.
    let record = fixture.repo.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": std::process::id(),
            "psStarted": ps_started(std::process::id()),
        })
        .to_string(),
    )
    .unwrap();

    // Register this process as the hub too.
    let exe_name = std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let hub_record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(hub_record.parent().unwrap()).unwrap();
    std::fs::write(
        &hub_record,
        serde_json::json!({
            "pid": std::process::id(),
            "hubName": exe_name,
            "cwd": "/",
            "psStarted": ps_started(std::process::id()),
        })
        .to_string(),
    )
    .unwrap();

    let replies = mcp(
        &fixture,
        &[
            // 1. Plain notice to worker -> no wake needed.
            request(
                1,
                "tools/call",
                serde_json::json!({"name": "adjutant_tell", "arguments": {
                    "worktree": &worktree,
                    "subject": "Filing note",
                    "body": "Issue created",
                }}),
            ),
            // 2. Question to worker -> wake needed.
            request(
                2,
                "tools/call",
                serde_json::json!({"name": "adjutant_tell", "arguments": {
                    "worktree": &worktree,
                    "subject": "[question 123] which screen?",
                    "body": "need clarification",
                }}),
            ),
            // 3. Plain notice to worker with wake: true -> forced wake.
            request(
                3,
                "tools/call",
                serde_json::json!({"name": "adjutant_tell", "arguments": {
                    "worktree": &worktree,
                    "subject": "Filing note",
                    "body": "Forced wake",
                    "wake": true,
                }}),
            ),
            // 4. Question to worker with wake: false -> suppressed wake.
            request(
                4,
                "tools/call",
                serde_json::json!({"name": "adjutant_tell", "arguments": {
                    "worktree": &worktree,
                    "subject": "[question 123] which screen?",
                    "body": "Suppressed wake",
                    "wake": false,
                }}),
            ),
            // 5. Hub self-note -> no wake needed.
            request(
                5,
                "tools/call",
                serde_json::json!({"name": "adjutant_send", "arguments": {
                    "from": HUB,
                    "kind": "question",
                    "subject": "Note to self",
                    "body": "investigate later",
                }}),
            ),
            // 6. Report to hub -> wake needed.
            request(
                6,
                "tools/call",
                serde_json::json!({"name": "adjutant_send", "arguments": {
                    "from": "worker-1",
                    "kind": "report",
                    "subject": "Task done",
                    "body": "PR ready",
                }}),
            ),
            // 7. Report to hub with wake: false -> suppressed wake.
            request(
                7,
                "tools/call",
                serde_json::json!({"name": "adjutant_send", "arguments": {
                    "from": "worker-1",
                    "kind": "report",
                    "subject": "Task done",
                    "body": "PR ready without wake",
                    "wake": false,
                }}),
            ),
        ],
    );

    let res1 = tool_result(&replies[0]);
    assert_eq!(res1["present"], true);
    assert_eq!(res1["woken"], false);
    assert!(
        res1["note"]
            .as_str()
            .unwrap()
            .contains("waking was skipped because this message needs no action"),
        "{res1}"
    );

    let res2 = tool_result(&replies[1]);
    assert_eq!(res2["present"], true);
    assert_eq!(res2["woken"], true);

    let res3 = tool_result(&replies[2]);
    assert_eq!(res3["present"], true);
    assert_eq!(res3["woken"], true);

    let res4 = tool_result(&replies[3]);
    assert_eq!(res4["present"], true);
    assert_eq!(res4["woken"], false);
    assert!(
        res4["note"]
            .as_str()
            .unwrap()
            .contains("waking was skipped because this message needs no action"),
        "{res4}"
    );

    let res5 = tool_result(&replies[4]);
    assert_eq!(res5["present"], true);
    assert_eq!(res5["woken"], false);
    assert!(
        res5["note"]
            .as_str()
            .unwrap()
            .contains("waking was skipped because this message needs no action"),
        "{res5}"
    );

    let res6 = tool_result(&replies[5]);
    assert_eq!(res6["present"], true);
    assert_eq!(res6["woken"], true);

    let res7 = tool_result(&replies[6]);
    assert_eq!(res7["present"], true);
    assert_eq!(res7["woken"], false);
    assert!(
        res7["note"]
            .as_str()
            .unwrap()
            .contains("waking was skipped because this message needs no action"),
        "{res7}"
    );
}
