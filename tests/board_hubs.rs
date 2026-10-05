//! The hub routes of the resident's board: start, focus, stop, reset, restart and close a hub,
//! run against a fake tmux.

mod common;

use common::*;

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

/// 「タブで話す」 on a board whose hub is not running: nothing to raise, and the page is told
/// so rather than handed an error.
#[test]
fn hub_focus_on_a_board_whose_hub_is_not_running_raises_nothing() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hub/focus"), "{}");
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        answer,
        serde_json::json!({ "present": false, "ran": false })
    );
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
    let sleeper = Sleeper::new();
    let record = running_repo_hub(&fixture, &tmux, sleeper.pid());
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
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.pid()));

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
    assert_eq!(ps_started(sleeper.pid()), "", "the hub is still running");
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
    let sleeper = Sleeper::new();
    let (record, _board) = running_parent_hub(&fixture, &tmux, sleeper.pid());
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.pid()));

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
    let sleeper = Sleeper::new();
    let record = running_repo_hub(&fixture, &tmux, sleeper.pid());
    std::fs::write(format!("{}.failnew", tmux.log.display()), "").unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.pid()));

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
    let sleeper = Sleeper::new();
    let record = running_repo_hub(&fixture, &tmux, sleeper.pid());
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.pid()));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/reset"), "{}");
    assert_eq!(status, 400, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        answer["error"], "starting a hub from the board needs terminal.preset \"tmux\"",
        "{body}"
    );
    assert!(!tmux.logged().contains("kill-pane"), "{}", tmux.logged());
    assert!(record.exists());
    assert!(
        !ps_started(sleeper.pid()).is_empty(),
        "the process was killed"
    );
}

/// The conversation a running hub had, saved where `adj hub --resume` reads it.
fn saved_hub_conversation(fixture: &Fixture, slug: &str) -> (PathBuf, String) {
    let saved = fixture.state.join("sessions").join(format!("{slug}.json"));
    std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
    let text = serde_json::json!({
        "sessionId": "0b7e6a52-0000-4000-8000-000000000002",
        "nwo": "acme/widget",
        "hubName": HUB,
    })
    .to_string();
    std::fs::write(&saved, &text).unwrap();
    (saved, text)
}

#[test]
fn hub_restart_stops_the_running_hub_and_resumes_its_conversation() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::new();
    let record = running_repo_hub(&fixture, &tmux, sleeper.pid());
    let (saved, saved_text) = saved_hub_conversation(&fixture, SLUG);
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.pid()));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/restart"), "{}");
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["restarted"], true, "{body}");
    assert_eq!(answer["wasRunning"], true, "{body}");
    assert_eq!(answer["started"], true, "{body}");
    assert!(answer["description"].is_string(), "{body}");

    let log = tmux.logged();
    let kill = log
        .lines()
        .position(|l| l.contains("-L scratch kill-pane -t %3"))
        .unwrap_or_else(|| panic!("no pane was closed: {log}"));
    let open = log
        .lines()
        .position(|l| l.contains("new-window") && l.contains("--resume"))
        .unwrap_or_else(|| panic!("the conversation was not resumed: {log}"));
    assert!(kill < open, "{log}");
    let window = log.lines().nth(open).unwrap();
    assert!(window.contains(" hub --resume"), "{window}");
    assert!(!window.contains("--new"), "{window}");
    assert!(!window.contains("--hub="), "{window}");
    assert!(!record.exists(), "the old record was left behind");
    assert_eq!(ps_started(sleeper.pid()), "", "the hub is still running");
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), saved_text);
}

#[test]
fn hub_restart_of_a_parent_hub_names_its_key() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::new();
    let (record, _board) = running_parent_hub(&fixture, &tmux, sleeper.pid());
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.pid()));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub-wid-957/restart"), "{}");
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
    assert!(window.contains("--resume"), "{window}");
    assert!(!record.exists(), "the old record was left behind");
    // The conversation it resumes is still the one it had.
    assert!(
        fixture
            .state
            .join("sessions")
            .join(format!("{FEATURE_SLUG}.json"))
            .exists()
    );
}

#[test]
fn hub_restart_is_refused_before_anything_is_stopped() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::new();
    let record = running_repo_hub(&fixture, &tmux, sleeper.pid());
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.pid()));
    let restart = format!("/b/{SLUG}/api/hubs/hub/restart");
    let refused = |expected: &str| {
        let (status, body) = resident.post(&restart, "{}");
        assert_eq!(status, 400, "{body}");
        assert!(body.contains(expected), "{expected}: {body}");
        assert!(!tmux.logged().contains("kill-pane"), "{}", tmux.logged());
        assert!(record.exists());
        assert!(
            !ps_started(sleeper.pid()).is_empty(),
            "the process was killed"
        );
    };

    // Nothing saved to come back to.
    refused("no saved session to resume");

    // Everything below has a conversation, so each refusal is about the settings.
    saved_hub_conversation(&fixture, SLUG);
    write_tmux_config_with(&fixture, |config| {
        config["terminal"] = serde_json::json!({"preset": "iterm2"});
    });
    refused("needs terminal.preset");

    write_tmux_config_with(&fixture, |config| {
        config["hubResumeRunner"] = serde_json::json!("claude --continue");
    });
    refused("hubResumeRunner has no {sessionId}");

    // A runner of the person's own would be replaced by the built-in one.
    write_tmux_config_with(&fixture, |config| {
        config["hubRunner"] = serde_json::json!("my-agent {prompt}");
    });
    refused("hubResumeRunner is not set");
}

#[test]
fn hub_restart_of_a_parent_hub_with_an_unknown_key_stops_nothing() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::new();
    // A record that carries no key, under a name the key cannot be read back from.
    let record = fixture.state.join("hubs").join("oddstem.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": sleeper.pid(),
            "psStarted": ps_started(sleeper.pid()),
            "hubName": "hub-oddstem",
            "cwd": fixture.repo.to_str().unwrap(),
            "terminal": {"backend": "tmux", "socket": "scratch", "pane": "%3"},
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &tmux.panes,
        format!(
            "%3\t{}\t/dev/ttys999\t@1\tadjutant-test\t1\tmain\n",
            sleeper.pid()
        ),
    )
    .unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.pid()));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub-oddstem/restart"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("key of this hub is not known"), "{body}");
    assert!(!tmux.logged().contains("kill-pane"), "{}", tmux.logged());
    assert!(record.exists());
    assert!(
        !ps_started(sleeper.pid()).is_empty(),
        "the process was killed"
    );
}

#[test]
fn hub_restart_keeps_the_hub_that_will_not_stop() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::new();
    let record = running_repo_hub(&fixture, &tmux, sleeper.pid());
    let (saved, saved_text) = saved_hub_conversation(&fixture, SLUG);
    // Nothing kills the process, as a hub waiting for an answer would not die.
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/restart"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("is still running"), "{body}");
    assert!(
        !tmux.logged().contains("new-window"),
        "a second hub was started beside it: {}",
        tmux.logged()
    );
    assert!(
        record.exists(),
        "the record of the hub that is still running was removed"
    );
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), saved_text);
}

#[test]
fn hub_restart_says_so_when_the_hub_was_stopped_but_could_not_start() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::new();
    let record = running_repo_hub(&fixture, &tmux, sleeper.pid());
    let (saved, saved_text) = saved_hub_conversation(&fixture, SLUG);
    std::fs::write(format!("{}.failnew", tmux.log.display()), "").unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.pid()));

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub/restart"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("stopped"), "{body}");
    assert!(body.contains("could not start it again"), "{body}");
    assert!(tmux.logged().contains("kill-pane"), "{}", tmux.logged());
    assert!(!record.exists(), "the old record was left behind");
    // What `hub を起動` resumes from is still there.
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), saved_text);
}

#[test]
fn the_state_says_whether_a_hub_can_be_resumed_from_the_board() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let refused = state_of(&resident);
    assert_eq!(refused["hubResume"]["available"], false, "{refused}");
    assert!(
        refused["hubResume"]["reason"]
            .as_str()
            .unwrap()
            .contains("tmux"),
        "{refused}"
    );

    write_tmux_config(&fixture);
    let ready = state_of(&resident);
    assert_eq!(ready["hubResume"]["available"], true, "{ready}");
    assert!(ready["hubResume"]["reason"].is_null(), "{ready}");

    write_tmux_config_with(&fixture, |config| {
        config["hubResumeRunner"] = serde_json::json!("claude --continue");
    });
    let blind = state_of(&resident);
    assert_eq!(blind["hubResume"]["available"], false, "{blind}");
    assert!(
        blind["hubResume"]["reason"]
            .as_str()
            .unwrap()
            .contains("hubResumeRunner has no {sessionId}"),
        "{blind}"
    );

    write_tmux_config_with(&fixture, |config| {
        config["hubRunner"] = serde_json::json!("my-agent {prompt}");
    });
    let own = state_of(&resident);
    assert_eq!(own["hubResume"]["available"], false, "{own}");
}

#[test]
fn closing_a_parent_hub_with_no_workers_stops_it_and_takes_it_off_the_list() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let sleeper = Sleeper::new();
    let (record, board) = running_parent_hub(&fixture, &tmux, sleeper.pid());
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.pid()));
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
    assert_eq!(ps_started(sleeper.pid()), "", "the hub is still running");
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
    let sleeper = Sleeper::new();
    let (record, board) = running_parent_hub(&fixture, &tmux, sleeper.pid());

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
    let resident = resident_with_tmux(&fixture, &tmux, Some(sleeper.pid()));

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
