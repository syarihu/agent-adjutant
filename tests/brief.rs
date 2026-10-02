//! `adj task brief`: the worker's `.claude/task-brief.md`, written from the record.

mod common;

use common::*;
use std::io::Write;
use std::process::Stdio;

/// A brief for a session with no task, the instruction coming in on stdin.
fn session_brief(fixture: &Fixture, instruction: &str) -> std::process::Output {
    let mut child = fixture
        .command([
            "task",
            "brief",
            "--worktree",
            fixture.repo.to_str().unwrap(),
            "--base",
            "origin/main",
            "--instruction",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(instruction.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn a_task_brief_is_written_from_the_record_and_the_config() {
    let fixture = Fixture::new(QUIET);
    let id = fixture.json(&[
        "task",
        "add",
        "--title",
        "Fix login",
        "--body",
        "x",
        "--issue-url",
        "https://github.com/acme/widget/issues/12",
        "--done-when",
        "verify",
        "--stop-at",
        "diff",
        "--waiting-in",
        fixture.repo.to_str().unwrap(),
        "--json",
    ])["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    fixture.ok(&[
        "task",
        "update",
        "--id",
        &id,
        "--instruction",
        "Look at the retry first",
    ]);

    let args = [
        "task",
        "brief",
        "--id",
        &id,
        "--worktree",
        fixture.repo.to_str().unwrap(),
        "--base",
        "origin/main",
        "--json",
    ];
    let written = fixture.json(&args);
    let path = fixture.repo.join(".claude").join("task-brief.md");
    assert_eq!(written["path"], path.to_str().unwrap(), "{written}");
    assert_eq!(written["task"], id.as_str(), "{written}");
    assert_eq!(written["branch"], "main", "{written}");

    let brief = std::fs::read_to_string(&path).unwrap();
    for line in [
        "- Task: WID-12 \"Fix login\" (github)\n  https://github.com/acme/widget/issues/12\n",
        "(branch main)",
        "- Base branch: origin/main\n",
        "- Parent task: -\n",
        &format!("- Task record: {id}\n"),
        "- Done when: up to handing over for verification\n",
        "- Stop at: diff\n",
        "- Handover note: Look at the retry first\n",
        "- Copilot review: ask\n",
        "- Verify commands:\n  - cargo test\n",
    ] {
        assert!(brief.contains(line), "no {line:?} in:\n{brief}");
    }

    // Written again over the first, not refused.
    fixture.json(&args);
}

#[test]
fn a_session_brief_ends_with_the_instruction_and_a_missing_one_is_the_greeting() {
    let fixture = Fixture::new(QUIET);
    let out = session_brief(&fixture, "Look at the flaky test.\n");
    assert!(out.status.success(), "{out:?}");
    let path = fixture.repo.join(".claude").join("task-brief.md");
    let brief = std::fs::read_to_string(&path).unwrap();
    assert!(brief.contains("- Task: -\n"), "{brief}");
    assert!(
        brief.ends_with("## Instruction\n\nLook at the flaky test.\n"),
        "{brief}"
    );

    let out = session_brief(&fixture, "-\n");
    assert!(out.status.success(), "{out:?}");
    let brief = std::fs::read_to_string(&path).unwrap();
    assert!(
        brief.ends_with(
            "## Instruction\n\nNo instruction yet. Greet the person in this tab, say you are ready, and wait for what they want.\n"
        ),
        "{brief}"
    );
}

#[test]
fn a_worktree_off_any_branch_gets_no_brief() {
    let fixture = Fixture::new(QUIET);
    let detached = std::process::Command::new("git")
        .hermetic()
        .args(["checkout", "-q", "--detach"])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(detached.status.success());
    let out = session_brief(&fixture, "anything");
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("is not on a branch"),
        "{out:?}"
    );
}

/// A record waiting in the fixture's own worktree, with whatever extra flags the test wants.
fn record(fixture: &Fixture, extra: &[&str]) -> String {
    let mut args = vec![
        "task",
        "add",
        "--title",
        "Look into it",
        "--body",
        "Why is the build slow?",
        "--waiting-in",
        fixture.repo.to_str().unwrap(),
    ];
    args.extend(extra.iter().copied());
    args.push("--json");
    fixture.json(&args)["task"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn brief_of(fixture: &Fixture, id: &str, extra: &[&str]) -> std::process::Output {
    let mut args = vec![
        "task",
        "brief",
        "--id",
        id,
        "--worktree",
        fixture.repo.to_str().unwrap(),
        "--base",
        "-",
    ];
    args.extend(extra.iter().copied());
    fixture.cmd(&args)
}

#[test]
fn a_task_with_no_issue_carries_its_request_text() {
    let fixture = Fixture::new(QUIET);
    let id = record(&fixture, &["--issue-url", ""]);
    let out = brief_of(&fixture, &id, &[]);
    assert!(out.status.success(), "{out:?}");
    let brief = std::fs::read_to_string(fixture.repo.join(".claude/task-brief.md")).unwrap();
    assert!(
        brief.contains("- Task: - \"Look into it\" (-)\n  Why is the build slow?\n"),
        "{brief}"
    );
    assert!(brief.contains("- Base branch: -\n"), "{brief}");
}

#[test]
fn key_and_tracker_can_be_given_and_out_moves_the_file() {
    let fixture = Fixture::new(QUIET);
    // A repository the config has no key for: nothing to read the key from.
    let id = record(
        &fixture,
        &["--issue-url", "https://github.com/acme/unknown/issues/3"],
    );
    let out = brief_of(&fixture, &id, &[]);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("pass --key and --tracker"),
        "{out:?}"
    );

    let elsewhere = fixture._dir.path().join("elsewhere").join("brief.md");
    let out = brief_of(
        &fixture,
        &id,
        &[
            "--key",
            "WEB-3",
            "--tracker",
            "github-project",
            "--out",
            elsewhere.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "{out:?}");
    let brief = std::fs::read_to_string(&elsewhere).unwrap();
    assert!(
        brief.contains(
            "- Task: WEB-3 \"Look into it\" (github-project)\n  https://github.com/acme/unknown/issues/3\n"
        ),
        "{brief}"
    );
    assert!(!fixture.repo.join(".claude/task-brief.md").exists());
}

#[test]
fn a_task_handed_to_jules_gets_no_brief() {
    let fixture = Fixture::new(QUIET);
    let id = record(&fixture, &["--executor", "jules"]);
    let out = brief_of(&fixture, &id, &[]);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("is handed to Jules"),
        "{out:?}"
    );
    assert!(!fixture.repo.join(".claude/task-brief.md").exists());
}

#[test]
fn the_parent_comes_from_the_flag_then_the_record_then_a_dash() {
    let fixture = Fixture::new(QUIET);
    let path = fixture.repo.join(".claude/task-brief.md");
    let read = || std::fs::read_to_string(&path).unwrap();

    let none = record(&fixture, &[]);
    assert!(brief_of(&fixture, &none, &[]).status.success());
    assert!(read().contains("- Parent task: -\n"), "{}", read());

    let child = record(
        &fixture,
        &["--parent", "https://github.com/acme/widget/issues/1"],
    );
    assert!(brief_of(&fixture, &child, &[]).status.success());
    assert!(
        read().contains("- Parent task: https://github.com/acme/widget/issues/1\n"),
        "{}",
        read()
    );
    // A blank flag is not given.
    assert!(
        brief_of(&fixture, &child, &["--parent", ""])
            .status
            .success()
    );
    assert!(read().contains("issues/1\n"), "{}", read());

    let over = "https://github.com/acme/widget/issues/2";
    assert!(
        brief_of(&fixture, &child, &["--parent", over])
            .status
            .success()
    );
    assert!(
        read().contains(&format!("- Parent task: {over}\n")),
        "{}",
        read()
    );

    let before = read();
    let out = brief_of(&fixture, &child, &["--parent", "x'; echo hi; '"]);
    assert!(!out.status.success(), "{out:?}");
    assert_eq!(
        read(),
        before,
        "a refused brief changed the one already written"
    );
}

#[test]
fn an_unknown_tracker_is_refused() {
    let fixture = Fixture::new(QUIET);
    let id = record(&fixture, &[]);
    let out = brief_of(&fixture, &id, &["--tracker", "bogus"]);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no such tracker: bogus"),
        "{out:?}"
    );
}

#[test]
fn an_issue_added_later_fills_in_for_a_blank_issue_url() {
    let fixture = Fixture::new(QUIET);
    let id = record(&fixture, &["--issue-url", ""]);
    fixture.ok(&[
        "task",
        "update",
        "--id",
        &id,
        "--issue",
        "https://github.com/acme/widget/issues/12",
    ]);
    assert!(brief_of(&fixture, &id, &[]).status.success());
    let brief = std::fs::read_to_string(fixture.repo.join(".claude/task-brief.md")).unwrap();
    assert!(
        brief.contains(
            "- Task: WID-12 \"Look into it\" (github)\n  https://github.com/acme/widget/issues/12\n"
        ),
        "{brief}"
    );
}

#[test]
fn an_id_and_an_instruction_together_are_refused() {
    let fixture = Fixture::new(QUIET);
    let id = record(&fixture, &[]);
    let out = brief_of(&fixture, &id, &["--instruction", "do it"]);
    assert!(!out.status.success(), "{out:?}");
    assert!(!fixture.repo.join(".claude/task-brief.md").exists());
}

#[test]
fn an_instruction_on_the_command_line_ends_the_session_brief_verbatim() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.cmd(&[
        "task",
        "brief",
        "--worktree",
        fixture.repo.to_str().unwrap(),
        "--base",
        "origin/main",
        "--instruction",
        "Look at the flaky test.\n## not a header",
    ]);
    assert!(out.status.success(), "{out:?}");
    let brief = std::fs::read_to_string(fixture.repo.join(".claude/task-brief.md")).unwrap();
    assert!(
        brief.ends_with("## Instruction\n\nLook at the flaky test.\n## not a header\n"),
        "{brief}"
    );
}

#[test]
fn a_key_that_is_not_a_key_is_refused() {
    let fixture = Fixture::new(QUIET);
    let id = record(&fixture, &[]);
    let out = brief_of(&fixture, &id, &["--key", "x'; id; '", "--tracker", "jira"]);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not a tracker key"),
        "{out:?}"
    );
    let out = brief_of(&fixture, &id, &["--key", "ABC-123", "--tracker", "jira"]);
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn a_parent_that_is_a_bare_key_is_refused_until_it_is_a_url() {
    let fixture = Fixture::new(QUIET);
    let id = record(&fixture, &["--parent", "ALPHA-233"]);
    let out = brief_of(&fixture, &id, &[]);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("the parent task is a key (ALPHA-233)"),
        "{out:?}"
    );
    let url = "https://github.com/acme/widget/issues/7";
    assert!(brief_of(&fixture, &id, &["--parent", url]).status.success());
    let brief = std::fs::read_to_string(fixture.repo.join(".claude/task-brief.md")).unwrap();
    assert!(
        brief.contains(&format!("- Parent task: {url}\n")),
        "{brief}"
    );
}
