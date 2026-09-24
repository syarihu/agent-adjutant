//! Starting a worker: the launcher the tab runs, the runner the config names, and the title it
//! is given.

mod common;

use common::*;

#[test]
fn the_tab_runs_the_launcher_so_the_worker_can_write_down_its_own_pid() {
    let fixture = Fixture::new(CODEX);
    let out = fixture.ok(&[
        "work",
        "--worktree",
        fixture.repo.to_str().unwrap(),
        "--title",
        "WID-1 画像が潰れる",
        "--dry-run",
    ]);
    // Starting the agent directly here would record a PID belonging to nothing, and waking
    // a worker later needs a PID that is still the worker.
    assert!(out.contains(" worker --worktree "), "{out}");
    assert!(out.contains("WID-1 画像が潰れる"), "{out}");
}

#[test]
fn a_worker_is_started_by_whatever_the_config_names() {
    let fixture = Fixture::new(CODEX);
    let out = fixture.ok(&[
        "worker",
        "--worktree",
        fixture.repo.to_str().unwrap(),
        "--title",
        "WID-1",
        "--dry-run",
    ]);
    assert!(out.contains("codex exec"), "{out}");
    assert!(!out.contains("claude --permission-mode"), "{out}");
    assert!(out.contains("task-brief.md"), "{out}");
}

#[test]
fn a_title_with_shell_metacharacters_cannot_run_anything() {
    // Deliberately not a dry run. A dry run opens no tab and runs no template, so asserting
    // that a file was not created proves only that nothing happened at all — which is what
    // this test used to do, and why it could not have failed however the title was handled.
    let fixture = Fixture::new(QUIET);
    let recorder = fixture.repo.join("recorder.sh");
    let recorded = fixture.repo.join("argv.txt");
    let pwned = fixture.repo.join("pwned");
    std::fs::write(
        &recorder,
        format!(
            "#!/bin/sh\n: > {0}\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> {0}; done\n",
            shell_quoted(&recorded.to_string_lossy())
        ),
    )
    .unwrap();
    // A spawn template with `{cwd}` in it takes an argv, so the title and the command are
    // each meant to arrive as exactly one element.
    std::fs::write(
        &fixture.config,
        serde_json::json!({
            "notification": "true",
            "defaults": { "ide": "code" },
            "repos": {},
            "terminal": { "spawn": format!(
                "sh {} {{cwd}} {{title}} {{command}}",
                shell_quoted(&recorder.to_string_lossy())
            )},
        })
        .to_string(),
    )
    .unwrap();

    // Short on purpose: a tab title is truncated past a certain length, and a title that
    // arrives shortened would pass this test for the wrong reason. Relative, so that if the
    // shell ever did run it the file lands in the working directory the command is given.
    let title = "$(touch pwned)".to_string();
    fixture.ok(&[
        "spawn",
        "--cwd",
        fixture.repo.to_str().unwrap(),
        "--title",
        &title,
        "--",
        "echo",
        "hi there",
    ]);

    let argv: Vec<String> = std::fs::read_to_string(&recorded)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(argv[0], fixture.repo.to_string_lossy(), "{argv:?}");
    // One element, verbatim: the title went into an argument slot rather than into
    // something the shell reads as syntax. (`{command}` is substituted raw on purpose —
    // the template author decides how to quote it — so it is the tail, not one element.)
    assert_eq!(argv[1], title, "{argv:?}");
    assert_eq!(argv[2..].join(" "), "echo hi there", "{argv:?}");
    assert!(!pwned.exists(), "the title ran: {argv:?}");
}

#[test]
fn naming_this_tab_is_replaceable_and_can_be_turned_off() {
    let fixture = Fixture::new(
        r#"{"notification": "true", "terminal": {"title": "tmux rename-window {title}"},
            "defaults": {"ide": "code"}, "repos": {}}"#,
    );
    assert_eq!(
        fixture
            .ok(&["title", "--title", "🗂 hub widget", "--dry-run"])
            .trim(),
        "tmux rename-window '🗂 hub widget'"
    );

    let off = Fixture::new(r#"{"terminal": {"title": false}, "repos": {}}"#);
    assert_eq!(off.ok(&["title", "--title", "x", "--dry-run"]).trim(), "");
}

// ── maxWorkers ──

/// `QUIET` with a limit of one worker.
fn one_worker_at_a_time() -> Fixture {
    Fixture::new(&QUIET.replacen(
        r#""notification": "true","#,
        r#""notification": "true", "maxWorkers": 1,"#,
        1,
    ))
}

/// A linked worktree of the fixture's repository, as git prints it.
fn linked_worktree(fixture: &Fixture, name: &str) -> String {
    let path = fixture.repo.parent().unwrap().join(name);
    let out = std::process::Command::new("git")
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
    std::fs::canonicalize(path)
        .unwrap()
        .to_string_lossy()
        .to_string()
}

/// What `adj work` leaves behind before the tab has had time to register anyone.
fn just_dispatched(worktree: &str) {
    let dir = std::path::Path::new(worktree).join(".claude");
    std::fs::create_dir_all(&dir).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        dir.join("adjutant-worker-starting.json"),
        format!(r#"{{"at": {now}}}"#),
    )
    .unwrap();
}

#[test]
fn a_dispatch_past_max_workers_starts_nothing_and_says_so_with_its_own_exit_code() {
    let fixture = one_worker_at_a_time();
    let busy = linked_worktree(&fixture, "wid-1");
    let next = linked_worktree(&fixture, "wid-2");

    // A free slot: the dry run goes through, and leaves no mark — it started nothing.
    fixture.ok(&["work", "--worktree", &next, "--title", "WID-2", "--dry-run"]);
    assert!(
        !std::path::Path::new(&next)
            .join(".claude/adjutant-worker-starting.json")
            .exists()
    );

    // One worker dispatched and not registered yet is already the one allowed.
    just_dispatched(&busy);
    let out = fixture.cmd(&["work", "--worktree", &next, "--title", "WID-2", "--dry-run"]);
    // 3 rather than 1: the hub answers "full" by queueing the task, and any other failure
    // by reporting it.
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("worker limit reached"), "{said}");
    assert!(said.contains(&busy), "{said}");
    assert!(out.stdout.is_empty(), "{out:?}");

    // The worktree being dispatched into is taking a slot, not competing for one.
    fixture.ok(&["work", "--worktree", &busy, "--title", "WID-1", "--dry-run"]);
}

#[test]
fn a_task_waiting_for_a_slot_is_queued_with_its_worktree_and_sends_the_hub_nothing() {
    let fixture = one_worker_at_a_time();
    let worktree = linked_worktree(&fixture, "wid-3");
    let added = fixture.json(&[
        "task",
        "add",
        "--title",
        "WID-3",
        "--body",
        "画像が潰れる",
        "--waiting-in",
        &worktree,
        "--json",
    ]);
    assert_eq!(added["task"]["status"], "queued", "{added}");
    assert_eq!(added["task"]["worktree"], worktree.as_str(), "{added}");
    assert!(added["handed"].is_null(), "{added}");
    // The hub writing this down is the hub that would read it.
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

#[test]
fn a_worktree_that_does_not_exist_is_refused_rather_than_made() {
    // Marking the slot writes into the worktree, and a write that made its directories
    // would turn a mistyped path into a tab opened in an empty directory.
    let fixture = Fixture::new(QUIET);
    let missing = fixture.repo.parent().unwrap().join("typo");
    let out = fixture.cmd(&[
        "work",
        "--worktree",
        missing.to_str().unwrap(),
        "--title",
        "x",
    ]);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no such directory"),
        "{out:?}"
    );
    assert!(!missing.exists());
}

#[test]
fn a_tab_that_failed_to_open_gives_its_slot_back() {
    let fixture = Fixture::new(
        r#"{"notification": "true", "terminal": {"spawn": "false {cwd} {command}"}, "repos": {}}"#,
    );
    let worktree = linked_worktree(&fixture, "wid-4");
    let out = fixture.cmd(&["work", "--worktree", &worktree, "--title", "WID-4"]);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        !std::path::Path::new(&worktree)
            .join(".claude/adjutant-worker-starting.json")
            .exists()
    );
}

#[test]
fn a_worker_turned_away_by_one_already_running_gives_back_the_slot_it_was_marked_with() {
    let fixture = Fixture::new(QUIET);
    let worktree = linked_worktree(&fixture, "wid-5");
    just_dispatched(&worktree);
    // The worker already there: this test process, which is certainly running.
    let pid = std::process::id();
    std::fs::write(
        std::path::Path::new(&worktree).join(".claude/adjutant-worker.json"),
        serde_json::json!({"pid": pid, "title": "WID-5", "psStarted": ps_started(pid)}).to_string(),
    )
    .unwrap();

    let out = fixture.ok(&["worker", "--worktree", &worktree, "--title", "WID-5"]);
    assert!(out.contains("already running"), "{out}");
    assert!(
        !std::path::Path::new(&worktree)
            .join(".claude/adjutant-worker-starting.json")
            .exists()
    );
}

#[test]
fn a_worker_in_the_main_checkout_counts_against_max_workers_too() {
    // Not where workers are meant to go, but `adj work` does not refuse it, so leaving it
    // out of the count would let one more start than the limit says.
    let fixture = one_worker_at_a_time();
    let next = linked_worktree(&fixture, "wid-6");
    just_dispatched(fixture.repo.to_str().unwrap());
    let out = fixture.cmd(&["work", "--worktree", &next, "--title", "WID-6", "--dry-run"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
}

#[test]
fn the_hub_finds_a_worktrees_task_and_clears_a_note_by_saying_nothing() {
    let fixture = Fixture::new(QUIET);
    let worktree = linked_worktree(&fixture, "wid-7");
    let other = linked_worktree(&fixture, "wid-8");
    let added = fixture.json(&[
        "task",
        "add",
        "--title",
        "WID-7",
        "--body",
        "x",
        "--waiting-in",
        &worktree,
        "--json",
    ]);
    fixture.ok(&[
        "task",
        "add",
        "--title",
        "WID-8",
        "--body",
        "y",
        "--waiting-in",
        &other,
    ]);
    let id = added["task"]["id"].as_str().unwrap().to_string();
    // No note of its own: the same record is written before a worker starts, not only when
    // one was turned away.
    assert!(added["task"]["note"].is_null(), "{added}");

    // Asked by worktree, which is all the hub has in hand when a worker reports done.
    let found = fixture.json(&["task", "list", "--worktree", &worktree, "--json"]);
    let found = found.as_array().unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["id"], id.as_str());

    fixture.ok(&["task", "update", "--id", &id, "--note", "worker の枠待ち"]);
    // A command line cannot say null, and the note has to go once the worker starts.
    let updated = fixture.json(&[
        "task",
        "update",
        "--id",
        &id,
        "--status",
        "dispatched",
        "--note",
        "",
        "--json",
    ]);
    assert!(updated["task"]["note"].is_null(), "{updated}");
    // Neither record ever went to the hub: the hub is the one writing them.
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

#[test]
fn the_hub_can_requeue_a_task_without_messaging_itself_and_mark_one_approved() {
    let fixture = Fixture::new(QUIET);
    let worktree = linked_worktree(&fixture, "wid-9");
    let added = fixture.json(&[
        "task",
        "add",
        "--title",
        "WID-9",
        "--body",
        "z",
        "--ask-first",
        "--waiting-in",
        &worktree,
        "--json",
    ]);
    let id = added["task"]["id"].as_str().unwrap().to_string();
    assert_eq!(added["task"]["autoStart"], false, "{added}");

    // Approved on the board: from here the queue may start it without asking again.
    let approved = fixture.json(&[
        "task",
        "update",
        "--id",
        &id,
        "--auto-start",
        "true",
        "--json",
    ]);
    assert_eq!(approved["task"]["autoStart"], true, "{approved}");

    // A resumed worker turned away for a slot goes back to queued. The hub is the one doing
    // it, so nothing may land in its own inbox.
    fixture.ok(&["task", "update", "--id", &id, "--status", "dispatched"]);
    fixture.ok(&[
        "task",
        "update",
        "--id",
        &id,
        "--status",
        "queued",
        "--no-hand-over",
    ]);
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);

    // Without the flag, queueing is still a hand-over — the board relies on that.
    fixture.ok(&["task", "update", "--id", &id, "--status", "dispatched"]);
    fixture.ok(&["task", "update", "--id", &id, "--status", "queued"]);
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 1);
}

#[test]
fn a_task_body_given_as_a_dash_is_read_from_stdin_and_nothing_in_it_is_run() {
    // How the hub writes a record: the title and summary come from a task or a report, so
    // they go through a quoted heredoc rather than onto the command line.
    let fixture = Fixture::new(QUIET);
    let worktree = linked_worktree(&fixture, "wid-10");
    let mut child = fixture
        .command([
            "task",
            "add",
            "--body",
            "-",
            "--done-when",
            "report-only",
            "--waiting-in",
            &worktree,
            "--json",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"WID-10 it's broken; touch pwned\n\nsummary with 'quotes' and $(date)\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let added: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(added["task"]["title"], "WID-10 it's broken; touch pwned");
    assert_eq!(added["task"]["doneWhen"], "report-only");
    assert!(added["task"]["body"].as_str().unwrap().contains("$(date)"));
}

#[test]
fn a_worktree_name_or_issue_url_that_could_break_out_of_a_quote_is_refused() {
    // Both are typed on the board and later quoted into commands the hub runs.
    let fixture = Fixture::new(QUIET);
    for args in [
        vec!["--worktree-name", "x'; touch pwned; '"],
        vec!["--worktree-name", "has space"],
        vec![
            "--issue-url",
            "https://github.com/acme/widget/issues/1'; touch pwned; '",
        ],
        vec!["--issue-url", "file:///etc/passwd"],
        vec!["--issue-url", "https://"],
        vec!["--worktree-name", ".."],
        vec!["--worktree-name", "-x"],
        vec!["--worktree-name", "a.lock"],
        vec!["--worktree-name", "topic."],
        vec!["--issue-url", "https://:8080/acme/widget/issues/1"],
        vec!["--issue-url", "https://github.com:abc/acme/widget/issues/1"],
    ] {
        let mut all = vec!["task", "add", "--body", "x"];
        all.extend(&args);
        let out = fixture.cmd(&all);
        assert!(!out.status.success(), "{args:?} was accepted");
    }
    // A refusal leaves nothing behind: the id is claimed only once the values pass.
    let tasks = fixture.state.join("tasks");
    let left: Vec<_> = std::fs::read_dir(&tasks)
        .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(
        left.iter()
            .all(|p| p.is_dir() && std::fs::read_dir(p).unwrap().next().is_none()),
        "{left:?}"
    );
    fixture.ok(&[
        "task",
        "add",
        "--body",
        "x",
        "--worktree-name",
        "login-retry.2_b",
        "--issue-url",
        "https://github.com/acme/widget/issues/1",
    ]);
}

/// Run with `input` on stdin, the way the procedures redirect a file in.
fn with_stdin(fixture: &Fixture, args: &[&str], input: &str) -> std::process::Output {
    use std::io::Write;
    let mut child = fixture
        .command(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn a_worker_tab_is_named_after_its_task_record_so_the_title_never_touches_the_shell() {
    let fixture = Fixture::new(CODEX);
    let worktree = linked_worktree(&fixture, "wid-11");
    let title = "WID-11 it's broken; touch pwned";
    let added = with_stdin(
        &fixture,
        &[
            "task",
            "add",
            "--body",
            "-",
            "--waiting-in",
            &worktree,
            "--json",
        ],
        &format!("{title}\n\nsummary\n"),
    );
    assert!(added.status.success(), "{added:?}");
    let added: serde_json::Value = serde_json::from_slice(&added.stdout).unwrap();
    let id = added["task"]["id"].as_str().unwrap().to_string();

    let out = fixture.ok(&["work", "--worktree", &worktree, "--task", &id, "--dry-run"]);
    // Quoted by adjutant for the terminal, and so present as one argument rather than run.
    assert!(out.contains("WID-11 it"), "{out}");
    // The tab names itself with stdin closed: a title of `-` would otherwise be read from the
    // terminal, and the worker behind it would never start.
    assert!(out.contains("< /dev/null"), "{out}");

    // A title that looks like an option stays the title's value: passed as its own word,
    // `--help` would be read by clap in the new tab and the worker would never start.
    let dashed = with_stdin(
        &fixture,
        &[
            "task",
            "add",
            "--body",
            "-",
            "--waiting-in",
            &worktree,
            "--json",
        ],
        "--help\n",
    );
    let dashed: serde_json::Value = serde_json::from_slice(&dashed.stdout).unwrap();
    let dashed_id = dashed["task"]["id"].as_str().unwrap().to_string();
    let out = fixture.ok(&[
        "work",
        "--worktree",
        &worktree,
        "--task",
        &dashed_id,
        "--dry-run",
    ]);
    assert!(out.contains("--title=--help"), "{out}");
    assert!(!out.contains("--title --help"), "{out}");

    // One or the other: a --task beside --title or --resume would go unchecked.
    for extra in [["--title", "x"].as_slice(), ["--resume"].as_slice()] {
        let mut args = vec![
            "work",
            "--worktree",
            worktree.as_str(),
            "--task",
            id.as_str(),
        ];
        args.extend_from_slice(extra);
        args.push("--dry-run");
        assert!(
            !fixture.cmd(&args).status.success(),
            "{extra:?} was taken with --task"
        );
    }

    let missing = fixture.cmd(&[
        "work",
        "--worktree",
        &worktree,
        "--task",
        "nope",
        "--dry-run",
    ]);
    assert!(!missing.status.success(), "{missing:?}");
}

#[test]
fn a_note_and_a_tab_title_given_as_a_dash_are_read_from_stdin() {
    let fixture = Fixture::new(QUIET);
    let worktree = linked_worktree(&fixture, "wid-12");
    let added = fixture.json(&[
        "task",
        "add",
        "--body",
        "x",
        "--waiting-in",
        &worktree,
        "--json",
    ]);
    let id = added["task"]["id"].as_str().unwrap().to_string();

    let reason = "着手できなかったのだ: fatal: 'origin/x' is not a commit; $(date)\n";
    let out = with_stdin(
        &fixture,
        &["task", "update", "--id", &id, "--note", "-", "--json"],
        reason,
    );
    assert!(out.status.success(), "{out:?}");
    let updated: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(updated["task"]["note"], reason.trim_end());

    // A title template of its own, because the built-in writes to this process's terminal and
    // prints nothing where there is none — which is every CI runner.
    let titled = Fixture::new(
        r#"{"notification": "true", "terminal": {"title": "tmux rename-window {title}"}, "repos": {}}"#,
    );
    let out = with_stdin(
        &titled,
        &["title", "--title", "-", "--dry-run"],
        "WID-12 it's a tab\n",
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        r#"tmux rename-window 'WID-12 it'\''s a tab'"#,
        "{out:?}"
    );
}

/// A worker record naming this test process, which is certainly running.
fn a_running_worker_in(worktree: &str) {
    let pid = std::process::id();
    let dir = std::path::Path::new(worktree).join(".claude");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("adjutant-worker.json"),
        serde_json::json!({"pid": pid, "title": "WID-13", "psStarted": ps_started(pid)})
            .to_string(),
    )
    .unwrap();
}

#[test]
fn a_worker_says_which_phase_it_is_in_and_a_typo_is_refused() {
    let fixture = Fixture::new(QUIET);
    let worktree = linked_worktree(&fixture, "wid-13");
    // Nobody registered there yet: there is no run of a worker to describe.
    assert!(
        !fixture
            .cmd(&["phase", "--set", "plan", "--worktree", &worktree])
            .status
            .success()
    );

    a_running_worker_in(&worktree);
    fixture.ok(&["phase", "--set", "self-review", "--worktree", &worktree]);
    assert_eq!(
        fixture.ok(&["phase", "--worktree", &worktree]).trim(),
        "self-review"
    );
    assert!(
        !fixture
            .cmd(&["phase", "--set", "reviewing", "--worktree", &worktree])
            .status
            .success()
    );
}

#[test]
fn a_workers_tab_is_raised_through_the_focus_template() {
    let fixture = Fixture::new(
        r#"{"notification": "true", "terminal": {"focus": "raise {pid} {title}"}, "repos": {}}"#,
    );
    let worktree = linked_worktree(&fixture, "wid-14");
    // Nothing running there: exit 1, the same answer `adj focus` gives for a hub that is down.
    let none = fixture.cmd(&["focus", "--worktree", &worktree, "--dry-run"]);
    assert_eq!(none.status.code(), Some(1), "{none:?}");

    a_running_worker_in(&worktree);
    let out = fixture.ok(&["focus", "--worktree", &worktree, "--dry-run"]);
    assert!(
        out.contains(&format!("raise {} WID-13", std::process::id())),
        "{out}"
    );
}

#[test]
fn a_tasks_worktree_is_stored_the_way_git_names_it() {
    // The board matches a task to its worker by this string. Stored through a symlink, it
    // read as a worker that was not there.
    let fixture = Fixture::new(QUIET);
    let worktree = linked_worktree(&fixture, "wid-15");
    let link = fixture.repo.parent().unwrap().join("via-link");
    std::os::unix::fs::symlink(&worktree, &link).unwrap();
    let added = fixture.json(&[
        "task",
        "add",
        "--body",
        "x",
        "--waiting-in",
        link.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(added["task"]["worktree"], worktree.as_str(), "{added}");
    let id = added["task"]["id"].as_str().unwrap().to_string();
    let updated = fixture.json(&[
        "task",
        "update",
        "--id",
        &id,
        "--worktree",
        link.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(updated["task"]["worktree"], worktree.as_str(), "{updated}");
}

#[test]
fn a_stored_worktree_is_left_alone_by_an_update_that_does_not_give_one() {
    // Resolved when given, against the directory of the command that gave it. An update run
    // from somewhere else must not read the stored path against its own directory.
    let fixture = Fixture::new(QUIET);
    let added = fixture.json(&[
        "task",
        "add",
        "--body",
        "x",
        "--waiting-in",
        "not-yet/wt",
        "--json",
    ]);
    let stored = added["task"]["worktree"].as_str().unwrap().to_string();
    assert!(
        stored.starts_with('/'),
        "made absolute where it was given: {stored}"
    );
    let id = added["task"]["id"].as_str().unwrap().to_string();
    // Somewhere else inside the repository, where the same relative path does exist.
    let elsewhere = fixture.repo.join("sub");
    std::fs::create_dir_all(elsewhere.join("not-yet/wt")).unwrap();
    let out = fixture
        .command([
            "task",
            "update",
            "--id",
            &id,
            "--status",
            "dispatched",
            "--json",
        ])
        .current_dir(&elsewhere)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let updated: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(updated["task"]["worktree"], stored.as_str(), "{updated}");
}

#[test]
fn a_worktree_not_created_yet_is_stored_as_git_will_name_it() {
    // Resolved through the part of it that exists, so a path through a symlink matches what
    // `git worktree list` prints once the worktree is created there.
    let fixture = Fixture::new(QUIET);
    let real = fixture.repo.parent().unwrap().join("real-base");
    std::fs::create_dir_all(&real).unwrap();
    let link = fixture.repo.parent().unwrap().join("link-base");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let added = fixture.json(&[
        "task",
        "add",
        "--body",
        "x",
        "--waiting-in",
        link.join("wid-16").to_str().unwrap(),
        "--json",
    ]);
    let expected = std::fs::canonicalize(&real).unwrap().join("wid-16");
    assert_eq!(
        added["task"]["worktree"],
        expected.to_str().unwrap(),
        "{added}"
    );
}
