//! What the resident's board does with a session (resume, restart, open, clean up) and starting a
//! parent-task hub from it, run against a fake tmux.

mod common;

use common::*;

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
    let running = Sleeper::new();
    session_worktree(&fixture, "busy", None, None, running.pid());
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

/// The pane a running worker's window shows up as in the fake tmux.
fn worker_pane(tmux: &FakeTmux, pid: u32, window: &str) {
    std::fs::write(
        &tmux.panes,
        format!("%3\t{pid}\t/dev/ttys999\t@1\tadjutant-test\t1\t{window}\n"),
    )
    .unwrap();
}

#[test]
fn restarting_a_worker_closes_its_window_and_resumes_it() {
    let fixture = Fixture::new(QUIET);
    resume_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "live", Some(FEATURE), None, running.pid());
    worker_pane(&tmux, running.pid(), "live");
    let saved_before = saved_session(&worktree);
    let resident = resident_with_tmux(&fixture, &tmux, Some(running.pid()));

    let answer = reply_of(
        resident.post(&sessions_url("/worker-live/restart"), "{}"),
        200,
    );
    assert_eq!(answer["restarted"], true, "{answer}");
    assert_eq!(answer["wasRunning"], true, "{answer}");
    assert_eq!(answer["hub"], format!("hub-{FEATURE}"), "{answer}");
    assert_eq!(answer["hubRunning"], false, "{answer}");

    let log = tmux.logged();
    let kill = log
        .lines()
        .position(|l| l.contains("-L scratch kill-window -t @1"))
        .unwrap_or_else(|| panic!("no window was closed: {log}"));
    let open = log
        .lines()
        .position(|l| l.contains("new-window"))
        .unwrap_or_else(|| panic!("no window was opened: {log}"));
    assert!(kill < open, "{log}");
    assert!(
        log.lines().nth(open).unwrap().contains(&format!(
            "worker --resume --worktree {}",
            worktree.display()
        )),
        "{log}"
    );
    assert_eq!(ps_started(running.pid()), "", "the worker is still running");
    assert_eq!(saved_session(&worktree), saved_before);
}

#[test]
fn a_worker_restart_is_refused_before_anything_is_closed() {
    let fixture = Fixture::new(QUIET);
    resume_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "live", None, None, running.pid());
    worker_pane(&tmux, running.pid(), "live");
    let resident = resident_with_tmux(&fixture, &tmux, Some(running.pid()));
    let restart = sessions_url("/worker-live/restart");
    let refused = |expected: &str| {
        let (status, body) = resident.post(&restart, "{}");
        assert_eq!(status, 400, "{body}");
        assert!(body.contains(expected), "{expected}: {body}");
        assert!(!tmux.logged().contains("kill-window"), "{}", tmux.logged());
        assert!(worker_record(&worktree)["pid"].as_u64().is_some());
        assert!(
            !ps_started(running.pid()).is_empty(),
            "the process was killed"
        );
    };

    // It is starting: a second worker would be opened beside the one being opened.
    let marker = worktree
        .join(".claude")
        .join("adjutant-worker-starting.json");
    std::fs::write(
        &marker,
        serde_json::json!({"at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()})
        .to_string(),
    )
    .unwrap();
    refused("starting");
    std::fs::remove_file(&marker).unwrap();

    write_tmux_config_with(&fixture, |config| {
        config["terminal"]["close"] = serde_json::json!(false);
    });
    refused("terminal.close is off");

    write_tmux_config_with(&fixture, |config| {
        config["agentResumeRunner"] = serde_json::json!("claude --continue");
    });
    refused("agentResumeRunner has no {sessionId}");

    write_tmux_config_with(&fixture, |config| {
        config["agentRunner"] = serde_json::json!("gemini {prompt}");
    });
    refused("gemini has no agentResumeRunner");

    write_tmux_config_with(&fixture, |config| {
        config["terminal"] = serde_json::json!({"preset": "iterm2"});
    });
    refused("tmux");

    write_tmux_config(&fixture);
    std::fs::remove_file(worktree.join(".claude").join("adjutant-session.json")).unwrap();
    refused("no saved worker session");

    // Only a worker has a window to close and reopen.
    let (status, body) = resident.post(&sessions_url("/hub/restart"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("only a worker"), "{body}");
}

#[test]
fn a_worker_restart_keeps_a_worker_that_would_not_stop() {
    let fixture = Fixture::new(QUIET);
    resume_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "stubborn", None, None, running.pid());
    worker_pane(&tmux, running.pid(), "stubborn");
    // Nothing kills the process, as a terminal waiting for an answer would not.
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let (status, body) = resident.post(&sessions_url("/worker-stubborn/restart"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("nothing was restarted"), "{body}");
    assert!(!tmux.logged().contains("new-window"), "{}", tmux.logged());
    assert!(worker_record(&worktree)["pid"].as_u64().is_some());
    assert!(
        session_ids(&resident).contains(&"worker-stubborn".to_string()),
        "the session dropped out of the list"
    );
}

#[test]
fn a_worker_restart_that_cannot_start_again_keeps_the_session() {
    let fixture = Fixture::new(QUIET);
    resume_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "live", None, None, running.pid());
    worker_pane(&tmux, running.pid(), "live");
    std::fs::write(format!("{}.failnew", tmux.log.display()), "").unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(running.pid()));

    let (status, body) = resident.post(&sessions_url("/worker-live/restart"), "{}");
    assert_eq!(status, 400, "{body}");
    assert!(
        body.contains("closed the session, but could not start it again"),
        "{body}"
    );
    assert!(tmux.logged().contains("kill-window"), "{}", tmux.logged());
    // Still listed with its conversation, so 再開 works.
    let state = state_of(&resident);
    assert_eq!(
        session_of(&state, "worker-live")["conversation"],
        "sid-1",
        "{state}"
    );
    assert_eq!(saved_session(&worktree)["sessionId"], "sid-1");
}

/// A hub's own board, which none of the actions a resident serves are routes on.
#[test]
fn the_session_actions_and_starting_a_hub_are_not_routes_on_a_hub_s_own_board() {
    let fixture = Fixture::new(QUIET);
    session_worktree(&fixture, "ended", None, None, dead_pid());
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

    let answers: Vec<(u16, String)> = [
        "/api/sessions/worker-ended/resume",
        "/api/sessions/worker-ended/open",
        "/api/sessions/worker-ended/cleanup",
        "/api/sessions/worker-ended/restart",
        "/api/hubs/hub/restart",
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
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "live", None, None, running.pid());
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
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "live", None, None, running.pid());
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
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "live", None, None, running.pid());
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
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "live", None, None, running.pid());
    pushed(&worktree, "live");
    std::fs::write(
        &tmux.panes,
        format!(
            "%3\t{}\t/dev/ttys999\t@1\tadjutant-test\t1\tlive\n",
            running.pid()
        ),
    )
    .unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, Some(running.pid()));

    let answer = reply_of(resident.post(&cleanup_url("live"), "{}"), 200);
    assert_eq!(answer["removed"], true, "{answer}");
    assert_eq!(answer["closed"], true, "{answer}");
    assert!(
        tmux.logged().contains("-L scratch kill-window -t @1"),
        "{}",
        tmux.logged()
    );
    assert!(!worktree.exists());
    assert_eq!(ps_started(running.pid()), "", "the worker is still running");
}

#[test]
fn cleanup_keeps_a_session_whose_worker_would_not_stop() {
    let fixture = Fixture::new(QUIET);
    cleanup_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "stubborn", None, None, running.pid());
    pushed(&worktree, "stubborn");
    std::fs::write(
        &tmux.panes,
        format!(
            "%3\t{}\t/dev/ttys999\t@1\tadjutant-test\t1\tstubborn\n",
            running.pid()
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
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.pid());
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
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "busy-one", None, None, running.pid());
    pushed(&worktree, "busy-one");
    std::fs::write(
        &tmux.panes,
        format!(
            "%3\t{}\t/dev/ttys999\t@1\tadjutant-test\t1\tbusy\n",
            running.pid()
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
    let resident = resident_with_tmux(&fixture, &tmux, Some(running.pid()));

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
