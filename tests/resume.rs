//! `--resume` and the automatic resume of a hub that ended recently.

mod common;

use common::*;

/// A config whose runners start nothing: `true` takes the place of the agent on the exec
/// path, so a test can go all the way through a launch and read what it wrote down.
fn write_resumable_stub_config(fixture: &Fixture, spawned: &Path) {
    std::fs::write(
        &fixture.config,
        serde_json::json!({
            "notification": "true",
            "hubRunner": "true {name} {sessionId} {prompt}",
            "agentRunner": "true {sessionId} {prompt}",
            "terminal": {
                "spawn": format!(
                    "echo {{cwd}} {{command}} > {}",
                    shell_quoted(&spawned.to_string_lossy())
                ),
            },
            "repos": {
                "acme/widget": {"taskSource": "github", "issueRepo": "acme/widget"},
            },
        })
        .to_string(),
    )
    .unwrap();
}

fn saved_session(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("no session saved at {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

/// What the hub's MCP server writes as it beats, forged for a test that cannot run an agent.
fn forge_last_alive(fixture: &Fixture, slug: &str, session: &str, secs_ago: i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let path = fixture.state.join("sessions").join(format!("{slug}.alive"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        serde_json::json!({"sessionId": session, "lastAlive": now - secs_ago}).to_string(),
    )
    .unwrap();
}

fn set_config(fixture: &Fixture, key: &str, value: serde_json::Value) {
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fixture.config).unwrap()).unwrap();
    config[key] = value;
    std::fs::write(&fixture.config, config.to_string()).unwrap();
}

/// Start a hub through the stub runner and hand back the session it saved.
fn started_hub_session(fixture: &Fixture) -> String {
    fixture.ok(&["hub"]);
    let saved = saved_session(&fixture.state.join("sessions").join(format!("{SLUG}.json")));
    saved["sessionId"].as_str().unwrap().to_string()
}

#[test]
fn the_default_runners_hand_the_agent_a_session_id() {
    let fixture = Fixture::new(QUIET);
    let hub = fixture.ok(&["hub", "--dry-run"]);
    assert!(
        hub.contains(&format!("claude -n {HUB} --session-id ")),
        "{hub}"
    );
    let worker = fixture.ok(&[
        "worker",
        "--worktree",
        fixture.repo.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(worker.contains("claude --session-id "), "{worker}");
    assert!(worker.contains("task-brief.md"), "{worker}");
}

#[test]
fn a_hub_resumes_the_session_it_was_started_into() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_resumable_stub_config(&fixture, &spawned);

    fixture.ok(&["hub"]);
    let saved = saved_session(&fixture.state.join("sessions").join(format!("{SLUG}.json")));
    let sid = saved["sessionId"].as_str().unwrap().to_string();
    assert_eq!(sid.len(), 36, "{saved}");
    assert_eq!(saved["hubName"], HUB);
    assert_eq!(saved["nwo"], "acme/widget");

    // Cleared the way a hub shutting down clears it: the session is not the record's.
    fixture.ok(&["hub-stop"]);
    let resumed = fixture.ok(&["hub", "--resume", "--dry-run"]);
    assert!(
        resumed.contains(&format!("claude -n {HUB} --resume {sid} ")),
        "{resumed}"
    );
    assert!(resumed.contains("adjutant_pending"), "{resumed}");
    assert!(!resumed.contains("--session-id"), "{resumed}");

    // The tab route carries the flag across, above the separator.
    let tab = fixture.ok(&["hub", "--tab", "--resume", "--dry-run", "--", "-x"]);
    assert!(tab.contains("--resume -- -x"), "{tab}");
}

#[test]
fn resuming_a_hub_with_nothing_saved_says_which_hubs_can_be() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_resumable_stub_config(&fixture, &spawned);
    fixture.ok(&["hub", "--hub", FEATURE]);
    assert!(
        fixture
            .state
            .join("sessions")
            .join(format!("{FEATURE_SLUG}.json"))
            .exists()
    );

    let out = fixture.cmd(&["hub", "--resume", "--dry-run"]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success(), "{said}");
    assert!(said.contains("has no saved session"), "{said}");
    assert!(
        said.contains(&format!("adj hub --resume --hub {FEATURE}")),
        "{said}"
    );

    let feature = fixture.ok(&["hub", "--resume", "--hub", FEATURE, "--dry-run"]);
    assert!(
        feature.contains(&format!("-n {FEATURE_HUB} --resume ")),
        "{feature}"
    );
}

#[test]
fn a_resume_runner_with_nowhere_to_put_the_session_is_refused() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_resumable_stub_config(&fixture, &spawned);
    fixture.ok(&["hub"]);
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fixture.config).unwrap()).unwrap();
    config["hubResumeRunner"] = "claude --continue {prompt}".into();
    std::fs::write(&fixture.config, config.to_string()).unwrap();

    let out = fixture.cmd(&["hub", "--resume", "--dry-run"]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success(), "{said}");
    assert!(
        said.contains("hubResumeRunner has no {sessionId}"),
        "{said}"
    );
}

#[test]
fn a_worker_resumes_in_its_worktree_under_the_hub_that_dispatched_it() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_resumable_stub_config(&fixture, &spawned);
    let worktree = fixture.repo.to_str().unwrap().to_string();

    fixture.ok(&[
        "worker",
        "--worktree",
        &worktree,
        "--title",
        "WID-1 画像が潰れる",
        "--hub",
        FEATURE,
    ]);
    let saved = saved_session(&fixture.repo.join(".claude").join("adjutant-session.json"));
    let sid = saved["sessionId"].as_str().unwrap().to_string();
    assert_eq!(saved["title"], "WID-1 画像が潰れる");
    assert_eq!(saved["hub"], FEATURE);

    // Typed from inside the worktree with nothing else said: the worktree, the title and
    // the session all come from what was saved.
    let resumed = fixture.ok(&["worker", "--resume", "--dry-run"]);
    assert!(
        resumed.contains(&format!("claude --resume {sid} --permission-mode auto ")),
        "{resumed}"
    );
    assert!(resumed.contains("adjutant_outbox"), "{resumed}");

    // Through to the exec path, in a tab that inherited another hub's identity: the record
    // the resumed worker writes still names the hub that dispatched it.
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fixture.config).unwrap()).unwrap();
    config["agentResumeRunner"] = "true {sessionId}".into();
    std::fs::write(&fixture.config, config.to_string()).unwrap();
    let out = fixture
        .command(["worker", "--resume"])
        .env("ADJUTANT_HUB", "someone-else")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let record = saved_session(&fixture.repo.join(".claude").join("adjutant-worker.json"));
    assert_eq!(record["hub"], FEATURE, "{record}");
    assert_eq!(record["title"], "WID-1 画像が潰れる", "{record}");
    // Resuming reopens the session; it does not replace the one that was saved.
    let again = saved_session(&fixture.repo.join(".claude").join("adjutant-session.json"));
    assert_eq!(again["sessionId"], sid.as_str());
}

#[test]
fn resuming_a_worker_where_nothing_was_saved_is_refused() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.cmd(&["worker", "--resume", "--dry-run"]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success(), "{said}");
    assert!(said.contains("no saved worker session"), "{said}");

    // And without --resume the worktree is still required.
    let out = fixture.cmd(&["worker", "--dry-run"]);
    assert!(!out.status.success());
}

#[test]
fn work_resume_opens_a_tab_that_resumes_and_leaves_the_hub_to_the_saved_session() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_resumable_stub_config(&fixture, &spawned);
    let worktree = fixture.repo.to_str().unwrap().to_string();
    fixture.ok(&[
        "worker",
        "--worktree",
        &worktree,
        "--title",
        "WID-1",
        "--hub",
        FEATURE,
    ]);

    let out = fixture
        .command(["work", "--resume", "--worktree", &worktree, "--dry-run"])
        .env("ADJUTANT_HUB", "someone-else")
        .output()
        .unwrap();
    let line = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(line.contains(" worker --resume --worktree "), "{line}");
    assert!(line.contains("--title=WID-1"), "{line}");
    assert!(!line.contains("--hub"), "{line}");
}

#[test]
fn the_hub_tells_its_mcp_server_which_session_it_is() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.ok(&["hub", "--dry-run"]);
    assert!(
        out.contains(&format!("ADJUTANT_HUB_SESSION={SLUG}/")),
        "{out}"
    );
}

#[test]
fn the_hub_tells_its_mcp_server_to_serve_its_board_unless_told_not_to() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.ok(&["hub", "--dry-run"]);
    assert!(out.contains(&format!("ADJUTANT_HUB_SERVE={SLUG}")), "{out}");

    set_config(&fixture, "hubServe", false.into());
    let off = fixture.ok(&["hub", "--dry-run"]);
    assert!(!off.contains("ADJUTANT_HUB_SERVE"), "{off}");
}

#[test]
fn a_plain_hub_comes_back_to_a_session_that_ended_recently() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_resumable_stub_config(&fixture, &spawned);
    let sid = started_hub_session(&fixture);
    // The stub `hubRunner` is one of its own, and such a hub is only brought back uninvited
    // through a resume runner of its own too.
    set_config(
        &fixture,
        "hubResumeRunner",
        "claude -n {name} --resume {sessionId} {prompt}".into(),
    );

    // Ended ten minutes ago, well inside the default window.
    forge_last_alive(&fixture, SLUG, &sid, 600);
    let out = fixture.cmd(&["hub", "--dry-run"]);
    let line = String::from_utf8_lossy(&out.stdout).to_string();
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{said}");
    assert!(line.contains(&format!("--resume {sid} ")), "{line}");
    assert!(
        line.contains(&format!("ADJUTANT_HUB_SESSION={SLUG}/{sid}")),
        "{line}"
    );
    assert!(said.contains("ended 10 min ago"), "{said}");
    assert!(said.contains("adj hub --new"), "{said}");

    // `--new` is the way out.
    let fresh = fixture.ok(&["hub", "--new", "--dry-run"]);
    assert!(!fresh.contains("--resume"), "{fresh}");
    assert!(!fresh.contains(&sid), "{fresh}");

    // And `--tab` leaves the decision to the tab, carrying only what was said outright.
    let tab = fixture.ok(&["hub", "--tab", "--dry-run"]);
    assert!(!tab.contains("--resume") && !tab.contains("--new"), "{tab}");
    let tab = fixture.ok(&["hub", "--tab", "--new", "--dry-run"]);
    assert!(tab.contains(" --new"), "{tab}");
    assert!(
        !fixture
            .cmd(&["hub", "--resume", "--new", "--dry-run"])
            .status
            .success()
    );
}

#[test]
fn a_plain_hub_starts_fresh_when_the_last_session_is_old_or_unknown() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_resumable_stub_config(&fixture, &spawned);
    let sid = started_hub_session(&fixture);
    // So that the only thing standing between these and a resume is the one each is about.
    set_config(
        &fixture,
        "hubResumeRunner",
        "claude -n {name} --resume {sessionId} {prompt}".into(),
    );

    // Nothing has said when it ended: no MCP server, or an older version.
    let unknown = fixture.ok(&["hub", "--dry-run"]);
    assert!(!unknown.contains("--resume"), "{unknown}");

    // Ended last night.
    forge_last_alive(&fixture, SLUG, &sid, 10 * 3600);
    let old = fixture.ok(&["hub", "--dry-run"]);
    assert!(!old.contains("--resume"), "{old}");

    // Recently — but the beat is about some other session.
    forge_last_alive(&fixture, SLUG, "another-session", 60);
    let other = fixture.ok(&["hub", "--dry-run"]);
    assert!(!other.contains("--resume"), "{other}");

    // Recently, about this one, on a machine that turned it off.
    forge_last_alive(&fixture, SLUG, &sid, 60);
    set_config(&fixture, "hubAutoResumeHours", 0.into());
    let off = fixture.ok(&["hub", "--dry-run"]);
    assert!(!off.contains("--resume"), "{off}");
    // `--resume` still works there.
    let asked = fixture.ok(&["hub", "--resume", "--dry-run"]);
    assert!(asked.contains(&format!("--resume {sid} ")), "{asked}");
}

#[test]
fn the_mcp_server_under_a_hub_records_when_the_session_ended() {
    let fixture = Fixture::new(QUIET);
    let out = fixture
        .command(["mcp"])
        .env("ADJUTANT_HUB_SESSION", format!("{SLUG}/sid-under-test"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let alive = saved_session(&fixture.state.join("sessions").join(format!("{SLUG}.alive")));
    assert_eq!(alive["sessionId"], "sid-under-test");
    assert!(alive["lastAlive"].as_i64().unwrap() > 0, "{alive}");

    // A server with no hub session behind it — every other session on the machine — writes
    // nothing at all.
    let quiet = Fixture::new(QUIET);
    quiet
        .command(["mcp"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!quiet.state.join("sessions").exists());
}

#[test]
fn a_worker_never_inherits_the_hubs_session() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_resumable_stub_config(&fixture, &spawned);
    let seen = fixture.repo.join("seen.txt");
    set_config(
        &fixture,
        "agentRunner",
        format!(
            "sh -c 'echo \"[$ADJUTANT_HUB_SESSION$ADJUTANT_HUB_SERVE]\"' > {} ; true {{sessionId}} {{prompt}}",
            shell_quoted(&seen.to_string_lossy())
        )
        .into(),
    );
    let out = fixture
        .command(["worker", "--worktree", fixture.repo.to_str().unwrap()])
        .env("ADJUTANT_HUB_SESSION", format!("{SLUG}/the-hubs-session"))
        .env("ADJUTANT_HUB_SERVE", SLUG)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read_to_string(&seen).unwrap().trim(), "[]");
}

#[test]
fn a_resume_template_that_cannot_work_is_refused_before_a_tab_is_opened() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_resumable_stub_config(&fixture, &spawned);
    fixture.ok(&["hub"]);
    fixture.ok(&["worker", "--worktree", fixture.repo.to_str().unwrap()]);
    set_config(
        &fixture,
        "hubResumeRunner",
        "claude --continue {prompt}".into(),
    );
    set_config(
        &fixture,
        "agentResumeRunner",
        "claude --continue {prompt}".into(),
    );

    // Not a dry run: the stub spawn writes a file when a tab is opened, and none must be.
    let hub = fixture.cmd(&["hub", "--tab", "--resume"]);
    let said = String::from_utf8_lossy(&hub.stderr).to_string();
    assert!(!hub.status.success(), "{said}");
    assert!(
        said.contains("hubResumeRunner has no {sessionId}"),
        "{said}"
    );

    let work = fixture.cmd(&[
        "work",
        "--resume",
        "--worktree",
        fixture.repo.to_str().unwrap(),
    ]);
    let said = String::from_utf8_lossy(&work.stderr).to_string();
    assert!(!work.status.success(), "{said}");
    assert!(
        said.contains("agentResumeRunner has no {sessionId}"),
        "{said}"
    );
    assert!(
        !spawned.exists(),
        "a tab was opened for a resume that cannot work"
    );
}

#[test]
fn a_fresh_start_that_records_nothing_forgets_what_the_one_before_saved() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_resumable_stub_config(&fixture, &spawned);
    let sid = started_hub_session(&fixture);
    forge_last_alive(&fixture, SLUG, &sid, 60);
    fixture.ok(&["worker", "--worktree", fixture.repo.to_str().unwrap()]);

    set_config(&fixture, "hubRunner", "true {name} {prompt}".into());
    set_config(&fixture, "agentRunner", "true {prompt}".into());
    // `--new`, or the recent beat would bring the old hub back instead.
    fixture.ok(&["hub", "--new"]);
    fixture.ok(&["worker", "--worktree", fixture.repo.to_str().unwrap()]);

    let sessions = fixture.state.join("sessions");
    assert!(!sessions.join(format!("{SLUG}.json")).exists());
    assert!(!sessions.join(format!("{SLUG}.alive")).exists());
    assert!(
        !fixture
            .repo
            .join(".claude")
            .join("adjutant-session.json")
            .exists()
    );
    assert!(
        !fixture
            .cmd(&["hub", "--resume", "--dry-run"])
            .status
            .success()
    );
    assert!(
        !fixture
            .cmd(&["worker", "--resume", "--dry-run"])
            .status
            .success()
    );
    // And a plain `adj hub` does not come back to the hub two starts ago either.
    let plain = fixture.ok(&["hub", "--dry-run"]);
    assert!(!plain.contains(&sid), "{plain}");
}

#[test]
fn a_hub_with_a_runner_of_its_own_is_not_resumed_by_the_built_in_one_uninvited() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    // The stub config's `hubRunner` is already one of its own, and records a session.
    write_resumable_stub_config(&fixture, &spawned);
    let sid = started_hub_session(&fixture);
    forge_last_alive(&fixture, SLUG, &sid, 60);

    let out = fixture.cmd(&["hub", "--dry-run"]);
    let line = String::from_utf8_lossy(&out.stdout).to_string();
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!line.contains("--resume"), "{line}");
    assert!(said.contains("hubResumeRunner"), "{said}");

    // Given a resume runner of its own, it comes back through that one.
    set_config(
        &fixture,
        "hubResumeRunner",
        "true again {name} {sessionId}".into(),
    );
    let resumed = fixture.ok(&["hub", "--dry-run"]);
    assert!(
        resumed.contains(&format!("true again {HUB} {sid}")),
        "{resumed}"
    );
}

#[test]
fn reopening_a_worker_is_held_to_max_workers_like_starting_one() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_resumable_stub_config(&fixture, &spawned);
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fixture.config).unwrap()).unwrap();
    config["maxWorkers"] = serde_json::json!(1);
    std::fs::write(&fixture.config, config.to_string()).unwrap();

    let mut worktrees = Vec::new();
    for name in ["wid-1", "wid-2"] {
        let path = fixture.repo.parent().unwrap().join(name);
        let out = std::process::Command::new("git")
            .hermetic()
            .args(["worktree", "add", "-q", "-b", name])
            .arg(&path)
            .current_dir(&fixture.repo)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        worktrees.push(
            std::fs::canonicalize(path)
                .unwrap()
                .to_string_lossy()
                .to_string(),
        );
    }
    let (busy, crashed) = (&worktrees[0], &worktrees[1]);
    // A worker that ran and ended, leaving its session behind to be reopened.
    fixture.ok(&["worker", "--worktree", crashed, "--title", "WID-2"]);
    // And one just dispatched into the other worktree, taking the only slot.
    let marker = Path::new(busy).join(".claude");
    std::fs::create_dir_all(&marker).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        marker.join("adjutant-worker-starting.json"),
        format!(r#"{{"at": {now}}}"#),
    )
    .unwrap();

    let out = fixture.cmd(&["work", "--resume", "--worktree", crashed, "--dry-run"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    // Refused before anything was touched: the session is still there to reopen later.
    saved_session(
        &Path::new(crashed)
            .join(".claude")
            .join("adjutant-session.json"),
    );
}
