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
    assert!(page.contains("BASE + path"), "the page does not use BASE");

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

    let (status, index) = resident.get("/");
    assert_eq!(status, 200);
    assert!(index.contains("adj ボード一覧"), "{index}");
    assert!(index.contains("hub 停止中"), "{index}");
    assert!(index.contains(&format!("/b/{SLUG}/?token=")), "{index}");

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

/// A `tmux` that writes down what it was asked, answers `list-panes` from a file, and closes
/// a pane by killing the process the test names. Nothing here reaches a real tmux server.
struct FakeTmux {
    bin: PathBuf,
    log: PathBuf,
    panes: PathBuf,
    clients: PathBuf,
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
             *list-panes*) cat \"$FAKE_TMUX_PANES\" ;;\n\
             *list-clients*) cat \"$FAKE_TMUX_CLIENTS\" ;;\n\
             *kill-pane*) [ -n \"$FAKE_TMUX_KILL\" ] && kill \"$FAKE_TMUX_KILL\" ;;\n\
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
        FakeTmux {
            bin,
            log: root.join("tmux.log"),
            panes,
            clients,
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
    std::fs::write(
        &fixture.config,
        serde_json::json!({
            "notification": "true",
            "terminal": {"preset": "tmux", "session": "adjutant-test", "socket": "scratch"},
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget"}},
        })
        .to_string(),
    )
    .unwrap();
}

fn resident_with_tmux(fixture: &Fixture, tmux: &FakeTmux, kill: Option<u32>) -> Resident {
    let path = tmux.path();
    let log = tmux.log.to_string_lossy().to_string();
    let panes = tmux.panes.to_string_lossy().to_string();
    let clients = tmux.clients.to_string_lossy().to_string();
    let kill = kill.map(|pid| pid.to_string()).unwrap_or_default();
    Resident::start_with(
        fixture,
        &[
            ("PATH", &path),
            ("FAKE_TMUX_LOG", &log),
            ("FAKE_TMUX_PANES", &panes),
            ("FAKE_TMUX_CLIENTS", &clients),
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
        body.contains("\"worktreeName\":\"look-at-the-flaky-upload-test\""),
        "{body}"
    );
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
    let parent = fixture.json(&["pending", "--json", "--hub", FEATURE]);
    assert_eq!(parent["count"], 1, "{parent}");
    assert_eq!(parent["messages"][0]["kind"], "session");
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

#[test]
fn a_session_request_with_a_bad_name_no_instruction_or_another_agent_is_refused() {
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
        (serde_json::json!({"worktreeName": "ok"}), "instruction"),
        (serde_json::json!({"instruction": 5}), "instruction"),
        (
            serde_json::json!({"instruction": "x", "agent": ["claude"]}),
            "agent",
        ),
        (
            serde_json::json!({"instruction": "x", "worktreeName": 7}),
            "worktreeName",
        ),
        (serde_json::json!({"instruction": "   "}), "instruction"),
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

fn write_gate_file(fixture: &Fixture, slug: &str, id: &str, kind: &str, worktree: &Path) {
    let dir = fixture.state.join("gates").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{id}.json")),
        serde_json::json!({
            "id": id,
            "kind": kind,
            "worktree": worktree.to_str().unwrap(),
            "title": "どちらにするか",
            "openedAt": "20260922T041233Z",
        })
        .to_string(),
    )
    .unwrap();
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
    // scratch.txt, and the `.claude` directory the worker's own records sit in.
    assert_eq!(git["uncommitted"]["untracked"], 2, "{body}");
    assert_eq!(git["uncommitted"]["insertions"], 2, "{body}");
    assert_eq!(git["uncommitted"]["deletions"], 1, "{body}");
    // No remote here, so both commits are ones nobody else has.
    assert_eq!(git["unpushed"]["count"], 2, "{body}");
    assert_eq!(git["unpushed"]["commits"][0]["subject"], "keep two lines");

    let (status, body) = resident.get(&sessions_url("/worker-nobody/git"));
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("no such session"), "{body}");
}
