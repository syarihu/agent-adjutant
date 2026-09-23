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
