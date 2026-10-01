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
    for piece in ["id=\"view-tabs-row\"", "data-tab=\"sessions\"", "1つずつ"] {
        assert!(page.contains(piece), "{piece}");
    }
    assert!(!page.contains("id=\"nav-sessions\""));

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

    let mut child = fixture
        .command(["mcp"])
        .env("ADJUTANT_HUB_SERVE", SLUG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
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
    let out = child.wait_with_output().unwrap();

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

/// A GET whose own query is `query`, with the token added after it.
fn get_with_query(resident: &Resident, path: &str, query: &str) -> (u16, String) {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", resident.port)).unwrap();
    write!(
        stream,
        "GET {path}?{query}&token={} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        resident.token
    )
    .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    let (head, body) = answer.split_once("\r\n\r\n").unwrap();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        body.to_string(),
    )
}

/// `/api/boards`, as a list of objects.
fn boards_of(resident: &Resident) -> Vec<serde_json::Value> {
    let (status, body) = resident.get("/api/boards");
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

fn board_of(resident: &Resident, slug: &str) -> serde_json::Value {
    boards_of(resident)
        .into_iter()
        .find(|b| b["slug"] == slug)
        .unwrap_or_else(|| panic!("no board {slug}"))
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
    let mut board = fixture
        .command(["serve", "--port", "0", "--no-open"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
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
        let mut text = String::new();
        for _ in 0..40 {
            text = std::fs::read_to_string(&log).unwrap();
            if text.contains("serving on") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(text.contains("serving on"), "{text}");
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

/// A `tmux` that writes down what it was asked, answers `list-panes` from a file, and closes
/// a pane by killing the process the test names. Nothing here reaches a real tmux server.
struct FakeTmux {
    bin: PathBuf,
    log: PathBuf,
    panes: PathBuf,
    clients: PathBuf,
    /// What `display-message` answers: where a window lives, as `session<TAB>group`.
    home: PathBuf,
    /// What `capture-pane` prints: the screen of every pane.
    screen: PathBuf,
}

impl FakeTmux {
    fn new(fixture: &Fixture) -> FakeTmux {
        let root = fixture._dir.path();
        let bin = root.join("fakebin");
        std::fs::create_dir_all(&bin).unwrap();
        let tmux = bin.join("tmux");
        std::fs::write(
            &tmux,
            "#!/bin/sh\n\
             echo \"$@\" >> \"$FAKE_TMUX_LOG\"\n\
             case \"$*\" in\n\
             -V) echo \"tmux 3.4\" ;;\n\
             *new-window*) [ -f \"$FAKE_TMUX_LOG.failnew\" ] && { echo \"no space for a new window\" >&2; exit 1; } ;;\n\
             *display-message*) cat \"$FAKE_TMUX_HOME\" ;;\n\
             *capture-pane*) cat \"$FAKE_TMUX_SCREEN\" ;;\n\
             *list-panes*) cat \"$FAKE_TMUX_PANES\" ;;\n\
             *list-clients*) cat \"$FAKE_TMUX_CLIENTS\" ;;\n\
             *kill-pane*|*kill-window*) [ -f \"$FAKE_TMUX_LOG.onkill\" ] && sh \"$FAKE_TMUX_LOG.onkill\"; [ -n \"$FAKE_TMUX_KILL\" ] && kill \"$FAKE_TMUX_KILL\" ;;\n\
             esac\n\
             exit 0\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o755)).unwrap();
        let panes = root.join("panes.txt");
        std::fs::write(&panes, "").unwrap();
        let clients = root.join("clients.txt");
        std::fs::write(&clients, "").unwrap();
        let home = root.join("home.txt");
        std::fs::write(&home, "").unwrap();
        let screen = root.join("screen.txt");
        std::fs::write(&screen, "").unwrap();
        FakeTmux {
            bin,
            log: root.join("tmux.log"),
            panes,
            clients,
            home,
            screen,
        }
    }

    fn path(&self) -> String {
        format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    fn logged(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

fn write_tmux_config(fixture: &Fixture) {
    write_tmux_config_with(fixture, |_| {});
}

/// The tmux config, with whatever else the test needs put into it before it is written.
fn write_tmux_config_with(fixture: &Fixture, change: impl FnOnce(&mut serde_json::Value)) {
    let mut config = serde_json::json!({
        "notification": "true",
        "terminal": {"preset": "tmux", "session": "adjutant-test", "socket": "scratch"},
        "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget"}},
    });
    change(&mut config);
    std::fs::write(&fixture.config, config.to_string()).unwrap();
}

fn resident_with_tmux(fixture: &Fixture, tmux: &FakeTmux, kill: Option<u32>) -> Resident {
    let path = tmux.path();
    let log = tmux.log.to_string_lossy().to_string();
    let panes = tmux.panes.to_string_lossy().to_string();
    let clients = tmux.clients.to_string_lossy().to_string();
    let home = tmux.home.to_string_lossy().to_string();
    let screen = tmux.screen.to_string_lossy().to_string();
    let kill = kill.map(|pid| pid.to_string()).unwrap_or_default();
    Resident::start_with(
        fixture,
        &[
            ("PATH", &path),
            ("FAKE_TMUX_LOG", &log),
            ("FAKE_TMUX_PANES", &panes),
            ("FAKE_TMUX_CLIENTS", &clients),
            ("FAKE_TMUX_HOME", &home),
            ("FAKE_TMUX_SCREEN", &screen),
            ("FAKE_TMUX_KILL", &kill),
        ],
    )
}

#[test]
fn hub_start_is_refused_without_the_tmux_preset() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);

    let state: serde_json::Value =
        serde_json::from_str(&resident.get(&format!("/b/{SLUG}/api/state")).1).unwrap();
    assert_eq!(state["hubStart"]["available"], false);

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/start"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("tmux"), "{body}");

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/nope/start"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("no such hub: nope"), "{body}");
}

#[test]
fn hub_start_is_not_a_route_on_a_hub_s_own_board() {
    let fixture = Fixture::new(QUIET);
    let mut board = fixture
        .command(["serve", "--port", "0", "--no-open"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
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

    let (status, body) = post(port, token, "/api/hubs/hub/start", "{}");
    let (state_status, state) = get(port, token, "/api/state");
    board.kill().unwrap();
    board.wait().unwrap();

    assert_eq!(status, 404, "{body}");
    assert!(body.contains("no such route"), "{body}");
    assert_eq!(state_status, 200);
    let state: serde_json::Value = serde_json::from_str(&state).unwrap();
    assert_eq!(state["resident"], false);
}

#[test]
fn hub_start_runs_adj_hub_in_a_tmux_window() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let state: serde_json::Value =
        serde_json::from_str(&resident.get(&format!("/b/{SLUG}/api/state")).1).unwrap();
    assert_eq!(state["hubStart"]["available"], true);

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/start"), "{}");
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["started"], true, "{body}");

    let log = tmux.logged();
    let window = log
        .lines()
        .find(|l| l.contains("new-window"))
        .unwrap_or_else(|| panic!("no window was opened: {log}"));
    assert!(window.contains("-L scratch"), "{window}");
    assert!(window.contains("-t =adjutant-test:"), "{window}");
    assert!(
        window.contains(&format!("-c {}", fixture.repo.display())),
        "{window}"
    );
    assert!(window.contains("-n adjutant-acme-widget"), "{window}");
    assert!(window.contains(BIN), "{window}");
    assert!(window.contains(" hub"), "{window}");
    assert!(!window.contains("--hub="), "{window}");
}

#[test]
fn hub_stop_kills_the_pane_and_clears_the_record() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);

    // A process that is nobody's child but init's, so that it is gone when it is killed
    // rather than lingering until this test reaps it.
    let out = Command::new("sh")
        .args(["-c", "sleep 300 >/dev/null 2>&1 & echo $!"])
        .output()
        .unwrap();
    let sleeper: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    struct Reap(u32);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = Command::new("kill")
                .arg(self.0.to_string())
                .stderr(Stdio::null())
                .status();
        }
    }
    let _reap = Reap(sleeper);

    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": sleeper,
            "psStarted": ps_started(sleeper),
            "hubName": HUB,
            "cwd": fixture.repo.to_str().unwrap(),
            "nameInCommand": false,
            "terminal": {"backend": "tmux", "socket": "scratch", "pane": "%3"},
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &tmux.panes,
        format!("%3\t{sleeper}\t/dev/ttys999\t@1\tadjutant-test\t1\tmain\n"),
    )
    .unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/stop"), "{}");
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["stopped"], true, "{body}");
    assert_eq!(answer["wasRunning"], true, "{body}");
    assert!(
        tmux.logged().contains("-L scratch kill-pane -t %3"),
        "{}",
        tmux.logged()
    );
    assert!(!record.exists(), "the record was left behind");
    assert_eq!(ps_started(sleeper), "", "the hub is still running");

    // Nothing is left to stop, and saying so is not an error.
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/stop"), "{}");
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["wasRunning"], false, "{body}");
}

#[test]
fn hub_stop_is_refused_for_a_record_with_no_start_time() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let out = Command::new("sh")
        .args(["-c", "sleep 300 >/dev/null 2>&1 & echo $!"])
        .output()
        .unwrap();
    let sleeper: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    struct Reap(u32);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = Command::new("kill")
                .arg(self.0.to_string())
                .stderr(Stdio::null())
                .status();
        }
    }
    let _reap = Reap(sleeper);

    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": sleeper,
            "hubName": HUB,
            "cwd": fixture.repo.to_str().unwrap(),
            "nameInCommand": false,
            "terminal": {"backend": "tmux", "socket": "scratch", "pane": "%3"},
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &tmux.panes,
        format!("%3\t{sleeper}\t/dev/ttys999\t@1\tadjutant-test\t1\tmain\n"),
    )
    .unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/stop"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("no start time"), "{body}");
    // Nothing was looked up in tmux, let alone closed, and the record is as it was.
    assert!(!tmux.logged().contains("kill-pane"), "{}", tmux.logged());
    assert!(!tmux.logged().contains("list-panes"), "{}", tmux.logged());
    assert!(record.exists());
    assert!(!ps_started(sleeper).is_empty(), "the process was killed");
}

/// A process that is nobody's child but init's, killed when the guard goes.
struct Sleeper(u32);

impl Sleeper {
    fn start() -> Sleeper {
        let out = Command::new("sh")
            .args(["-c", "sleep 300 >/dev/null 2>&1 & echo $!"])
            .output()
            .unwrap();
        Sleeper(String::from_utf8_lossy(&out.stdout).trim().parse().unwrap())
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .arg(self.0.to_string())
            .stderr(Stdio::null())
            .status();
    }
}

/// A running parent-task hub `FEATURE`, as `adj hub --hub` leaves it: its record, its board
/// address, its saved session and a task, with a pane in the fake tmux to close.
fn running_parent_hub(fixture: &Fixture, tmux: &FakeTmux, sleeper: u32) -> (PathBuf, PathBuf) {
    let record = fixture
        .state
        .join("hubs")
        .join(format!("{FEATURE_SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": sleeper,
            "psStarted": ps_started(sleeper),
            "hubName": FEATURE_HUB,
            "hub": FEATURE,
            "cwd": fixture.repo.to_str().unwrap(),
            "nameInCommand": false,
            "terminal": {"backend": "tmux", "socket": "scratch", "pane": "%3"},
        })
        .to_string(),
    )
    .unwrap();
    let board = fixture
        .state
        .join("boards")
        .join(format!("{FEATURE_SLUG}.json"));
    std::fs::create_dir_all(board.parent().unwrap()).unwrap();
    std::fs::write(
        &board,
        serde_json::json!({"main": fixture.repo.to_str().unwrap(), "nwo": "acme/widget", "hub": FEATURE})
            .to_string(),
    )
    .unwrap();
    let session = fixture
        .state
        .join("sessions")
        .join(format!("{FEATURE_SLUG}.json"));
    std::fs::create_dir_all(session.parent().unwrap()).unwrap();
    std::fs::write(
        &session,
        serde_json::json!({
            "sessionId": "0b7e6a52-0000-4000-8000-000000000001",
            "hub": FEATURE,
            "nwo": "acme/widget",
            "hubName": FEATURE_HUB,
        })
        .to_string(),
    )
    .unwrap();
    let tasks = fixture.state.join("tasks").join(FEATURE_SLUG);
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(tasks.join("task-1.json"), "{}").unwrap();
    std::fs::write(
        &tmux.panes,
        format!("%3\t{sleeper}\t/dev/ttys999\t@1\tadjutant-test\t1\tmain\n"),
    )
    .unwrap();
    (record, board)
}

fn hub_ids(resident: &Resident) -> Vec<String> {
    let state: serde_json::Value =
        serde_json::from_str(&resident.get(&format!("/b/{SLUG}/api/state")).1).unwrap();
    state["hubs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap().to_string())
        .collect()
}

/// A running repository hub, as `adj hub` leaves it: its record and a pane in the fake tmux.
fn running_repo_hub(fixture: &Fixture, tmux: &FakeTmux, sleeper: u32) -> PathBuf {
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": sleeper,
            "psStarted": ps_started(sleeper),
            "hubName": HUB,
            "cwd": fixture.repo.to_str().unwrap(),
            "nameInCommand": false,
            "terminal": {"backend": "tmux", "socket": "scratch", "pane": "%3"},
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &tmux.panes,
        format!("%3\t{sleeper}\t/dev/ttys999\t@1\tadjutant-test\t1\tmain\n"),
    )
    .unwrap();
    record
}

#[test]
fn hub_reset_stops_the_running_hub_and_starts_a_new_conversation() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::start();
    let record = running_repo_hub(&fixture, &tmux, sleeper.0);
    // The conversation the old hub had: the server must leave it where it is.
    let saved = fixture.state.join("sessions").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
    let saved_text = serde_json::json!({
        "sessionId": "0b7e6a52-0000-4000-8000-000000000002",
        "nwo": "acme/widget",
        "hubName": HUB,
    })
    .to_string();
    std::fs::write(&saved, &saved_text).unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.0));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/reset"), "{}");
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["reset"], true, "{body}");
    assert_eq!(answer["wasRunning"], true, "{body}");
    assert_eq!(answer["started"], true, "{body}");

    let log = tmux.logged();
    let kill = log
        .lines()
        .position(|l| l.contains("-L scratch kill-pane -t %3"))
        .unwrap_or_else(|| panic!("no pane was closed: {log}"));
    let open = log
        .lines()
        .position(|l| l.contains("new-window") && l.contains("--new"))
        .unwrap_or_else(|| panic!("no new conversation was opened: {log}"));
    assert!(kill < open, "{log}");
    let window = log.lines().nth(open).unwrap();
    assert!(window.contains(" hub"), "{window}");
    assert!(window.contains("--new"), "{window}");
    assert!(!window.contains("--hub="), "{window}");
    assert!(!record.exists(), "the old record was left behind");
    assert_eq!(ps_started(sleeper.0), "", "the hub is still running");
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), saved_text);
}

#[test]
fn hub_reset_of_a_stopped_hub_starts_it_fresh() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/reset"), "{}");
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["reset"], true, "{body}");
    assert_eq!(answer["wasRunning"], false, "{body}");
    assert_eq!(answer["started"], true, "{body}");
    let log = tmux.logged();
    assert!(!log.contains("kill-pane"), "{log}");
    let window = log
        .lines()
        .find(|l| l.contains("new-window"))
        .unwrap_or_else(|| panic!("no window was opened: {log}"));
    assert!(window.contains("--new"), "{window}");
}

#[test]
fn hub_reset_of_a_parent_hub_names_its_key() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::start();
    let (record, _board) = running_parent_hub(&fixture, &tmux, sleeper.0);
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.0));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub-wid-957/reset"), "{}");
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["wasRunning"], true, "{body}");
    assert_eq!(answer["started"], true, "{body}");
    let log = tmux.logged();
    assert!(log.contains("-L scratch kill-pane -t %3"), "{log}");
    let window = log
        .lines()
        .find(|l| l.contains("new-window"))
        .unwrap_or_else(|| panic!("no window was opened: {log}"));
    assert!(window.contains(&format!("--hub={FEATURE}")), "{window}");
    assert!(window.contains("--new"), "{window}");
    assert!(!record.exists(), "the old record was left behind");
    // What the hub knew is untouched.
    assert_eq!(
        std::fs::read_to_string(
            fixture
                .state
                .join("tasks")
                .join(FEATURE_SLUG)
                .join("task-1.json")
        )
        .unwrap(),
        "{}"
    );
    assert!(
        fixture
            .state
            .join("sessions")
            .join(format!("{FEATURE_SLUG}.json"))
            .exists()
    );
}

#[test]
fn hub_reset_says_so_when_the_hub_was_stopped_but_could_not_start() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::start();
    let record = running_repo_hub(&fixture, &tmux, sleeper.0);
    std::fs::write(format!("{}.failnew", tmux.log.display()), "").unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.0));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/reset"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("stopped"), "{body}");
    assert!(body.contains("could not start it again"), "{body}");
    assert!(tmux.logged().contains("kill-pane"), "{}", tmux.logged());
    assert!(!record.exists(), "the old record was left behind");
}

#[test]
fn hub_reset_is_refused_before_anything_is_stopped() {
    // Not the tmux preset: starting is refused, so nothing may be stopped for it. The fake
    // tmux is wired and the record names a tmux pane, so a missing guard would close it.
    let fixture = Fixture::new(QUIET);
    write_tmux_config_with(&fixture, |config| {
        config["terminal"] = serde_json::json!({"preset": "iterm2"});
    });
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::start();
    let record = running_repo_hub(&fixture, &tmux, sleeper.0);
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.0));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/reset"), "{}");
    assert_eq!(status, 400, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        answer["error"], "starting a hub from the board needs terminal.preset \"tmux\"",
        "{body}"
    );
    assert!(!tmux.logged().contains("kill-pane"), "{}", tmux.logged());
    assert!(record.exists());
    assert!(!ps_started(sleeper.0).is_empty(), "the process was killed");
}

#[test]
fn closing_a_parent_hub_with_no_workers_stops_it_and_takes_it_off_the_list() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::start();
    let (record, board) = running_parent_hub(&fixture, &tmux, sleeper.0);
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.0));
    assert!(hub_ids(&resident).contains(&"hub-wid-957".to_string()));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub-wid-957/close"), "{}");
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["closed"], true, "{body}");
    assert_eq!(answer["wasRunning"], true, "{body}");
    assert_eq!(answer["unread"], 0, "{body}");
    assert!(
        tmux.logged().contains("-L scratch kill-pane -t %3"),
        "{}",
        tmux.logged()
    );
    assert!(!record.exists(), "the record was left behind");
    assert!(!board.exists(), "the board address was left behind");
    assert_eq!(ps_started(sleeper.0), "", "the hub is still running");
    assert!(!hub_ids(&resident).contains(&"hub-wid-957".to_string()));
    // What the hub knew is kept, so the same key picks it up again.
    assert!(
        fixture
            .state
            .join("sessions")
            .join(format!("{FEATURE_SLUG}.json"))
            .exists()
    );
    assert!(
        fixture
            .state
            .join("tasks")
            .join(FEATURE_SLUG)
            .join("task-1.json")
            .exists()
    );
}

#[test]
fn a_parent_hub_with_a_worker_is_not_closed_from_the_board() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::start();
    let (record, board) = running_parent_hub(&fixture, &tmux, sleeper.0);

    // A worker that has ended: only its saved session in a linked worktree names the hub.
    let worktree = fixture._dir.path().join("widget-wid-957");
    let out = Command::new("git")
        .hermetic()
        .args(["worktree", "add", "-q", "-b", "wid-957"])
        .arg(&worktree)
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::create_dir_all(worktree.join(".claude")).unwrap();
    std::fs::write(
        worktree.join(".claude").join("adjutant-session.json"),
        serde_json::json!({"sessionId": "sid-1", "hub": FEATURE}).to_string(),
    )
    .unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.0));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub-wid-957/close"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("still report"), "{body}");
    assert!(!tmux.logged().contains("kill-pane"), "{}", tmux.logged());
    assert!(record.exists());
    assert!(board.exists());
    assert!(hub_ids(&resident).contains(&"hub-wid-957".to_string()));

    // Stopping is still what it always was.
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub-wid-957/stop"), "{}");
    assert_eq!(status, 200, "{body}");
    assert!(hub_ids(&resident).contains(&"hub-wid-957".to_string()));
}

#[test]
fn the_repository_hub_cannot_be_closed_from_the_board() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/close"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("hub-stop"), "{body}");
    assert!(hub_ids(&resident).contains(&"hub".to_string()));
}

#[test]
fn a_hub_whose_key_needs_percent_encoding_can_be_named_from_the_board() {
    let fixture = Fixture::new(QUIET);
    // A parent-task hub whose key has a space, a slash and letters that are not ASCII, known to
    // the board only through its record.
    let record = fixture.state.join("hubs").join("acme-widget-kt-1.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "hubName": "adjutant-acme-widget-kt-1",
            "cwd": fixture.repo.to_str().unwrap(),
            "hub": "親 キー/x",
        })
        .to_string(),
    )
    .unwrap();
    let resident = Resident::start(&fixture);

    let state: serde_json::Value =
        serde_json::from_str(&resident.get(&format!("/b/{SLUG}/api/state")).1).unwrap();
    assert!(
        state["hubs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["id"] == "hub-親 キー/x"),
        "{state}"
    );

    let id = "hub-%E8%A6%AA%20%E3%82%AD%E3%83%BC%2Fx";
    // Past the check that the hub exists: what refuses the start is the settings.
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/{id}/start"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(!body.contains("no such hub"), "{body}");
    assert!(body.contains("tmux"), "{body}");
    // And a stop of a hub that is not running is an answer, not an error.
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/{id}/stop"), "{}");
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"wasRunning\":false"), "{body}");

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub-%zz/start"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("percent-encoding"), "{body}");
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

/// Open a waiting question gate in `worktree`, with `openedAt` written as given so a test
/// does not have to sleep to get gates in order.
fn open_question_in(fixture: &Fixture, worktree: &Path, kind: &str, opened_at: &str) -> String {
    let file = fixture.repo.join("gate.json");
    std::fs::write(
        &file,
        serde_json::json!({
            "kind": kind,
            "task": "t-1",
            "title": "どちらにするか",
            "worktree": worktree.to_str().unwrap(),
        })
        .to_string(),
    )
    .unwrap();
    let opened = fixture.json(&["gate", "open", "--file", file.to_str().unwrap(), "--json"]);
    let id = opened["gate"]["id"].as_str().unwrap().to_string();
    let path = fixture
        .state
        .join("gates")
        .join(SLUG)
        .join(format!("{id}.json"));
    let mut gate: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    gate["openedAt"] = serde_json::json!(opened_at);
    std::fs::write(&path, gate.to_string()).unwrap();
    id
}

fn write_worker(worktree: &Path, started_at: &str, phase_at: Option<i64>) {
    std::fs::create_dir_all(worktree.join(".claude")).unwrap();
    let mut record = serde_json::json!({ "pid": 1, "startedAt": started_at, "phase": "implement" });
    if let Some(at) = phase_at {
        record["phaseAt"] = serde_json::json!(at);
    }
    std::fs::write(
        worktree.join(".claude").join("adjutant-worker.json"),
        record.to_string(),
    )
    .unwrap();
}

fn open_ids(resident: &Resident) -> Vec<String> {
    let (status, body) = resident.get(&format!("/b/{SLUG}/api/state"));
    assert_eq!(status, 200, "{body}");
    let state: serde_json::Value = serde_json::from_str(&body).unwrap();
    state["gates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["id"].as_str().unwrap().to_string())
        .collect()
}

fn archived_gate(fixture: &Fixture, id: &str) -> Option<serde_json::Value> {
    let path = fixture
        .state
        .join("gates")
        .join(SLUG)
        .join("answered")
        .join(format!("{id}.json"));
    Some(serde_json::from_str(&std::fs::read_to_string(path).ok()?).unwrap())
}

// 2026-09-22T05:00:00Z, well after the gates below are opened, and its stamp.
const LATER_SECS: i64 = 1_790_053_200;
const LATER_STAMP: &str = "20260922T050000Z";

#[test]
fn a_gate_whose_worker_moved_to_a_later_phase_is_closed_as_answered_in_the_terminal() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let worktree = fixture.repo.clone();
    write_worker(&worktree, "20260922T040000Z", Some(LATER_SECS));
    let id = open_question_in(&fixture, &worktree, "question", "20260922T041233Z");

    assert!(!open_ids(&resident).contains(&id));
    let gate = archived_gate(&fixture, &id).expect("archived");
    assert_eq!(gate["decision"], "terminal", "{gate}");
    assert_eq!(gate["answeredAt"], LATER_STAMP, "{gate}");
    assert!(
        gate["comment"].as_str().unwrap().contains("implement"),
        "{gate}"
    );
}

#[test]
fn a_gate_stays_open_unless_the_same_worker_visibly_moved_on() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let worktree = fixture.repo.clone();

    // A worker started after the gate was opened is not the one that opened it.
    write_worker(&worktree, "20260922T045000Z", Some(LATER_SECS));
    let late_worker = open_question_in(&fixture, &worktree, "question", "20260922T041233Z");
    assert!(open_ids(&resident).contains(&late_worker));

    // Nothing happened after it was opened.
    write_worker(&worktree, "20260922T040000Z", None);
    assert!(open_ids(&resident).contains(&late_worker));
    std::fs::remove_file(
        fixture
            .state
            .join("gates")
            .join(SLUG)
            .join(format!("{late_worker}.json")),
    )
    .unwrap();

    // The hub's gates are never swept, whatever the worktree's worker did.
    write_worker(&worktree, "20260922T040000Z", Some(LATER_SECS));
    let dispatch = open_question_in(&fixture, &worktree, "dispatch", "20260922T041233Z");
    assert!(open_ids(&resident).contains(&dispatch));
    assert!(archived_gate(&fixture, &dispatch).is_none());

    // No worker record at all.
    std::fs::remove_file(worktree.join(".claude").join("adjutant-worker.json")).unwrap();
    let orphan = open_question_in(&fixture, &worktree, "question", "20260922T041233Z");
    assert!(open_ids(&resident).contains(&orphan));
}

#[test]
fn opening_a_later_gate_closes_the_earlier_one_as_answered_in_the_terminal() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let worktree = fixture.repo.clone();
    write_worker(&worktree, "20260922T040000Z", None);
    let first = open_question_in(&fixture, &worktree, "question", "20260922T041233Z");
    let second = open_question_in(&fixture, &worktree, "question", "20260922T042000Z");

    let open = open_ids(&resident);
    assert!(!open.contains(&first), "{open:?}");
    assert!(open.contains(&second), "{open:?}");
    let gate = archived_gate(&fixture, &first).expect("archived");
    assert_eq!(gate["decision"], "terminal", "{gate}");
    assert_eq!(gate["answeredAt"], "20260922T042000Z", "{gate}");
    assert!(
        gate["comment"].as_str().unwrap().contains(&second),
        "{gate}"
    );
}

// ── sessions the board starts with no task, and links to one afterwards ──

fn sessions_url(path: &str) -> String {
    format!("/b/{SLUG}/api/sessions{path}")
}

fn state_of(resident: &Resident) -> serde_json::Value {
    let (status, body) = resident.get(&format!("/b/{SLUG}/api/state"));
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

/// A parent-task hub `FEATURE` known to the board only through its record, which is all a
/// session request or a link needs of it.
fn listed_parent_hub(fixture: &Fixture) {
    let record = fixture
        .state
        .join("hubs")
        .join(format!("{FEATURE_SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "hubName": FEATURE_HUB,
            "cwd": fixture.repo.to_str().unwrap(),
            "hub": FEATURE,
        })
        .to_string(),
    )
    .unwrap();
}

/// A linked worktree next to the repository, with the worker record and saved session a worker
/// that has started leaves in it. `pid` is the process it names: 1 for one nobody could take for
/// running, a `Sleeper` for one that is.
fn session_worktree(
    fixture: &Fixture,
    name: &str,
    hub: Option<&str>,
    task: Option<&str>,
    pid: u32,
) -> PathBuf {
    let worktree = fixture.repo.parent().unwrap().join(name);
    let out = Command::new("git")
        .hermetic()
        .args(["worktree", "add", "-q", "-b", name])
        .arg(&worktree)
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::create_dir_all(worktree.join(".claude")).unwrap();
    let mut record = serde_json::json!({
        "pid": pid,
        "psStarted": ps_started(pid),
        "title": name,
        "startedAt": "20260922T040000Z",
    });
    let mut session = serde_json::json!({"sessionId": "sid-1", "title": name});
    for (key, value) in [("hub", hub), ("task", task)] {
        if let Some(value) = value {
            record[key] = serde_json::json!(value);
            session[key] = serde_json::json!(value);
        }
    }
    std::fs::write(
        worktree.join(".claude").join("adjutant-worker.json"),
        record.to_string(),
    )
    .unwrap();
    std::fs::write(
        worktree.join(".claude").join("adjutant-session.json"),
        session.to_string(),
    )
    .unwrap();
    worktree
}

fn worker_record(worktree: &Path) -> serde_json::Value {
    let text =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-worker.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn saved_session(worktree: &Path) -> serde_json::Value {
    let text =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-session.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// A task made through the board, as its record.
fn made_task(resident: &Resident, fields: serde_json::Value) -> serde_json::Value {
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/tasks"), &fields.to_string());
    assert_eq!(status, 200, "{body}");
    serde_json::from_str::<serde_json::Value>(&body).unwrap()["task"].clone()
}

fn children_of(state: &serde_json::Value, hub: &str) -> u64 {
    state["hubs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["id"] == hub)
        .unwrap_or_else(|| panic!("no hub {hub} in {state}"))["children"]
        .as_u64()
        .unwrap()
}

#[test]
fn the_state_names_the_command_a_hub_runs() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);

    // Unset, it is the built-in line, with its placeholders still in it.
    let runner = state_of(&resident)["hubRunner"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        runner.contains("{name}") && runner.contains("{sessionId}"),
        "{runner}"
    );
    drop(resident);

    // Set, it is what was written, and the `{name}` is left for the page to fill in.
    write_tmux_config_with(&fixture, |c| {
        c["hubRunner"] = serde_json::json!("my-agent {name}");
    });
    let resident = Resident::start(&fixture);
    assert_eq!(state_of(&resident)["hubRunner"], "my-agent {name}");
}

#[test]
fn a_parent_hubs_board_gives_the_sidebar_its_tasks_and_history() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let resident = Resident::start(&fixture);

    let (status, body) = resident.post(
        &format!("/b/{FEATURE_SLUG}/api/tasks"),
        &serde_json::json!({"title": "A child of the feature"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The task is on that hub's board and not on the repository's.
    let listed = |path: &str| -> Vec<String> {
        let (status, body) = resident.get(path);
        assert_eq!(status, 200, "{body}");
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(listed(&format!("/b/{FEATURE_SLUG}/api/state")).contains(&id));
    assert!(!listed(&format!("/b/{SLUG}/api/state")).contains(&id));

    // Its history is read from the same board, which is where the page asks for it.
    let (status, body) = resident.get(&format!("/b/{FEATURE_SLUG}/api/tasks/{id}/history"));
    assert_eq!(status, 200, "{body}");
    let history: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(history["answered"].is_array(), "{history}");
    assert!(history["records"].is_array(), "{history}");
}

#[test]
fn the_polled_state_carries_no_diffs_and_the_history_still_does() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(
        &format!("/b/{SLUG}/api/tasks"),
        &serde_json::json!({"title": "Has a diff"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let diff = "diff --git a/f b/f\n+one\n";
    let records = fixture.state.join("gates").join(SLUG).join("records");
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(
        records.join("20260922T041233Z-diff-record.json"),
        serde_json::json!({
            "id": "20260922T041233Z-diff-record",
            "kind": "diff",
            "task": id,
            "worktree": fixture.repo.to_str().unwrap(),
            "title": "round one",
            "wait": false,
            "openedAt": "20260922T041233Z",
            "diff": diff,
        })
        .to_string(),
    )
    .unwrap();

    let state = state_of(&resident);
    let task = state["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap();
    let record = &task["records"][0];
    assert_eq!(record["id"], "20260922T041233Z-diff-record", "{task}");
    assert!(record.get("diff").is_none(), "{record}");
    assert_eq!(record["diffSize"], diff.len(), "{record}");

    let (status, body) = resident.get(&format!("/b/{SLUG}/api/tasks/{id}/history"));
    assert_eq!(status, 200, "{body}");
    let history: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(history["records"][0]["diff"], diff, "{history}");
}

#[test]
fn a_session_request_lands_in_the_chosen_hubs_inbox_with_its_instruction() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let instruction = "Try a retry on the upload.\n## Not a header\nkeep 'quotes' and $vars";
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": instruction, "worktreeName": "try-retry"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["worktreeName"], "try-retry");
    assert_eq!(answer["hubStarted"], false, "{body}");
    assert_eq!(answer["handed"]["present"], false, "{body}");

    let pending = fixture.json(&["pending", "--json"]);
    assert_eq!(pending["count"], 1, "{pending}");
    let message = &pending["messages"][0];
    assert_eq!(message["kind"], "session");
    assert_eq!(message["from"], "dashboard");
    assert_eq!(message["subject"], "start a session: try-retry");
    let read = fixture.ok(&["pending", "--read", message["name"].as_str().unwrap()]);
    assert!(read.contains("## Worktree name try-retry"), "{read}");
    assert!(read.contains("## Agent         claude"), "{read}");
    assert!(read.contains(instruction), "{read}");

    // Left out, the name comes from the instruction.
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "Look at the flaky upload test"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains("\"worktreeName\":\"look-at-the-flaky\""),
        "{body}"
    );

    // The reply says which hub took it and under what file name the hub will find it.
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["hub"], "hub", "{body}");
    let inbox = state_of(&resident)["hubs"][0]["inbox"].clone();
    let names: Vec<&str> = inbox
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&answer["message"].as_str().unwrap()),
        "{names:?} {body}"
    );
}

#[test]
fn a_session_request_with_no_instruction_gets_a_dated_name_and_says_so() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(&sessions_url(""), "{}");
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    let name = answer["worktreeName"].as_str().unwrap();
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let rest = name.strip_prefix("session-").expect(name);
    let (day, time) = rest.split_once('-').expect(name);
    assert!(digits(day, 8) && digits(time, 4), "{name}");

    let pending = fixture.json(&["pending", "--json"]);
    let read = fixture.ok(&[
        "pending",
        "--read",
        pending["messages"][0]["name"].as_str().unwrap(),
    ]);
    assert!(read.contains("## Instruction\n-\n"), "{read}");
}

#[test]
fn a_session_started_from_the_board_names_the_agent_its_runner_is_set_to() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    assert_eq!(state_of(&resident)["sessionStart"]["agent"], "claude");
    drop(resident);

    write_tmux_config_with(&fixture, |c| {
        c["agentRunner"] = serde_json::json!("codex exec {prompt}");
    });
    let resident = Resident::start(&fixture);
    assert_eq!(state_of(&resident)["sessionStart"]["agent"], "codex");
}

#[test]
fn a_session_request_for_a_parent_hub_goes_to_that_hubs_inbox() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "Sketch it", "hub": "hub-wid-957"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["hub"], "hub-wid-957", "{body}");
    let parent = fixture.json(&["pending", "--json", "--hub", FEATURE]);
    assert_eq!(parent["count"], 1, "{parent}");
    assert_eq!(parent["messages"][0]["kind"], "session");
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

#[test]
fn a_session_request_with_a_bad_name_another_agent_or_a_non_text_instruction_is_refused() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    for (input, wanted) in [
        (
            serde_json::json!({"instruction": "x", "worktreeName": "bad name"}),
            "worktree name",
        ),
        (
            serde_json::json!({"instruction": "x", "worktreeName": "-flag"}),
            "branch",
        ),
        (serde_json::json!({"instruction": 5}), "instruction"),
        (serde_json::json!({"instruction": " - "}), "no instruction"),
        (
            serde_json::json!({"instruction": "x", "agent": ["claude"]}),
            "agent",
        ),
        (
            serde_json::json!({"instruction": "x", "worktreeName": 7}),
            "worktreeName",
        ),
        (
            serde_json::json!({"instruction": "x", "agent": "codex"}),
            "only claude",
        ),
        (
            serde_json::json!({"instruction": "x", "hub": "hub-nope"}),
            "no such hub",
        ),
    ] {
        let (status, body) = resident.post(&sessions_url(""), &input.to_string());
        assert_eq!(status, 400, "{input}: {body}");
        assert!(body.contains(wanted), "{input}: {body}");
    }
    // The agent the runner starts is fine to name.
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "x", "agent": "claude"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 1);
}

#[test]
fn a_session_request_is_refused_when_no_worker_slot_is_free() {
    let fixture = Fixture::new(
        r#"{"notification": "true", "defaults": {"ide": "code", "maxWorkers": 1},
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget"}}}"#,
    );
    let running = Sleeper::start();
    session_worktree(&fixture, "busy", None, None, running.0);
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "x"}).to_string(),
    );
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("worker limit"), "{body}");
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

#[test]
fn linking_a_taskless_session_to_a_task_gives_it_a_card() {
    let fixture = Fixture::new(QUIET);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.0);
    let resident = Resident::start(&fixture);
    let task = made_task(&resident, serde_json::json!({"title": "Retry the upload"}));
    let id = task["id"].as_str().unwrap();

    // Before: the session is on the board with no task.
    let before = state_of(&resident);
    let session = before["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "worker-try-retry")
        .unwrap();
    assert!(session["task"].is_null(), "{session}");
    assert!(session["taskTitle"].is_null(), "{session}");

    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({"task": id}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["task"]["status"], "dispatched", "{body}");
    assert_eq!(
        answer["task"]["worktree"],
        worktree.to_string_lossy().as_ref(),
        "{body}"
    );

    let record = worker_record(&worktree);
    assert_eq!(record["task"], id);
    assert_eq!(record["phase"], "implement");
    assert!(record.get("hub").is_none(), "{record}");
    // What says it is the same worker is untouched.
    assert_eq!(record["pid"], running.0);
    assert_eq!(saved_session(&worktree)["task"], id);

    let after = state_of(&resident);
    let session = after["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "worker-try-retry")
        .unwrap();
    assert_eq!(session["task"], id, "{session}");
    assert_eq!(session["taskTitle"], "Retry the upload", "{session}");
    let worker = after["workers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "try-retry")
        .unwrap();
    assert_eq!(worker["task"], id, "{worker}");
    let stored = after["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap();
    assert_eq!(stored["status"], "dispatched");

    let outbox =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-outbox.md")).unwrap();
    assert!(outbox.contains(&format!("[linked {id}]")), "{outbox}");
    assert!(outbox.contains("adj skill adj-worker"), "{outbox}");
}

#[test]
fn linking_a_new_task_on_a_parent_hubs_board_moves_the_worker_to_that_hub() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.0);
    let resident = Resident::start(&fixture);
    assert_eq!(children_of(&state_of(&resident), "hub-wid-957"), 0);

    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({"newTask": {"title": "Child of the feature"}, "hub": "hub-wid-957"})
            .to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = answer["task"]["id"].as_str().unwrap();
    assert_eq!(answer["task"]["status"], "dispatched");

    // Stored in that hub's task directory, and not the repository's.
    let there = fixture
        .state
        .join("tasks")
        .join(FEATURE_SLUG)
        .join(format!("{id}.json"));
    assert!(there.exists(), "{}", there.display());
    assert!(
        !fixture
            .state
            .join("tasks")
            .join(SLUG)
            .join(format!("{id}.json"))
            .exists()
    );
    assert_eq!(worker_record(&worktree)["hub"], FEATURE);
    assert_eq!(worker_record(&worktree)["task"], id);
    assert_eq!(saved_session(&worktree)["hub"], FEATURE);
    let state = state_of(&resident);
    assert_eq!(children_of(&state, "hub-wid-957"), 1);
    let session = state["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "worker-try-retry")
        .unwrap();
    assert_eq!(session["hub"], "hub-wid-957", "{session}");
    // The task is on the parent hub's board, not this one's, and still names the session.
    assert_eq!(session["taskTitle"], "Child of the feature", "{session}");
}

/// The messages of `kind` waiting for the hub at `hub_args` (`[]` for the repository's).
fn messages_of_kind(fixture: &Fixture, hub: &[&str], kind: &str) -> Vec<serde_json::Value> {
    let mut args = vec!["pending", "--json"];
    args.extend_from_slice(hub);
    fixture.json(&args)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] == kind)
        .cloned()
        .collect()
}

/// Makes the fake tmux refuse to open a window, which is how a hub fails to start.
fn tmux_refuses_windows(tmux: &FakeTmux) {
    std::fs::write(format!("{}.failnew", tmux.log.display()), "").unwrap();
}

#[test]
fn a_session_request_starts_a_stopped_hub_after_the_message_is_in_its_inbox() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let resident = resident_with_tmux(&fixture, &tmux, None);
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "Look around"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["hubStarted"], true, "{body}");
    assert!(answer.get("hubStartError").is_none(), "{body}");
    assert!(tmux.logged().contains("new-window"), "{}", tmux.logged());
    assert_eq!(messages_of_kind(&fixture, &[], "session").len(), 1);
}

#[test]
fn a_hub_that_cannot_start_leaves_the_session_request_queued_and_says_why() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    tmux_refuses_windows(&tmux);
    let resident = resident_with_tmux(&fixture, &tmux, None);
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "Look around"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["hubStarted"], false, "{body}");
    assert!(
        answer["hubStartError"]
            .as_str()
            .is_some_and(|e| !e.is_empty()),
        "{body}"
    );
    assert!(answer["message"].is_string(), "{body}");
    assert_eq!(messages_of_kind(&fixture, &[], "session").len(), 1);
}

#[test]
fn a_file_issue_request_starts_a_stopped_target_hub_and_reports_when_it_cannot() {
    for refuse in [false, true] {
        let fixture = Fixture::new(QUIET);
        write_tmux_config(&fixture);
        listed_parent_hub(&fixture);
        let tmux = FakeTmux::new(&fixture);
        if refuse {
            tmux_refuses_windows(&tmux);
        }
        let running = Sleeper::start();
        let worktree = session_worktree(&fixture, "try-retry", None, None, running.0);
        let resident = resident_with_tmux(&fixture, &tmux, None);
        let (status, body) = resident.post(
            &sessions_url("/worker-try-retry/link"),
            &serde_json::json!({
                "newTask": {"title": "Needs an issue", "kind": "file-and-start"},
                "hub": "hub-wid-957",
            })
            .to_string(),
        );
        assert_eq!(status, 200, "{body}");
        let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
        // The link stands either way; only what the hub was told differs.
        assert_eq!(worker_record(&worktree)["hub"], FEATURE);
        assert_eq!(
            messages_of_kind(&fixture, &["--hub", FEATURE], "file-issue").len(),
            1
        );
        assert_eq!(answer["fileIssue"]["handed"]["present"], false, "{body}");
        if refuse {
            assert_eq!(answer["hubStarted"], false, "{body}");
            assert!(answer["hubStartError"].is_string(), "{body}");
        } else {
            assert_eq!(answer["hubStarted"], true, "{body}");
            assert!(answer.get("hubStartError").is_none(), "{body}");
        }
    }
}

#[test]
fn linking_a_new_task_that_needs_an_issue_asks_the_hub_to_file_it() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.0);
    let plain = session_worktree(&fixture, "plain", None, None, running.0);
    let resident = Resident::start(&fixture);

    // Without the ask, the hub hears nothing.
    let (status, body) = resident.post(
        &sessions_url("/worker-plain/link"),
        &serde_json::json!({"newTask": {"title": "No issue"}, "hub": "hub-wid-957"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    assert!(messages_of_kind(&fixture, &["--hub", FEATURE], "file-issue").is_empty());
    assert!(
        serde_json::from_str::<serde_json::Value>(&body)
            .unwrap()
            .get("fileIssue")
            .is_none()
    );
    let outbox = std::fs::read_to_string(plain.join(".claude").join("adjutant-outbox.md")).unwrap();
    assert!(outbox.contains("none will be filed"), "{outbox}");

    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({
            "newTask": {"title": "Needs an issue", "kind": "file-and-start"},
            "hub": "hub-wid-957",
        })
        .to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = answer["task"]["id"].as_str().unwrap();
    assert_eq!(answer["fileIssue"]["handed"]["present"], false, "{body}");
    assert!(answer.get("fileIssueError").is_none(), "{body}");

    let found = messages_of_kind(&fixture, &["--hub", FEATURE], "file-issue");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["from"], "dashboard");
    assert_eq!(found[0]["subject"], format!("[file {id}] Needs an issue"));
    let read = fixture.ok(&[
        "pending",
        "--hub",
        FEATURE,
        "--read",
        found[0]["name"].as_str().unwrap(),
    ]);
    assert!(read.contains(id), "{read}");
    assert!(read.contains("file and start"), "{read}");
    assert!(
        read.contains(&format!("## Worker running in {}", worktree.display())),
        "{read}"
    );
    // Nothing went to the repository's hub.
    assert!(messages_of_kind(&fixture, &[], "file-issue").is_empty());

    let outbox =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-outbox.md")).unwrap();
    assert!(outbox.contains("files the issue"), "{outbox}");
}

#[test]
fn a_hub_that_cannot_be_told_to_file_an_issue_leaves_the_link_and_tells_the_worker() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.0);
    // A file where the hub's inbox directory would be: nothing can be delivered there.
    let inbox = fixture.state.join("inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(inbox.join(FEATURE_SLUG), "").unwrap();
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({
            "newTask": {"title": "Needs an issue", "kind": "file-and-start"},
            "hub": "hub-wid-957",
        })
        .to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(answer["fileIssueError"].is_string(), "{body}");
    assert!(answer.get("fileIssue").is_none(), "{body}");
    assert_eq!(worker_record(&worktree)["hub"], FEATURE);
    let outbox =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-outbox.md")).unwrap();
    assert!(outbox.contains("not filed"), "{outbox}");
}

#[test]
fn linking_an_existing_file_and_start_task_files_nothing_again() {
    let fixture = Fixture::new(QUIET);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.0);
    let resident = Resident::start(&fixture);
    let task = made_task(
        &resident,
        serde_json::json!({"title": "Filed already", "kind": "file-and-start", "status": "backlog"}),
    );
    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({"task": task["id"]}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(answer.get("fileIssue").is_none(), "{body}");
    assert!(messages_of_kind(&fixture, &[], "file-issue").is_empty());
    let outbox =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-outbox.md")).unwrap();
    assert!(!outbox.contains("files the issue"), "{outbox}");
}

#[test]
fn linking_an_existing_task_is_refused_when_asked_to_file_an_issue_for_it() {
    let fixture = Fixture::new(QUIET);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.0);
    let resident = Resident::start(&fixture);
    let task = made_task(&resident, serde_json::json!({"title": "Already there"}));
    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({"task": task["id"], "kind": "file-and-start"}).to_string(),
    );
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("file-and-start"), "{body}");
    assert!(worker_record(&worktree).get("task").is_none());
}

#[test]
fn linking_to_a_repository_task_moves_a_parent_hub_worker_back_so_its_hub_can_close() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "try-retry", Some(FEATURE), None, running.0);
    let resident = Resident::start(&fixture);
    assert_eq!(children_of(&state_of(&resident), "hub-wid-957"), 1);
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub-wid-957/close"), "{}");
    assert_eq!(status, 400, "{body}");

    let task = made_task(&resident, serde_json::json!({"title": "Back home"}));
    let id = task["id"].as_str().unwrap();
    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({"task": id}).to_string(),
    );
    assert_eq!(status, 200, "{body}");

    let record = worker_record(&worktree);
    assert!(record.get("hub").is_none(), "{record}");
    assert_eq!(record["task"], id);
    assert!(saved_session(&worktree).get("hub").is_none());
    assert_eq!(children_of(&state_of(&resident), "hub-wid-957"), 0);
    // Nothing reports to it any more, so the board can close it.
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub-wid-957/close"), "{}");
    assert_eq!(status, 200, "{body}");
}

#[test]
fn a_link_is_refused_for_a_hub_a_session_not_started_a_finished_task_or_one_held_by_another_worker()
{
    let fixture = Fixture::new(QUIET);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.0);
    let other = session_worktree(&fixture, "other", None, None, running.0);
    // A worktree the board lists that no worker ever registered in.
    let unstarted = fixture.repo.parent().unwrap().join("unstarted");
    let out = Command::new("git")
        .hermetic()
        .args(["worktree", "add", "-q", "-b", "unstarted"])
        .arg(&unstarted)
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    let resident = Resident::start(&fixture);
    let task = made_task(&resident, serde_json::json!({"title": "Open one"}));
    let id = task["id"].as_str().unwrap().to_string();
    let link = |session: &str, input: serde_json::Value| {
        let (status, body) = resident.post(
            &sessions_url(&format!("/{session}/link")),
            &input.to_string(),
        );
        assert_eq!(status, 400, "{input}: {body}");
        body
    };

    assert!(
        link(
            "worker-try-retry",
            serde_json::json!({"task": id, "hub": "hub-nope"})
        )
        .contains("no such hub")
    );
    assert!(link("worker-nope", serde_json::json!({"task": id})).contains("no such session"));
    assert!(link("hub", serde_json::json!({"task": id})).contains("worker session"));
    assert!(
        link("worker-unstarted", serde_json::json!({"task": id})).contains("has not started yet")
    );
    assert!(link("worker-try-retry", serde_json::json!({})).contains("required"));
    assert!(
        link(
            "worker-try-retry",
            serde_json::json!({"task": "no-such-task"})
        )
        .contains("no-such-task")
    );

    // Finished.
    let (status, body) = resident.post(
        &format!("/b/{SLUG}/api/tasks/{id}"),
        &serde_json::json!({"status": "done"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    assert!(link("worker-try-retry", serde_json::json!({"task": id})).contains("finished"));

    // Held by a worker that is running in another worktree.
    let held = made_task(&resident, serde_json::json!({"title": "Held"}));
    let held_id = held["id"].as_str().unwrap();
    let (status, body) = resident.post(
        &format!("/b/{SLUG}/api/tasks/{held_id}"),
        &serde_json::json!({"status": "dispatched", "worktree": other.to_string_lossy()})
            .to_string(),
    );
    assert_eq!(status, 200, "{body}");
    assert!(
        link("worker-try-retry", serde_json::json!({"task": held_id}))
            .contains("already has a worker")
    );

    // For Jules.
    let jules = made_task(
        &resident,
        serde_json::json!({"title": "For Jules", "executor": "jules"}),
    );
    assert!(
        link(
            "worker-try-retry",
            serde_json::json!({"task": jules["id"].as_str().unwrap()})
        )
        .contains("Jules")
    );

    // A newTask for Jules is refused as an existing one is, and leaves no record behind.
    let tasks_dir = fixture.state.join("tasks").join(SLUG);
    let count = || std::fs::read_dir(&tasks_dir).unwrap().count();
    let before = count();
    assert!(
        link(
            "worker-try-retry",
            serde_json::json!({"newTask": {"title": "Jules too", "executor": "jules"}})
        )
        .contains("Jules")
    );
    assert_eq!(count(), before);

    // An id that is not a plain file name never reaches the filesystem.
    for bad in ["../../x", "a/b", "..", "a\\b"] {
        assert!(
            link("worker-try-retry", serde_json::json!({"task": bad})).contains("no such task")
        );
    }
    assert!(!fixture.state.join("x.json").exists());
    assert!(!fixture.state.join("x.lock").exists());

    // A session whose worker has ended stays listed, and cannot be linked.
    let mut finished = Command::new("true").spawn().unwrap();
    finished.wait().unwrap();
    session_worktree(&fixture, "gone", None, None, finished.id());
    assert!(link("worker-gone", serde_json::json!({"task": id.clone()})).contains("has ended"));

    // A session that already has a task keeps it.
    std::fs::write(
        worktree.join(".claude").join("adjutant-worker.json"),
        serde_json::json!({"pid": running.0, "psStarted": ps_started(running.0), "task": "task-x"})
            .to_string(),
    )
    .unwrap();
    let free = made_task(&resident, serde_json::json!({"title": "Free"}));
    assert!(
        link(
            "worker-try-retry",
            serde_json::json!({"task": free["id"].as_str().unwrap()})
        )
        .contains("already has a task")
    );

    // Nothing was written by any of the refusals.
    let stored = state_of(&resident);
    let free_now = stored["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == free["id"])
        .unwrap();
    assert_eq!(free_now["status"], "backlog");
    assert!(free_now["worktree"].is_null(), "{free_now}");
}

// ── what the board tells sessions apart by ───────────────────────────

fn session_of<'a>(state: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    state["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap_or_else(|| panic!("no session {id} in {state}"))
}

/// Say in a worker's record where it runs, as `adj work` does when it starts one.
fn place_worker(worktree: &Path, window: &str) {
    let path = worktree.join(".claude").join("adjutant-worker.json");
    let mut record = worker_record(worktree);
    record["terminal"] =
        serde_json::json!({"backend": "tmux", "socket": "scratch", "window": window});
    std::fs::write(path, record.to_string()).unwrap();
}

#[test]
fn a_session_says_when_its_window_was_last_active_and_how_many_are_attached() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let running = Sleeper::start();
    let one = session_worktree(&fixture, "one", None, None, running.0);
    let two = session_worktree(&fixture, "two", None, None, running.0);
    let elsewhere = session_worktree(&fixture, "elsewhere", None, None, running.0);
    place_worker(&one, "@5");
    place_worker(&two, "@6");
    place_worker(&elsewhere, "@9");
    std::fs::write(
        &tmux.panes,
        "%5\t1\t/dev/ttys005\t@5\tadjutant-test\t1\tone\t1790000000\n\
         %5\t1\t/dev/ttys005\t@5\tadjboard-1-1\t1\tone\t1790000000\n\
         %6\t2\t/dev/ttys006\t@6\tadjutant-test\t2\ttwo\t1790000100\n",
    )
    .unwrap();
    // The board's own session, a client looking at @5, and iTerm2's control client, which
    // has every window of its session open.
    std::fs::write(
        &tmux.clients,
        "adjboard-1-1\t0\t@5\nadjutant-test\t0\t@5\nadjutant-test\t1\t@6\n",
    )
    .unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let state = state_of(&resident);
    let one = session_of(&state, "worker-one");
    assert_eq!(one["lastActivityAt"], 1_790_000_000, "{one}");
    assert_eq!(one["attached"], 2, "{one}");
    let two = session_of(&state, "worker-two");
    assert_eq!(two["lastActivityAt"], 1_790_000_100, "{two}");
    assert_eq!(two["attached"], 1, "{two}");
    // A window tmux does not list says nothing rather than zero.
    let gone = session_of(&state, "worker-elsewhere");
    assert!(gone.get("lastActivityAt").is_none(), "{gone}");
    assert!(gone.get("attached").is_none(), "{gone}");

    // A tmux with nothing to say about clients leaves the rest of the poll as it was.
    std::fs::write(&tmux.clients, "").unwrap();
    let quiet = state_of(&resident);
    assert_eq!(session_of(&quiet, "worker-one")["attached"], 0);
    assert_eq!(
        session_of(&quiet, "worker-one")["lastActivityAt"],
        1_790_000_000
    );
}

#[test]
fn a_session_says_what_its_pane_last_showed_only_when_asked() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    std::fs::write(
        &tmux.screen,
        include_str!("../src/fixtures/panes/claude-idle-after-turn.txt"),
    )
    .unwrap();
    let running = Sleeper::start();
    let one = session_worktree(&fixture, "one", None, None, running.0);
    place_worker(&one, "@5");
    let panes = |activity: i64| {
        std::fs::write(
            &tmux.panes,
            format!("%5\t1\t/dev/ttys005\t@5\tadjutant-test\t1\tone\t{activity}\n"),
        )
        .unwrap();
    };
    panes(1_790_000_000);
    let resident = resident_with_tmux(&fixture, &tmux, None);
    let captures = || tmux.logged().matches("capture-pane").count();
    let state_at = |query: &str| -> serde_json::Value {
        let (status, body) = get_with_query(&resident, &format!("/b/{SLUG}/api/state"), query);
        assert_eq!(status, 200, "{body}");
        serde_json::from_str(&body).unwrap()
    };

    // The ordinary poll reads no screen and says nothing of one.
    let plain = state_of(&resident);
    assert!(session_of(&plain, "worker-one").get("lastLine").is_none());
    assert_eq!(captures(), 0, "{}", tmux.logged());
    // Nor does a poll that leaves the sessions out.
    let lean = state_at("sessions=0&lines=1");
    assert_eq!(lean["sessions"], serde_json::json!([]));
    assert_eq!(captures(), 0, "{}", tmux.logged());

    // Asked for, the line above the input box.
    let asked = state_at("lines=1");
    assert_eq!(
        session_of(&asked, "worker-one")["lastLine"],
        "✻ Brewed for 3s · done 2:20",
        "{asked}"
    );
    assert_eq!(captures(), 1);
    // A window that has not moved is not read again.
    state_at("lines=1");
    assert_eq!(captures(), 1, "{}", tmux.logged());
    // How soon a window that has moved is read again depends on the clock, so that is left to
    // the unit test of `LastLines`.
}

fn write_gate_file(fixture: &Fixture, slug: &str, id: &str, kind: &str, worktree: &Path) {
    write_gate_file_with(fixture, slug, id, kind, worktree, serde_json::json!({}));
}

/// A gate file with more fields than the bare ones, merged in from `extra`.
fn write_gate_file_with(
    fixture: &Fixture,
    slug: &str,
    id: &str,
    kind: &str,
    worktree: &Path,
    extra: serde_json::Value,
) {
    let dir = fixture.state.join("gates").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    let mut gate = serde_json::json!({
        "id": id,
        "kind": kind,
        "worktree": worktree.to_str().unwrap(),
        "title": "どちらにするか",
        "openedAt": "20260922T041233Z",
    });
    for (key, value) in extra.as_object().unwrap() {
        gate[key] = value.clone();
    }
    std::fs::write(dir.join(format!("{id}.json")), gate.to_string()).unwrap();
}

#[test]
fn a_session_shows_the_gate_it_waits_on_even_from_a_parent_hubs_directory() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let parent = session_worktree(&fixture, "under-parent", Some(FEATURE), None, 1);
    let moved = session_worktree(&fixture, "moved-on", Some(FEATURE), None, 1);
    let own = session_worktree(&fixture, "own-hub", None, None, 1);
    write_gate_file(&fixture, FEATURE_SLUG, "g-parent", "question", &parent);
    write_gate_file(&fixture, FEATURE_SLUG, "g-moved", "question", &moved);
    write_gate_file(&fixture, SLUG, "g-own", "question", &own);
    write_gate_file(&fixture, SLUG, "g-dispatch", "dispatch", &fixture.repo);
    let choosing = session_worktree(&fixture, "choosing", Some(FEATURE), None, 1);
    write_gate_file_with(
        &fixture,
        FEATURE_SLUG,
        "g-choose",
        "plan",
        &choosing,
        serde_json::json!({
            "focus": "あ".repeat(500),
            "choices": [
                {"id": "a", "label": "A 案", "recommended": true},
                {"id": "b", "label": "B 案"},
            ],
        }),
    );
    // This one's worker went on to another phase after opening the gate.
    let mut record = worker_record(&moved);
    record["phaseAt"] = serde_json::json!(LATER_SECS);
    std::fs::write(
        moved.join(".claude").join("adjutant-worker.json"),
        record.to_string(),
    )
    .unwrap();
    let resident = Resident::start(&fixture);

    let state = state_of(&resident);
    let waiting = &session_of(&state, "worker-under-parent")["waiting"];
    assert_eq!(waiting["id"], "g-parent", "{waiting}");
    assert_eq!(waiting["kind"], "question");
    assert_eq!(waiting["hub"], format!("hub-{FEATURE}"));
    assert_eq!(waiting["slug"], FEATURE_SLUG);
    assert_eq!(waiting["openedAt"], "20260922T041233Z");
    assert_eq!(waiting["count"], 1);
    // A gate that names no options gets its kind's own, so the page needs no table of them.
    assert_eq!(waiting["options"], serde_json::json!(["answer"]));
    assert!(waiting.get("choices").is_none() && waiting.get("focus").is_none());
    let choosing = &session_of(&state, "worker-choosing")["waiting"];
    assert_eq!(choosing["choices"][1]["label"], "B 案", "{choosing}");
    assert_eq!(choosing["options"][0], "approve");
    let focus = choosing["focus"].as_str().unwrap();
    assert_eq!(focus.chars().count(), 401, "{focus}");
    assert!(focus.ends_with('…'));
    assert_eq!(
        session_of(&state, "worker-own-hub")["waiting"]["hub"],
        "hub"
    );
    assert!(
        session_of(&state, "worker-moved-on")
            .get("waiting")
            .is_none()
    );
    // Read only: the parent hub's own board is the one that closes its gate.
    assert!(
        fixture
            .state
            .join("gates")
            .join(FEATURE_SLUG)
            .join("g-moved.json")
            .exists()
    );
    // What the repository hub opened for a person is what the hub is waiting on.
    assert_eq!(session_of(&state, "hub")["waiting"]["id"], "g-dispatch");
}

#[test]
fn a_hub_lists_its_inbox_newest_first_with_a_cap_and_the_full_count() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let inbox = fixture.state.join("inbox").join(SLUG);
    std::fs::create_dir_all(&inbox).unwrap();
    for i in 0..23 {
        let at = format!("20260922T04{i:02}00Z");
        std::fs::write(
            inbox.join(format!("{at}-report.md")),
            format!("---\nfrom: w{i}\nkind: report\nsubject: s{i}\nat: {at}\n---\n\nbody\n"),
        )
        .unwrap();
    }
    let state = state_of(&resident);
    let hub = &state["hubs"][0];
    assert_eq!(hub["inboxCount"], 23);
    let items = hub["inbox"].as_array().unwrap();
    assert_eq!(items.len(), 20);
    assert_eq!(items[0]["subject"], "s22");
    assert_eq!(items[0]["from"], "w22");
    assert_eq!(items[0]["kind"], "report");
    assert_eq!(items[0]["at"], "20260922T042200Z");
    assert_eq!(items[19]["subject"], "s3");
}

#[test]
fn a_session_keeps_every_phase_its_worker_said() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let worktree = fixture.repo.clone();
    write_worker(&worktree, "20260922T040000Z", None);
    // Written the way a worker running a version before the history was kept left it.
    let mut record = worker_record(&worktree);
    record["phaseAt"] = serde_json::json!(1_790_000_000);
    std::fs::write(
        worktree.join(".claude").join("adjutant-worker.json"),
        record.to_string(),
    )
    .unwrap();
    for phase in ["verify", "pr"] {
        let out = fixture
            .command(["phase", "--set", phase])
            .current_dir(&worktree)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let state = state_of(&resident);
    let phases = session_of(&state, "worker-main")["phases"]
        .as_array()
        .unwrap()
        .clone();
    let names: Vec<&str> = phases.iter().map(|p| p[0].as_str().unwrap()).collect();
    assert_eq!(names, ["implement", "verify", "pr"], "{phases:?}");
    assert_eq!(phases[0][1], 1_790_000_000);
    assert_eq!(session_of(&state, "worker-main")["phase"], "pr");
}

#[test]
fn the_git_route_reports_a_dirty_worktree_and_refuses_a_session_it_does_not_know() {
    let fixture = Fixture::new(QUIET);
    let worktree = session_worktree(&fixture, "dirty", None, None, 1);
    std::fs::write(worktree.join("scratch.txt"), "untracked\n").unwrap();
    std::fs::write(worktree.join("kept.txt"), "one\ntwo\n").unwrap();
    for args in [
        vec!["add", "kept.txt"],
        vec!["commit", "-q", "-m", "keep two lines"],
    ] {
        let out = Command::new("git")
            .hermetic()
            .args(&args)
            .current_dir(&worktree)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::write(worktree.join("kept.txt"), "one\nTWO\nthree\n").unwrap();
    let resident = Resident::start(&fixture);

    let (status, body) = resident.get(&sessions_url("/worker-dirty/git"));
    assert_eq!(status, 200, "{body}");
    let git: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(git["branch"], "dirty", "{body}");
    assert_eq!(git["uncommitted"]["files"], 1, "{body}");
    // The worker's own records in `.claude` are not work.
    assert_eq!(git["uncommitted"]["untracked"], 1, "{body}");
    assert_eq!(git["uncommitted"]["insertions"], 2, "{body}");
    assert_eq!(git["uncommitted"]["deletions"], 1, "{body}");
    // No remote here, so both commits are ones nobody else has.
    assert_eq!(git["unpushed"]["count"], 2, "{body}");
    assert_eq!(git["unpushed"]["commits"][0]["subject"], "keep two lines");

    let (status, body) = resident.get(&sessions_url("/worker-nobody/git"));
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("no such session"), "{body}");
}

#[test]
fn the_git_route_looks_at_its_own_worktree_and_no_other() {
    let fixture = Fixture::new(QUIET);
    let mine = Sleeper::start();
    session_worktree(&fixture, "spy-target", None, Some("WID-7"), mine.0);
    session_worktree(&fixture, "spy-other-a", None, None, 1);
    session_worktree(&fixture, "spy-other-b", None, None, 2);
    let spy = Spy::new(fixture._dir.path());
    let resident = Resident::start_with(&fixture, &[("PATH", &spy.path())]);
    // The first request to a board has the server find its checkout, which lists the
    // worktrees once; what is counted below is the route itself.
    resident.get(&sessions_url("/worker-nobody-at-all/nothing"));
    spy.clear();

    let (status, body) = resident.get(&sessions_url("/worker-spy-target/git"));
    assert_eq!(status, 200, "{body}");

    let calls = spy.calls();
    assert!(
        calls.iter().any(|c| c.contains("spy-target")),
        "the target was never looked at: {calls:?}"
    );
    // The branch is read from the worktree listing, so no `git branch` is run for it.
    let branches = calls.iter().filter(|c| c.contains("branch --show-current"));
    assert_eq!(branches.count(), 0, "{calls:?}");
    assert!(
        !calls.iter().any(|c| c.contains("spy-other")),
        "another worktree was asked about: {calls:?}"
    );
    // Once, for the session and for the hub of its task together.
    let listings = calls.iter().filter(|c| c.contains("worktree list"));
    assert_eq!(listings.count(), 1, "{calls:?}");
    // One `ps`, for the worker the session names: the others' pids are not asked about.
    let asked: Vec<&String> = calls.iter().filter(|c| c.starts_with("ps ")).collect();
    assert!(!asked.is_empty(), "{calls:?}");
    assert!(
        asked.iter().all(|c| c.ends_with(&format!("-p {}", mine.0))),
        "{calls:?}"
    );
}

#[test]
fn a_record_written_after_a_gate_shows_its_worker_moved_on_from_a_parent_hubs_board() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let recorder = session_worktree(&fixture, "recorder", Some(FEATURE), None, 1);
    let answered = session_worktree(&fixture, "answered", Some(FEATURE), None, 1);
    write_gate_file(&fixture, FEATURE_SLUG, "g-rec", "question", &recorder);
    write_gate_file(&fixture, FEATURE_SLUG, "g-ans", "question", &answered);
    let later = |dir: &str, id: &str, wait: bool, worktree: &Path| {
        let dir = fixture.state.join("gates").join(FEATURE_SLUG).join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::json!({
                "id": id,
                "kind": "verify",
                "worktree": worktree.to_str().unwrap(),
                "title": "later",
                "wait": wait,
                "openedAt": "20260922T042000Z",
            })
            .to_string(),
        )
        .unwrap();
    };
    // A record the worker wrote without stopping, and a later gate that was answered since.
    later("records", "r-1", false, &recorder);
    later("answered", "g-next", true, &answered);
    let resident = Resident::start(&fixture);

    let state = state_of(&resident);
    assert!(
        session_of(&state, "worker-recorder")
            .get("waiting")
            .is_none()
    );
    assert!(
        session_of(&state, "worker-answered")
            .get("waiting")
            .is_none()
    );
}

// ── what the board does with a session: resume, open, clean up, start a hub ──

/// The pid of a process that has come and gone: a session whose worker ended. Pid 1 is not one
/// (it is running, and its record would match).
fn dead_pid() -> u32 {
    let mut child = Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .hermetic()
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Make the worktree look pushed: a remote-tracking ref at its HEAD, as a fetch would leave, so
/// that nothing in it counts as unpushed and no network is needed.
fn pushed(worktree: &Path, name: &str) {
    git_in(
        worktree,
        &["update-ref", &format!("refs/remotes/origin/{name}"), "HEAD"],
    );
}

fn reply_of(status_and_body: (u16, String), expected: u16) -> serde_json::Value {
    let (status, body) = status_and_body;
    assert_eq!(status, expected, "{body}");
    serde_json::from_str(&body).unwrap()
}

fn resume_config(fixture: &Fixture) {
    write_tmux_config(fixture);
    listed_parent_hub(fixture);
}

#[test]
fn resuming_is_refused_without_the_tmux_preset_for_a_running_session_a_hub_or_no_conversation() {
    let fixture = Fixture::new(QUIET);
    let ended = session_worktree(&fixture, "ended", None, None, dead_pid());
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(&sessions_url("/worker-ended/resume"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("tmux"), "{body}");

    write_tmux_config(&fixture);
    let running = Sleeper::start();
    session_worktree(&fixture, "busy", None, None, running.0);
    let (status, body) = resident.post(&sessions_url("/worker-busy/resume"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("running"), "{body}");

    let (status, body) = resident.post(&sessions_url("/hub/resume"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("only a worker"), "{body}");

    let (status, body) = resident.post(&sessions_url("/worker-nobody/resume"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("no such session"), "{body}");

    std::fs::remove_file(ended.join(".claude").join("adjutant-session.json")).unwrap();
    let (status, body) = resident.post(&sessions_url("/worker-ended/resume"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("no saved worker session"), "{body}");
}

#[test]
fn the_state_says_whether_a_session_can_be_resumed_from_the_board() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let refused = state_of(&resident);
    assert_eq!(refused["sessionResume"]["available"], false, "{refused}");
    assert!(
        refused["sessionResume"]["reason"]
            .as_str()
            .unwrap()
            .contains("tmux"),
        "{refused}"
    );

    write_tmux_config(&fixture);
    let ready = state_of(&resident);
    assert_eq!(ready["sessionResume"]["available"], true, "{ready}");
    assert!(ready["sessionResume"]["reason"].is_null(), "{ready}");

    write_tmux_config_with(&fixture, |config| {
        config["agentRunner"] = serde_json::json!("gemini {prompt}");
    });
    let other = state_of(&resident);
    assert_eq!(other["sessionResume"]["available"], false, "{other}");
    assert!(
        other["sessionResume"]["reason"]
            .as_str()
            .unwrap()
            .contains("agentResumeRunner"),
        "{other}"
    );

    // A resume runner that cannot be told the conversation would only fail when pressed.
    write_tmux_config_with(&fixture, |config| {
        config["agentResumeRunner"] = serde_json::json!("claude --continue");
    });
    let blind = state_of(&resident);
    assert_eq!(blind["sessionResume"]["available"], false, "{blind}");
    assert!(
        blind["sessionResume"]["reason"]
            .as_str()
            .unwrap()
            .contains("agentResumeRunner has no {sessionId}"),
        "{blind}"
    );
}

#[test]
fn a_gate_a_parent_hub_worker_waits_on_is_answered_through_that_hubs_board() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let worktree = session_worktree(&fixture, "under-parent", Some(FEATURE), None, 1);
    write_gate_file(&fixture, FEATURE_SLUG, "g-q", "question", &worktree);
    let resident = Resident::start(&fixture);
    let before = state_of(&resident);
    assert_eq!(
        session_of(&before, "worker-under-parent")["waiting"]["slug"],
        FEATURE_SLUG
    );

    // The board the page is on has no such gate, which is why the page posts by `waiting.slug`.
    let (status, body) = resident.post(
        &format!("/b/{SLUG}/api/gates/g-q"),
        r#"{"decision":"answer","comment":"A で"}"#,
    );
    assert_eq!(status, 400, "{body}");
    let gates = fixture.state.join("gates").join(FEATURE_SLUG);
    assert!(gates.join("g-q.json").exists());

    let (status, body) = resident.post(
        &format!("/b/{FEATURE_SLUG}/api/gates/g-q"),
        r#"{"decision":"answer","comment":"A で"}"#,
    );
    assert_eq!(status, 200, "{body}");
    assert!(!gates.join("g-q.json").exists());
    let outbox =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-outbox.md")).unwrap();
    assert!(outbox.contains("A で"), "{outbox}");
    let after = state_of(&resident);
    assert!(
        session_of(&after, "worker-under-parent")
            .get("waiting")
            .is_none(),
        "{after}"
    );
}

#[test]
fn resuming_is_refused_for_a_resume_runner_that_cannot_be_told_the_conversation() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config_with(&fixture, |config| {
        config["agentResumeRunner"] = serde_json::json!("claude --continue");
    });
    session_worktree(&fixture, "ended", None, None, dead_pid());
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(&sessions_url("/worker-ended/resume"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("sessionId"), "{body}");

    // Another agent without a resume runner of its own has nothing that reopens its conversation.
    write_tmux_config_with(&fixture, |config| {
        config["agentRunner"] = serde_json::json!("gemini {prompt}");
    });
    let (status, body) = resident.post(&sessions_url("/worker-ended/resume"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("gemini has no agentResumeRunner"), "{body}");
}

#[test]
fn resuming_opens_a_tab_that_reopens_the_worker_under_its_own_hub() {
    let fixture = Fixture::new(QUIET);
    resume_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let worktree = session_worktree(&fixture, "ended", Some(FEATURE), None, dead_pid());
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let answer = reply_of(
        resident.post(&sessions_url("/worker-ended/resume"), "{}"),
        200,
    );
    assert_eq!(answer["resumed"], true, "{answer}");
    assert_eq!(answer["hub"], format!("hub-{FEATURE}"), "{answer}");
    assert_eq!(answer["hubRunning"], false, "{answer}");

    let log = tmux.logged();
    let window = log
        .lines()
        .find(|l| l.contains("new-window"))
        .unwrap_or_else(|| panic!("no window was opened: {log}"));
    assert!(
        window.contains(&format!(
            "worker --resume --worktree {}",
            worktree.display()
        )),
        "{window}"
    );
    // Told nothing of a hub: the saved session says which one it goes back to.
    assert!(!window.contains("--hub="), "{window}");
    assert!(
        worktree
            .join(".claude")
            .join("adjutant-worker-starting.json")
            .exists(),
        "the slot was not marked"
    );

    // It is starting now, and a second click is not a second worker.
    let (status, body) = resident.post(&sessions_url("/worker-ended/resume"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("starting"), "{body}");
}

/// A hub's own board, which none of the actions a resident serves are routes on.
#[test]
fn the_session_actions_and_starting_a_hub_are_not_routes_on_a_hub_s_own_board() {
    let fixture = Fixture::new(QUIET);
    session_worktree(&fixture, "ended", None, None, dead_pid());
    let mut board = fixture
        .command(["serve", "--port", "0", "--no-open"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
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

    let answers: Vec<(u16, String)> = [
        "/api/sessions/worker-ended/resume",
        "/api/sessions/worker-ended/open",
        "/api/sessions/worker-ended/cleanup",
        "/api/hubs",
    ]
    .iter()
    .map(|path| post(port, token, path, r#"{"key":"wid-957"}"#))
    .collect();
    board.kill().unwrap();
    board.wait().unwrap();

    for (status, body) in answers {
        assert_eq!(status, 404, "{body}");
        assert!(body.contains("no such route"), "{body}");
    }
    assert!(fixture.repo.parent().unwrap().join("ended").is_dir());
}

fn attach_config(fixture: &Fixture, attach: &str) {
    write_tmux_config_with(fixture, |config| {
        config["terminal"]["attach"] = serde_json::json!(attach);
    });
}

#[test]
fn opening_a_session_makes_a_grouped_session_and_hands_it_to_the_attach_template() {
    let fixture = Fixture::new(QUIET);
    let told = fixture._dir.path().join("attach.txt");
    attach_config(
        &fixture,
        &format!(
            "echo {{socket}} {{session}} {{window}} >> {}",
            told.display()
        ),
    );
    let tmux = FakeTmux::new(&fixture);
    std::fs::write(&tmux.home, "adjutant-test\n").unwrap();
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "live", None, None, running.0);
    place_worker(&worktree, "@5");
    session_worktree(&fixture, "ended", None, None, dead_pid());
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let state = state_of(&resident);
    assert_eq!(state["sessionOpen"]["available"], true, "{state}");
    assert_eq!(
        state["sessionOpen"]["terminal"], "terminal.attach",
        "{state}"
    );

    let answer = reply_of(resident.post(&sessions_url("/worker-live/open"), "{}"), 200);
    assert_eq!(answer["opened"], true, "{answer}");
    assert_eq!(answer["window"], "@5", "{answer}");
    let name = answer["session"].as_str().unwrap();
    assert!(name.starts_with("adjterm-"), "{name}");

    let log = tmux.logged();
    assert!(
        log.contains(&format!(
            "-L scratch new-session -d -s {name} -t adjutant-test"
        )),
        "{log}"
    );
    assert!(
        log.contains(&format!("-L scratch select-window -t ={name}:@5")),
        "{log}"
    );
    assert!(
        log.contains("list-sessions"),
        "the leftovers were not swept: {log}"
    );
    assert_eq!(
        std::fs::read_to_string(&told).unwrap().trim(),
        format!("-L scratch {name} @5")
    );

    // A session that is not running has no window to show.
    let (status, body) = resident.post(&sessions_url("/worker-ended/open"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("not running in a tmux window"), "{body}");
    let (status, body) = resident.post(&sessions_url("/worker-nobody/open"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("no such session"), "{body}");
}

#[test]
fn a_session_made_for_an_attach_that_failed_is_taken_down_again() {
    let fixture = Fixture::new(QUIET);
    attach_config(&fixture, "false");
    let tmux = FakeTmux::new(&fixture);
    std::fs::write(&tmux.home, "adjutant-test\n").unwrap();
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "live", None, None, running.0);
    place_worker(&worktree, "@5");
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let (status, body) = resident.post(&sessions_url("/worker-live/open"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("terminal.attach failed"), "{body}");
    let log = tmux.logged();
    let name = log
        .split_whitespace()
        .find(|w| w.starts_with("adjterm-"))
        .unwrap_or_else(|| panic!("no session was made: {log}"));
    assert!(
        log.contains(&format!("-L scratch kill-session -t ={name}")),
        "{log}"
    );
}

#[test]
fn opening_a_session_without_an_attach_template_says_which_key_is_missing() {
    // Where iTerm2 is installed the built-in opener is the answer and there is nothing to refuse.
    if cfg!(target_os = "macos") && Path::new("/Applications/iTerm.app").exists() {
        return;
    }
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    std::fs::write(&tmux.home, "adjutant-test\n").unwrap();
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "live", None, None, running.0);
    place_worker(&worktree, "@5");
    let resident = resident_with_tmux(&fixture, &tmux, None);

    assert_eq!(
        state_of(&resident)["sessionOpen"]["available"],
        false,
        "no terminal to open it in"
    );
    let (status, body) = resident.post(&sessions_url("/worker-live/open"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("terminal.attach is not set"), "{body}");
    assert!(!tmux.logged().contains("new-session"), "{}", tmux.logged());
}

fn hook_log(fixture: &Fixture) -> PathBuf {
    fixture._dir.path().join("hooks.txt")
}

fn cleanup_config(fixture: &Fixture) {
    let log = hook_log(fixture);
    write_tmux_config_with(fixture, |config| {
        config["repos"]["acme/widget"]["onWorktreeRemove"] =
            serde_json::json!([format!("echo {{worktree}} {{name}} >> {}", log.display())]);
    });
}

fn cleanup_url(name: &str) -> String {
    sessions_url(&format!("/worker-{name}/cleanup"))
}

fn session_ids(resident: &Resident) -> Vec<String> {
    state_of(resident)["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn cleanup_lists_what_would_be_lost_and_removes_nothing() {
    let fixture = Fixture::new(QUIET);
    cleanup_config(&fixture);
    let worktree = session_worktree(&fixture, "dirty", None, None, dead_pid());
    std::fs::write(worktree.join("scratch.txt"), "untracked\n").unwrap();
    let resident = Resident::start(&fixture);

    let answer = reply_of(resident.post(&cleanup_url("dirty"), "{}"), 200);
    assert_eq!(answer["removed"], false, "{answer}");
    let kinds: Vec<&str> = answer["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap())
        .collect();
    // No remote here, so the commit it was made from is one nobody else has either.
    assert_eq!(kinds, ["untracked", "unpushed"], "{answer}");
    assert_eq!(answer["git"]["branch"], "dirty", "{answer}");
    assert!(worktree.is_dir());
    assert!(session_ids(&resident).contains(&"worker-dirty".to_string()));
    assert!(!hook_log(&fixture).exists());
}

#[test]
fn cleanup_removes_a_finished_worktree_its_local_branch_and_finishes_its_task() {
    let fixture = Fixture::new(QUIET);
    cleanup_config(&fixture);
    let resident = Resident::start(&fixture);
    // Written by a worker that has started, in a repository that does not ignore `.claude`, so
    // git sees them as untracked files of its own.
    let task = made_task(
        &resident,
        serde_json::json!({"title": "Retry the upload", "status": "dispatched", "handOver": false}),
    );
    let task_id = task["id"].as_str().unwrap();
    let worktree = session_worktree(&fixture, "done-one", None, Some(task_id), dead_pid());
    std::fs::write(worktree.join(".claude").join("task-brief.md"), "brief\n").unwrap();
    pushed(&worktree, "done-one");
    let (status, body) = resident.post(
        &format!("/b/{SLUG}/api/tasks/{task_id}"),
        &serde_json::json!({"worktree": worktree.to_str().unwrap(), "handOver": false}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    assert!(session_ids(&resident).contains(&"worker-done-one".to_string()));

    let answer = reply_of(resident.post(&cleanup_url("done-one"), "{}"), 200);
    assert_eq!(answer["removed"], true, "{answer}");
    assert_eq!(answer["forced"], false, "{answer}");
    assert_eq!(answer["closed"], false, "{answer}");
    assert_eq!(answer["branch"]["name"], "done-one", "{answer}");
    assert_eq!(answer["branch"]["deleted"], true, "{answer}");
    assert_eq!(answer["tasks"], serde_json::json!([task_id]), "{answer}");
    assert_eq!(answer["hooks"][0]["ok"], true, "{answer}");

    assert!(!worktree.exists(), "the worktree is still there");
    assert!(
        git_in(&fixture.repo, &["branch", "--list", "done-one"])
            .trim()
            .is_empty()
    );
    // The remote's side is never touched.
    git_in(
        &fixture.repo,
        &["show-ref", "--verify", "refs/remotes/origin/done-one"],
    );
    let hooks = std::fs::read_to_string(hook_log(&fixture)).unwrap();
    assert_eq!(
        hooks.trim(),
        format!("{} done-one", worktree.display()),
        "{hooks}"
    );
    let state = state_of(&resident);
    let stored = state["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == task_id)
        .unwrap();
    assert_eq!(stored["status"], "done", "{stored}");
    // The worker's record was inside the worktree, so nothing of the session is left to show.
    assert!(
        !session_ids(&resident).contains(&"worker-done-one".to_string()),
        "{state}"
    );
}

#[test]
fn cleanup_of_a_running_session_closes_it_first() {
    let fixture = Fixture::new(QUIET);
    cleanup_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "live", None, None, running.0);
    pushed(&worktree, "live");
    std::fs::write(
        &tmux.panes,
        format!(
            "%3\t{}\t/dev/ttys999\t@1\tadjutant-test\t1\tlive\n",
            running.0
        ),
    )
    .unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(running.0));

    let answer = reply_of(resident.post(&cleanup_url("live"), "{}"), 200);
    assert_eq!(answer["removed"], true, "{answer}");
    assert_eq!(answer["closed"], true, "{answer}");
    assert!(
        tmux.logged().contains("-L scratch kill-window -t @1"),
        "{}",
        tmux.logged()
    );
    assert!(!worktree.exists());
    assert_eq!(ps_started(running.0), "", "the worker is still running");
}

#[test]
fn cleanup_keeps_a_session_whose_worker_would_not_stop() {
    let fixture = Fixture::new(QUIET);
    cleanup_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "stubborn", None, None, running.0);
    pushed(&worktree, "stubborn");
    std::fs::write(
        &tmux.panes,
        format!(
            "%3\t{}\t/dev/ttys999\t@1\tadjutant-test\t1\tstubborn\n",
            running.0
        ),
    )
    .unwrap();
    // Nothing kills the process, as a terminal waiting for an answer would not.
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let (status, body) = resident.post(&cleanup_url("stubborn"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("could not be closed"), "{body}");
    assert!(worktree.is_dir());
}

#[test]
fn a_forced_cleanup_needs_the_worktree_s_name_typed_back() {
    let fixture = Fixture::new(QUIET);
    cleanup_config(&fixture);
    let worktree = session_worktree(&fixture, "risky", None, None, dead_pid());
    std::fs::write(worktree.join("scratch.txt"), "untracked\n").unwrap();
    std::fs::write(worktree.join("kept.txt"), "one\n").unwrap();
    git_in(&worktree, &["add", "kept.txt"]);
    let resident = Resident::start(&fixture);

    for body in [
        r#"{"force":true}"#,
        r#"{"force":true,"confirm":"other"}"#,
        r#"{"force":"yes","confirm":"risky"}"#,
    ] {
        let (status, answer) = resident.post(&cleanup_url("risky"), body);
        assert_eq!(status, 400, "{body}: {answer}");
        assert!(worktree.is_dir(), "{body}");
    }
    // The name alone forces nothing.
    let answer = reply_of(
        resident.post(&cleanup_url("risky"), r#"{"confirm":"risky"}"#),
        200,
    );
    assert_eq!(answer["removed"], false, "{answer}");

    let answer = reply_of(
        resident.post(&cleanup_url("risky"), r#"{"force":true,"confirm":"risky"}"#),
        200,
    );
    assert_eq!(answer["removed"], true, "{answer}");
    assert_eq!(answer["forced"], true, "{answer}");
    assert!(!worktree.exists());
    assert!(!session_ids(&resident).contains(&"worker-risky".to_string()));
}

#[test]
fn cleanup_refuses_the_main_checkout_a_hub_and_a_worktree_a_queued_task_waits_in() {
    let fixture = Fixture::new(QUIET);
    cleanup_config(&fixture);
    // The main checkout is a session of its own once a worker has been there.
    std::fs::create_dir_all(fixture.repo.join(".claude")).unwrap();
    std::fs::write(
        fixture.repo.join(".claude").join("adjutant-session.json"),
        r#"{"sessionId":"sid-main"}"#,
    )
    .unwrap();
    let waiting = session_worktree(&fixture, "waiting", None, None, dead_pid());
    pushed(&waiting, "waiting");
    let resident = Resident::start(&fixture);
    made_task(
        &resident,
        serde_json::json!({
            "title": "Wait for a slot",
            "status": "queued",
            "worktree": waiting.to_str().unwrap(),
            "handOver": false,
        }),
    );

    let (status, body) = resident.post(&sessions_url("/worker-main/cleanup"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("main checkout"), "{body}");
    let (status, body) = resident.post(&sessions_url("/hub/cleanup"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("only a worker"), "{body}");
    let (status, body) = resident.post(&sessions_url("/worker-nobody/cleanup"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("no such session"), "{body}");

    // Forcing does not get past it: the queue would start a worker in the ground removed.
    let (status, body) = resident.post(
        &cleanup_url("waiting"),
        r#"{"force":true,"confirm":"waiting"}"#,
    );
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("queued task"), "{body}");
    assert!(waiting.is_dir());
}

#[test]
fn a_parent_hub_can_be_started_for_a_key_from_the_board() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let answer = reply_of(
        resident.post(&format!("/b/{SLUG}/api/hubs"), r#"{"key":" wid-957 "}"#),
        200,
    );
    assert_eq!(answer["started"], true, "{answer}");
    assert_eq!(answer["hub"]["id"], "hub-wid-957", "{answer}");
    assert_eq!(answer["hub"]["slug"], FEATURE_SLUG, "{answer}");
    let log = tmux.logged();
    let window = log
        .lines()
        .find(|l| l.contains("new-window"))
        .unwrap_or_else(|| panic!("no window was opened: {log}"));
    assert!(window.contains("--hub=wid-957"), "{window}");

    for body in [
        "{}",
        r#"{"key":"  "}"#,
        r#"{"key":3}"#,
        r#"{"key":"a","start":"x"}"#,
    ] {
        let (status, answer) = resident.post(&format!("/b/{SLUG}/api/hubs"), body);
        assert_eq!(status, 400, "{body}: {answer}");
    }
}

#[test]
fn starting_a_parent_hub_needs_the_tmux_preset() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs"), r#"{"key":"wid-957"}"#);
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("tmux"), "{body}");
}

#[test]
fn linking_can_name_the_phase_the_session_is_in() {
    let fixture = Fixture::new(QUIET);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.0);
    let resident = Resident::start(&fixture);
    let task = made_task(&resident, serde_json::json!({"title": "Retry the upload"}));
    let id = task["id"].as_str().unwrap();

    // Refused before anything is written.
    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({"task": id, "phase": "nope"}).to_string(),
    );
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("no such phase: nope"), "{body}");
    assert!(body.contains("self-review"), "{body}");
    let stored = state_of(&resident)["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap()
        .clone();
    assert_eq!(stored["status"], "backlog", "{stored}");
    assert!(worker_record(&worktree).get("task").is_none());

    reply_of(
        resident.post(
            &sessions_url("/worker-try-retry/link"),
            &serde_json::json!({"task": id, "phase": "plan"}).to_string(),
        ),
        200,
    );
    let record = worker_record(&worktree);
    assert_eq!(record["task"], id);
    assert_eq!(record["phase"], "plan");
    let phases = record["phases"].as_array().unwrap();
    assert_eq!(phases.last().unwrap()[0], "plan", "{record}");
}

#[test]
fn cleanup_looks_again_after_closing_and_keeps_what_the_worker_committed_meanwhile() {
    let fixture = Fixture::new(QUIET);
    cleanup_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let running = Sleeper::start();
    let worktree = session_worktree(&fixture, "busy-one", None, None, running.0);
    pushed(&worktree, "busy-one");
    std::fs::write(
        &tmux.panes,
        format!(
            "%3\t{}\t/dev/ttys999\t@1\tadjutant-test\t1\tbusy\n",
            running.0
        ),
    )
    .unwrap();
    // The worker gets a last commit in as its window is closed.
    std::fs::write(
        format!("{}.onkill", tmux.log.display()),
        format!(
            "cd {} && echo x > late.txt && git add late.txt && git commit -q -m late\n",
            worktree.display()
        ),
    )
    .unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(running.0));

    let answer = reply_of(resident.post(&cleanup_url("busy-one"), "{}"), 200);
    assert_eq!(answer["removed"], false, "{answer}");
    assert_eq!(answer["closed"], true, "{answer}");
    assert_eq!(answer["reasons"][0]["kind"], "unpushed", "{answer}");
    assert!(worktree.is_dir());
    assert!(
        !git_in(&fixture.repo, &["branch", "--list", "busy-one"])
            .trim()
            .is_empty()
    );
}

#[test]
fn a_removal_that_fails_leaves_the_session_as_it_was() {
    let fixture = Fixture::new(QUIET);
    cleanup_config(&fixture);
    let worktree = session_worktree(&fixture, "locked-one", None, None, dead_pid());
    pushed(&worktree, "locked-one");
    std::fs::write(worktree.join(".claude").join("task-brief.md"), "brief\n").unwrap();
    git_in(
        &fixture.repo,
        &["worktree", "lock", worktree.to_str().unwrap()],
    );
    let resident = Resident::start(&fixture);

    let (status, body) = resident.post(&cleanup_url("locked-one"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(worktree.is_dir());
    // Moved aside for the second try and put back when it failed too. The worker's record was
    // never touched: a worker that is gone is not closed, and closing would have cleared it.
    assert_eq!(worker_record(&worktree)["title"], "locked-one");
    assert_eq!(
        std::fs::read_to_string(worktree.join(".claude").join("task-brief.md")).unwrap(),
        "brief\n"
    );
    assert_eq!(saved_session(&worktree)["sessionId"], "sid-1");
    assert!(session_ids(&resident).contains(&"worker-locked-one".to_string()));
    let left: Vec<_> = std::fs::read_dir(&fixture.state)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("cleanup-"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    // The marker that held off new workers is gone with the attempt.
    assert!(!removing_marker(&fixture, &worktree).exists());
}

/// Where the marker saying a worktree is being removed is kept.
fn removing_marker(fixture: &Fixture, worktree: &Path) -> PathBuf {
    let name: String = std::fs::canonicalize(worktree)
        .unwrap()
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    fixture
        .repo
        .join(".claude")
        .join("adjutant-removing")
        .join(format!("{name}.json"))
}

#[test]
fn nothing_starts_in_a_worktree_that_is_being_removed_or_already_starting() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let worktree = session_worktree(&fixture, "going", None, None, dead_pid());
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let marker = removing_marker(&fixture, &worktree);
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        &marker,
        serde_json::json!({"pid": std::process::id(), "at": now}).to_string(),
    )
    .unwrap();
    let (status, body) = resident.post(&sessions_url("/worker-going/resume"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("being removed"), "{body}");
    let out = fixture.cmd(&["work", "--resume", "--worktree", worktree.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("being removed"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!tmux.logged().contains("new-window"), "{}", tmux.logged());

    // A marker from a removal that died long ago holds nothing.
    std::fs::write(
        &marker,
        serde_json::json!({"pid": 1, "at": now - 3600}).to_string(),
    )
    .unwrap();
    reply_of(
        resident.post(&sessions_url("/worker-going/resume"), "{}"),
        200,
    );
    // And the worktree is now marked as starting, which turns the next start away too.
    let out = fixture.cmd(&["work", "--resume", "--worktree", worktree.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already starting"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_worktree_git_cannot_read_is_a_reason_of_its_own() {
    let fixture = Fixture::new(QUIET);
    cleanup_config(&fixture);
    let worktree = session_worktree(&fixture, "broken", None, None, dead_pid());
    std::fs::write(worktree.join(".git"), "gitdir: /nonexistent/place\n").unwrap();
    let resident = Resident::start(&fixture);

    let answer = reply_of(resident.post(&cleanup_url("broken"), "{}"), 200);
    assert_eq!(answer["removed"], false, "{answer}");
    assert_eq!(answer["reasons"][0]["kind"], "git", "{answer}");
    assert!(answer["git"].is_null(), "{answer}");
    assert!(worktree.is_dir());
}

// ── the parent task's title on a parent-task hub ──

/// A `gh` that answers every issue with the title in `gh-title`, fails once `gh-fail` exists, and
/// writes down the URL of each issue it is asked about. Returns the `PATH` to run the server
/// with and that log.
fn stub_gh_for_titles(fixture: &Fixture) -> (String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let stubs = fixture.repo.join("stub-bin");
    std::fs::create_dir_all(&stubs).unwrap();
    let asked = fixture.repo.join("gh-asked");
    let fail = fixture.repo.join("gh-fail");
    let gh = stubs.join("gh");
    std::fs::write(
        &gh,
        format!(
            "#!/bin/sh\n\
             for a; do u=$a; done\n\
             echo \"$u\" >> {asked}\n\
             [ -e {fail} ] && {{ echo 'Could not resolve to an issue' >&2; exit 1; }}\n\
             printf '{{\"title\":\"The parent task\",\"body\":\"B\"}}'\n",
            asked = shell_quoted(&asked.to_string_lossy()),
            fail = shell_quoted(&fail.to_string_lossy()),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        stubs.to_string_lossy(),
        std::env::var("PATH").unwrap_or_default()
    );
    (path, asked)
}

fn gh_asked(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// The `title` of the hub `hub-wid-957` in the state, null while it has none.
fn feature_title(resident: &Resident) -> serde_json::Value {
    state_of(resident)["hubs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["id"] == "hub-wid-957")
        .unwrap_or_else(|| panic!("no hub-wid-957"))["title"]
        .clone()
}

/// Polls until the hub has a title, or says so after a few seconds.
fn wait_for_feature_title(resident: &Resident) -> serde_json::Value {
    for _ in 0..50 {
        let title = feature_title(resident);
        if !title.is_null() {
            return title;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    serde_json::Value::Null
}

/// Polls long enough for a read that was going to start to have started and finished.
fn poll_feature_title(resident: &Resident, times: usize) {
    for _ in 0..times {
        let _ = feature_title(resident);
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

#[test]
fn a_parent_hubs_title_is_read_once_and_kept_across_a_restart() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let (path, asked) = stub_gh_for_titles(&fixture);
    {
        let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
        assert_eq!(wait_for_feature_title(&resident), "The parent task");
    }
    assert_eq!(
        gh_asked(&asked),
        ["https://github.com/acme/widget/issues/957"]
    );
    // The repository's own hub has no parent task, so no title.
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    assert_eq!(wait_for_feature_title(&resident), "The parent task");
    poll_feature_title(&resident, 5);
    assert_eq!(gh_asked(&asked).len(), 1, "{:?}", gh_asked(&asked));
    let hubs = state_of(&resident)["hubs"].clone();
    assert!(hubs[0]["title"].is_null(), "{hubs}");
}

#[test]
fn a_parent_key_no_issue_key_matches_is_never_asked_about() {
    let fixture = Fixture::new(
        r#"{"notification": "true", "defaults": {"ide": "code"},
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
                                      "issueKeys": {"acme/widget": "GAMMA"}}}}"#,
    );
    listed_parent_hub(&fixture);
    let (path, asked) = stub_gh_for_titles(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    poll_feature_title(&resident, 8);
    assert!(feature_title(&resident).is_null());
    assert!(gh_asked(&asked).is_empty(), "{:?}", gh_asked(&asked));
}

#[test]
fn the_parent_a_child_task_names_is_preferred_to_the_issue_key() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let tasks = fixture.state.join("tasks").join(FEATURE_SLUG);
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        tasks.join("child.json"),
        serde_json::json!({
            "id": "child", "kind": "investigate", "title": "Child", "doneWhen": "report-only",
            "autoStart": true, "status": "backlog", "createdAt": "20260101T000000Z",
            "updatedAt": "20260101T000000Z",
            "parent": "https://github.com/acme/elsewhere/issues/5",
        })
        .to_string(),
    )
    .unwrap();
    let (path, asked) = stub_gh_for_titles(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    assert_eq!(wait_for_feature_title(&resident), "The parent task");
    assert_eq!(
        gh_asked(&asked),
        ["https://github.com/acme/elsewhere/issues/5"]
    );
}

#[test]
fn an_issue_that_could_not_be_read_is_not_asked_for_again_on_the_next_poll() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let (path, asked) = stub_gh_for_titles(&fixture);
    std::fs::write(fixture.repo.join("gh-fail"), "").unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    // Wait for the one read to have happened, so a slow machine does not pass with none.
    for _ in 0..100 {
        if !gh_asked(&asked).is_empty() {
            break;
        }
        let _ = feature_title(&resident);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    poll_feature_title(&resident, 10);
    assert!(feature_title(&resident).is_null());
    assert_eq!(gh_asked(&asked).len(), 1, "{:?}", gh_asked(&asked));
}
