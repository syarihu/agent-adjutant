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
             *kill-pane*) [ -n \"$FAKE_TMUX_KILL\" ] && kill \"$FAKE_TMUX_KILL\" ;;\n\
             esac\n\
             exit 0\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o755)).unwrap();
        let panes = root.join("panes.txt");
        std::fs::write(&panes, "").unwrap();
        FakeTmux {
            bin,
            log: root.join("tmux.log"),
            panes,
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
    let kill = kill.map(|pid| pid.to_string()).unwrap_or_default();
    Resident::start_with(
        fixture,
        &[
            ("PATH", &path),
            ("FAKE_TMUX_LOG", &log),
            ("FAKE_TMUX_PANES", &panes),
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
