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
    assert_eq!(replies[3]["result"]["tools"].as_array().unwrap().len(), 9);

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

#[test]
fn a_board_already_running_for_the_hub_is_left_to_serve() {
    let fixture = Fixture::new(QUIET);
    let mut by_hand = fixture
        .command(["serve", "--port", "0", "--no-open"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut said = String::new();
    std::io::BufReader::new(by_hand.stdout.as_mut().unwrap())
        .read_line(&mut said)
        .unwrap();
    let theirs = said.split(" — ").nth(1).unwrap().trim().to_string();

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
