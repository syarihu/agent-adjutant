//! The resident server: one process serving every repository's board under a path of its own,
//! whether or not a hub is running, and the hub that finds it there.

mod common;

use common::*;

/// Stops a resident started detached, however the test ends.
struct Detached<'a>(&'a Fixture);

impl Drop for Detached<'_> {
    fn drop(&mut self) {
        let _ = self.0.cmd(&["server", "stop"]);
    }
}

fn open_verify_gate(fixture: &Fixture) -> serde_json::Value {
    let file = fixture.repo.join("gate.json");
    std::fs::write(
        &file,
        serde_json::json!({
            "kind": "verify",
            "wait": false,
            "title": "動作確認",
            "worktree": fixture.repo.to_str().unwrap(),
        })
        .to_string(),
    )
    .unwrap();
    fixture.json(&["gate", "open", "--file", file.to_str().unwrap(), "--json"])
}

#[test]
fn a_resident_serves_a_board_for_a_repository_with_no_hub() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    assert!(!resident.said.contains("token"), "{}", resident.said);

    let (status, body) = resident.get(&format!("/b/{SLUG}/api/state"));
    assert_eq!(status, 200, "{body}");
    let state: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(state["repo"], "acme/widget");
    assert_eq!(state["resident"], true);
    assert_eq!(state["hub"]["present"], false);

    // The page itself is the board's, and its calls are relative to where it was opened.
    let (status, page) = resident.get(&format!("/b/{SLUG}/"));
    assert_eq!(status, 200);
    assert!(
        page.contains("boardApi(BASE, path"),
        "the page does not use BASE"
    );
    // The views are tabs under the board's title; the sidebar no longer lists them.
    for piece in [
        "id=\"view-tabs-row\"",
        "data-tab=\"sessions\"",
        "1つずつ",
        "id=\"task-panel\"",
        "id=\"btn-hub\"",
        "data-hub-term",
        "hubterm-btn",
        // The review view is one queue: its list, the frame around the terminal's host, and the
        // switch for moving on after an answer.
        "id=\"rv-judge\"",
        "id=\"rv-term-host\"",
        "rv-group-head",
        "data-rv-refs",
        // The Issue and PR rows at the top of 詳細 and 判断, and the PR chip in a card's header.
        "id=\"tp-links\"",
        "gh-row",
        "gh-pr",
        "PR はまだありません",
        "data-rv-next",
        "処理したら次へ",
    ] {
        assert!(page.contains(piece), "{piece}");
    }
    assert!(!page.contains("id=\"nav-sessions\""));
    // The tabs run エージェント, 人, セッション.
    let tab = |name: &str| page.find(&format!("data-tab=\"{name}\"")).expect(name);
    assert!(
        tab("agent") < tab("human") && tab("human") < tab("sessions"),
        "the view tabs are not ordered agent, human, sessions"
    );
    // 要対応 is not in the sidebar: the sidebar has 「いまの仕事」 and the boards.
    assert!(!page.contains("id=\"nav-review\""));
    assert!(page.contains("id=\"nav-work\""));
    // 着手を促す is the hub panel's, not the title bar's.
    assert!(!page.contains("id=\"btn-nudge\""));

    let (status, list) = resident.get("/api/boards");
    assert_eq!(status, 200);
    let list: serde_json::Value = serde_json::from_str(&list).unwrap();
    assert_eq!(list[0]["slug"], SLUG);
    assert_eq!(list[0]["nwo"], "acme/widget");
    assert_eq!(list[0]["hubPresent"], false);
    assert!(
        list[0]["url"]
            .as_str()
            .unwrap()
            .contains(&format!("/b/{SLUG}/?token=")),
        "{list}"
    );

    // `/` and `/review` are the one page that switches between boards.
    for path in ["/", "/review"] {
        let (status, page) = resident.get(path);
        assert_eq!(status, 200, "{path}");
        assert!(page.contains("id=\"board-rows\""), "{path}");
    }
    assert_eq!(list[0]["hubId"], "hub", "{list}");
    assert_eq!(list[0]["waiting"], 0, "{list}");
    assert_eq!(list[0]["working"], 0, "{list}");
    assert_eq!(list[0]["finished"], false, "{list}");
    assert!(list[0]["hubLastAlive"].is_null(), "{list}");
    assert_eq!(list[0]["gates"], serde_json::json!([]), "{list}");

    assert_eq!(resident.get("/b/no-such-board/api/state").0, 404);
    assert_eq!(
        get(resident.port, "", &format!("/b/{SLUG}/api/state")).0,
        403
    );
    assert_eq!(
        get(resident.port, "wrong", &format!("/b/{SLUG}/api/state")).0,
        403
    );
}

#[test]
fn a_hub_s_mcp_server_does_not_start_a_second_board_while_a_resident_serves() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);

    let mut child = Reaped(
        fixture
            .command(["mcp"])
            .env("ADJUTANT_HUB_SERVE", SLUG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    writeln!(
        child.stdin.as_mut().unwrap(),
        "{}",
        request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_config", "arguments": {}}),
        )
    )
    .unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("the resident server serves the board at"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("?token="),
        "the token reached a log: {stderr}"
    );
    assert!(
        !fixture
            .state
            .join("dashboards")
            .join(format!("{SLUG}.json"))
            .exists(),
        "the hub started a board of its own"
    );

    let reply: serde_json::Value =
        serde_json::from_slice(out.stdout.split(|b| *b == b'\n').next().unwrap()).unwrap();
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    let config: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(config["board"]["resident"], true, "{config}");
    let url = config["board"]["url"].as_str().unwrap();
    assert!(
        url.starts_with(&format!("http://127.0.0.1:{}/b/{SLUG}/", resident.port)),
        "{url}"
    );
}

#[test]
fn a_gate_opened_with_only_the_resident_up_reports_server_up() {
    let fixture = Fixture::new(QUIET);
    assert_eq!(open_verify_gate(&fixture)["server"], "down");
    let _resident = Resident::start(&fixture);
    assert_eq!(open_verify_gate(&fixture)["server"], "up");
}

#[test]
fn a_parent_task_hub_s_board_is_served_at_its_own_path() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);

    // Asking where the board is is what tells the server where the repository is.
    let config = fixture.json(&["config", "--hub", FEATURE]);
    assert_eq!(config["board"]["resident"], true, "{config}");
    assert!(
        config["board"]["url"]
            .as_str()
            .unwrap()
            .contains(&format!("/b/{FEATURE_SLUG}/?token=")),
        "{config}"
    );

    let (status, body) = resident.get(&format!("/b/{FEATURE_SLUG}/api/state"));
    assert_eq!(status, 200, "{body}");
    let state: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(state["hubName"], FEATURE_HUB);
    assert_ne!(SLUG, FEATURE_SLUG);
    let (_, other) = resident.get(&format!("/b/{SLUG}/api/state"));
    let other: serde_json::Value = serde_json::from_str(&other).unwrap();
    assert_eq!(other["hubName"], HUB);

    // The tab title looks the page's own hub up in this list, by name.
    let own = |state: &serde_json::Value| {
        let found: Vec<_> = state["hubs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|h| h["name"] == state["hubName"])
            .cloned()
            .collect();
        assert_eq!(found.len(), 1, "{state}");
        found[0].clone()
    };
    let parent = own(&state);
    assert_eq!(parent["parent"], true, "{parent}");
    assert_eq!(parent["key"], FEATURE, "{parent}");
    let repository = own(&other);
    assert_eq!(repository["parent"], false, "{repository}");
    assert!(repository["key"].is_null(), "{repository}");
}

/// Writes `patch` into a task's record on disk, which is how a test puts a task in a status
/// the board's own routes would not.
fn patch_task(fixture: &Fixture, slug: &str, id: &str, patch: serde_json::Value) {
    let path = fixture
        .state
        .join("tasks")
        .join(slug)
        .join(format!("{id}.json"));
    let mut task: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for (key, value) in patch.as_object().unwrap() {
        task[key] = value.clone();
    }
    std::fs::write(path, task.to_string()).unwrap();
}

#[test]
fn a_dedicated_board_has_no_board_list() {
    let fixture = Fixture::new(QUIET);
    let mut board = Reaped(
        fixture
            .command(["serve", "--port", "0", "--no-open"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut said = String::new();
    std::io::BufReader::new(board.stdout.as_mut().unwrap())
        .read_line(&mut said)
        .unwrap();
    let url = said.split(" — ").nth(1).unwrap().trim().to_string();
    let (host, query) = url
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap();
    let port: u16 = host.rsplit(':').next().unwrap().parse().unwrap();
    let token = query.split("token=").nth(1).unwrap();

    // The page tells a dedicated board from the resident by this 404.
    let (boards, _) = get(port, token, "/api/boards");
    let (status, page) = get(port, token, "/review");
    board.kill().unwrap();
    board.wait().unwrap();

    assert_eq!(boards, 404);
    assert_eq!(status, 200);
    assert!(page.contains("id=\"board-rows\""));
}

#[test]
fn the_board_list_counts_what_waits_and_who_works() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    write_gate_file(&fixture, SLUG, "g-open", "verify", &fixture.repo);
    let on_pr = made_task(&resident, serde_json::json!({"title": "A pull request"}));
    let working = made_task(&resident, serde_json::json!({"title": "At work"}));
    let queued = made_task(&resident, serde_json::json!({"title": "In line"}));
    patch_task(
        &fixture,
        SLUG,
        queued["id"].as_str().unwrap(),
        serde_json::json!({"status": "queued"}),
    );
    patch_task(
        &fixture,
        SLUG,
        on_pr["id"].as_str().unwrap(),
        serde_json::json!({"status": "pr", "pr": "https://example.com/pull/1"}),
    );
    patch_task(
        &fixture,
        SLUG,
        working["id"].as_str().unwrap(),
        serde_json::json!({"status": "dispatched"}),
    );

    let board = board_of(&resident, SLUG);
    // The gate has no task on the board, and the pull request has no worker to say otherwise.
    assert_eq!(board["waiting"], 2, "{board}");
    assert_eq!(board["working"], 1, "{board}");
    assert_eq!(board["queued"], 1, "{board}");
    assert_eq!(board["gates"].as_array().unwrap().len(), 1, "{board}");
    assert_eq!(board["gates"][0]["kind"], "verify", "{board}");
    assert!(board["gates"][0]["worktree"].is_string(), "{board}");
}

#[test]
fn a_parent_board_is_listed_with_its_key_and_hub_id_and_hides_when_finished() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let resident = Resident::start(&fixture);

    let board = board_of(&resident, FEATURE_SLUG);
    assert_eq!(board["hub"], FEATURE, "{board}");
    assert_eq!(board["hubId"], format!("hub-{FEATURE}"), "{board}");
    // A hub with no task yet has not been used: it stays listed, to be started.
    assert_eq!(board["finished"], false, "{board}");
    // The repository's own board is never finished.
    assert_eq!(board_of(&resident, SLUG)["finished"], false);

    // A task still to be done keeps it too, though nothing works on it yet.
    let (status, body) = resident.post(
        &format!("/b/{FEATURE_SLUG}/api/tasks"),
        &serde_json::json!({"title": "Still to do"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(board_of(&resident, FEATURE_SLUG)["finished"], false);
    patch_task(
        &fixture,
        FEATURE_SLUG,
        &id,
        serde_json::json!({"status": "done"}),
    );
    assert_eq!(board_of(&resident, FEATURE_SLUG)["finished"], true);

    // A gate left on disk keeps the board in the list.
    write_gate_file(&fixture, FEATURE_SLUG, "g-left", "question", &fixture.repo);
    let board = board_of(&resident, FEATURE_SLUG);
    assert_eq!(board["finished"], false, "{board}");
    assert_eq!(board["waiting"], 1, "{board}");
}

#[test]
fn a_stopped_hub_says_when_it_was_last_alive() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let sessions = fixture.state.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join(format!("{SLUG}.json")),
        serde_json::json!({"sessionId": "s-1", "nwo": "acme/widget"}).to_string(),
    )
    .unwrap();
    std::fs::write(
        sessions.join(format!("{SLUG}.alive")),
        serde_json::json!({"sessionId": "s-1", "lastAlive": 1_700_000_000}).to_string(),
    )
    .unwrap();
    assert_eq!(board_of(&resident, SLUG)["hubLastAlive"], 1_700_000_000);

    // A heartbeat from another session says nothing about this one.
    std::fs::write(
        sessions.join(format!("{SLUG}.alive")),
        serde_json::json!({"sessionId": "s-2", "lastAlive": 1_700_000_000}).to_string(),
    )
    .unwrap();
    assert!(board_of(&resident, SLUG)["hubLastAlive"].is_null());
}

#[test]
fn the_state_can_leave_the_sessions_out() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    write_gate_file(&fixture, SLUG, "g-open", "verify", &fixture.repo);

    let (_, full) = resident.get(&format!("/b/{SLUG}/api/state"));
    let full: serde_json::Value = serde_json::from_str(&full).unwrap();
    assert!(!full["sessions"].as_array().unwrap().is_empty(), "{full}");

    let (status, lean) = get_with_query(&resident, &format!("/b/{SLUG}/api/state"), "sessions=0");
    assert_eq!(status, 200, "{lean}");
    let lean: serde_json::Value = serde_json::from_str(&lean).unwrap();
    assert_eq!(lean["sessions"], serde_json::json!([]));
    for key in ["tasks", "gates", "workers", "hubs"] {
        assert!(lean[key].is_array(), "{key}");
    }
    assert_eq!(lean["gates"].as_array().unwrap().len(), 1, "{lean}");
}

#[test]
fn a_second_resident_is_refused_while_one_runs() {
    let fixture = Fixture::new(QUIET);
    let _resident = Resident::start(&fixture);

    let out = fixture.cmd(&[
        "server",
        "start",
        "--foreground",
        "--no-open",
        "--port",
        "0",
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("another adj server is running"),
        "{out:?}"
    );

    // Started without --foreground it is not an error: it says where the one that runs is.
    let said = fixture.ok(&["server", "start", "--no-open"]);
    assert!(said.contains("already running"), "{said}");
    assert!(said.contains(&format!("/b/{SLUG}/?token=")), "{said}");
}

#[test]
fn stopping_the_resident_leaves_no_board() {
    let fixture = Fixture::new(QUIET);
    let _stop = Detached(&fixture);

    let started = fixture.ok(&["server", "start", "--no-open", "--port", "0"]);
    assert!(
        started.contains("serving on http://127.0.0.1:"),
        "{started}"
    );
    // The person who asked is told the whole URL; the log the server writes is kept private
    // and never holds the token.
    assert!(started.contains("?token="), "{started}");
    {
        use std::os::unix::fs::PermissionsExt;
        let log = fixture.state.join("server.log");
        let mode = std::fs::metadata(&log).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // The server writes its record just before it says where it is.
        let text = wait_until("server.log to say where it serves", || {
            let text = std::fs::read_to_string(&log).unwrap();
            (text.contains("serving on"), text)
        });
        assert!(!text.contains("token"), "{text}");
    }
    let status = fixture.json(&["server", "status", "--json"]);
    assert_eq!(status["running"], true);
    assert_eq!(status["boards"][0]["slug"], SLUG, "{status}");
    assert_eq!(fixture.json(&["config"])["board"]["resident"], true);
    assert_eq!(open_verify_gate(&fixture)["server"], "up");

    let stopped = fixture.ok(&["server", "stop"]);
    assert!(stopped.contains("stopped adj server"), "{stopped}");
    assert_eq!(fixture.cmd(&["server", "status"]).status.code(), Some(1));
    assert!(fixture.json(&["config"])["board"].is_null());
    assert_eq!(open_verify_gate(&fixture)["server"], "down");

    // Stopping what is not there is not a failure.
    let again = fixture.ok(&["server", "stop"]);
    assert!(again.contains("not running"), "{again}");
}

fn alive(pid: u64) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[test]
fn restarting_the_resident_replaces_the_process_on_the_same_port() {
    let fixture = Fixture::new(QUIET);
    let _stop = Detached(&fixture);
    fixture.ok(&["server", "start", "--no-open", "--port", "0"]);
    let before = fixture.json(&["server", "status", "--json"]);
    let old = before["pid"].as_u64().unwrap();

    let said = fixture.ok(&["server", "restart"]);
    assert!(said.contains(&format!("stopped pid {old}")), "{said}");
    assert!(said.contains("restarted (pid"), "{said}");

    let after = fixture.json(&["server", "status", "--json"]);
    assert_eq!(after["running"], true, "{after}");
    assert_ne!(after["pid"].as_u64().unwrap(), old, "{after}");
    assert_eq!(after["port"], before["port"], "{after}");
    assert!(!alive(old), "pid {old} is still running");
}

#[test]
fn restarting_with_nothing_running_starts_one() {
    let fixture = Fixture::new(QUIET);
    let _stop = Detached(&fixture);

    let said = fixture.ok(&["server", "restart", "--port", "0"]);
    assert!(said.contains("was not running"), "{said}");
    assert!(said.contains("serving on http://127.0.0.1:"), "{said}");
    assert_eq!(
        fixture.json(&["server", "status", "--json"])["running"],
        true
    );
}

#[test]
fn restarting_on_port_zero_does_not_call_the_port_taken() {
    let fixture = Fixture::new(QUIET);
    let _stop = Detached(&fixture);
    fixture.ok(&["server", "start", "--no-open", "--port", "0"]);

    let said = fixture.ok(&["server", "restart", "--port", "0"]);
    assert!(!said.contains("was taken"), "{said}");
    assert_eq!(
        fixture.json(&["server", "status", "--json"])["running"],
        true
    );
}

#[test]
fn restart_port_overrides_the_one_the_server_had() {
    let fixture = Fixture::new(QUIET);
    let _stop = Detached(&fixture);
    fixture.ok(&["server", "start", "--no-open", "--port", "0"]);
    let old_port = fixture.json(&["server", "status", "--json"])["port"]
        .as_u64()
        .unwrap();

    // Free when asked about, and the server is the only one racing for it.
    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    assert_ne!(u64::from(free), old_port);
    fixture.ok(&["server", "restart", "--port", &free.to_string()]);
    let after = fixture.json(&["server", "status", "--json"]);
    assert_eq!(after["port"], free, "{after}");
}

#[test]
fn a_record_of_a_process_that_is_gone_reads_as_not_running_to_restart() {
    let fixture = Fixture::new(QUIET);
    let _stop = Detached(&fixture);
    std::fs::create_dir_all(&fixture.state).unwrap();
    // This test's own pid with a start time it never had: the record names no live process.
    std::fs::write(
        fixture.state.join("server.json"),
        serde_json::json!({
            "pid": std::process::id(),
            "psStarted": "Thu Jan  1 00:00:00 1970",
            "port": 1,
            "startedAt": "1970-01-01T00:00:00Z",
            "version": "0",
        })
        .to_string(),
    )
    .unwrap();

    let said = fixture.ok(&["server", "restart", "--port", "0"]);
    assert!(said.contains("was not running"), "{said}");
    let after = fixture.json(&["server", "status", "--json"]);
    assert_eq!(after["running"], true, "{after}");
    assert_ne!(
        after["pid"].as_u64().unwrap(),
        u64::from(std::process::id())
    );
}

#[test]
fn a_board_follows_its_address_when_the_checkout_changes() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let state_main = |resident: &Resident| {
        let (status, body) = resident.get(&format!("/b/{SLUG}/api/state"));
        assert_eq!(status, 200, "{body}");
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["main"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(state_main(&resident), fixture.repo.to_string_lossy());

    // A second checkout of the same repository, and the address book pointed at it.
    let second = fixture._dir.path().join("widget-moved");
    std::fs::create_dir_all(&second).unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "user.name", "test"],
        vec!["remote", "add", "origin", "git@github.com:acme/widget.git"],
        vec!["commit", "-q", "--allow-empty", "-m", "init"],
    ] {
        let out = Command::new("git")
            .hermetic()
            .args(&args)
            .current_dir(&second)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
    }
    let second = std::fs::canonicalize(&second).unwrap();
    std::fs::write(
        fixture.state.join("boards").join(format!("{SLUG}.json")),
        serde_json::json!({"main": second.to_str().unwrap(), "nwo": "acme/widget", "hub": null})
            .to_string(),
    )
    .unwrap();
    assert_eq!(state_main(&resident), second.to_string_lossy());

    // And a board the address book no longer has is not served from memory.
    std::fs::remove_file(fixture.state.join("boards").join(format!("{SLUG}.json"))).unwrap();
    assert_eq!(resident.get(&format!("/b/{SLUG}/api/state")).0, 404);
}
