//! End-to-end tests: run the real binary against a throwaway repository and a throwaway
//! state directory.
//!
//! Everything here is hermetic on purpose. `ADJUTANT_CONFIG` and `ADJUTANT_STATE_DIR` point
//! into tempdirs, so a test can never read the developer's own config or drop a fixture
//! report into a hub that is actually running. Anything that would open a window, start an
//! agent or notify a human is exercised through `--dry-run`.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_adjutant");

/// Every fixture repository has the same origin, so its address is fixed too. Written out
/// rather than derived from `adjutant::repo`, so that a change to how a slug is built shows
/// up here as a failing test instead of as two implementations agreeing with each other.
const SLUG: &str = "acme-widget-898449509108182c";
const HUB: &str = "adjutant-acme-widget-898449509108182c";

struct Fixture {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    state: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new(config_json: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("widget");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "test@example.invalid"],
            vec!["config", "user.name", "test"],
            vec!["remote", "add", "origin", "git@github.com:acme/widget.git"],
            vec!["commit", "-q", "--allow-empty", "-m", "init"],
        ] {
            let out = Command::new("git")
                .args(&args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let config = dir.path().join("config.json");
        std::fs::write(&config, config_json).unwrap();
        Fixture {
            state: dir.path().join("state"),
            // macOS puts tempdirs behind the /private symlink and git reports the resolved
            // path, so the fixture holds the resolved one too or every path check disagrees.
            repo: std::fs::canonicalize(&repo).unwrap(),
            _dir: dir,
            config,
        }
    }

    fn cmd(&self, args: &[&str]) -> std::process::Output {
        Command::new(BIN)
            .args(args)
            .current_dir(&self.repo)
            .env("ADJUTANT_CONFIG", &self.config)
            .env("ADJUTANT_STATE_DIR", &self.state)
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.cmd(args);
        assert!(
            out.status.success(),
            "adjutant {args:?} failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        serde_json::from_str(&self.ok(args)).unwrap()
    }
}

/// A config that notifies by doing nothing. Every test that sends uses it: the built-in
/// notifier puts a banner on the developer's screen, which is not something a test suite
/// gets to do.
const QUIET: &str = r#"{
  "notification": "true",
  "defaults": { "ide": "code" },
  "repos": {
    "acme/widget": {
      "taskSource": "github",
      "issueRepo": "acme/widget",
      "issueKeys": { "acme/widget": "WID" },
      "verify": ["cargo test"]
    }
  }
}"#;

#[test]
fn the_hub_name_comes_from_the_remote_not_the_directory() {
    let fixture = Fixture::new(QUIET);
    assert_eq!(fixture.ok(&["hub-name"]).trim(), HUB);
    let info = fixture.json(&["hub-name", "--json"]);
    assert_eq!(info["nwo"], "acme/widget");
    assert_eq!(info["nwoSource"], "origin");
    assert_eq!(
        info["main"].as_str().unwrap(),
        fixture.repo.to_string_lossy()
    );
}

#[test]
fn a_worktree_answers_for_the_repository_it_belongs_to() {
    let fixture = Fixture::new(QUIET);
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-1");
    let out = Command::new("git")
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "wid-1",
            worktree.to_str().unwrap(),
        ])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The hub lives in the main checkout, and a worker in a worktree has to reach the same
    // answer or its report goes to a different inbox.
    let from_worktree = Command::new(BIN)
        .args(["hub-name", "--json"])
        .current_dir(&worktree)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .output()
        .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&from_worktree.stdout).unwrap();
    assert_eq!(info["hubName"], HUB);
    assert_eq!(
        info["main"].as_str().unwrap(),
        fixture.repo.to_string_lossy()
    );
}

#[test]
fn config_resolves_the_flat_shorthand_and_the_machine_settings() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.json(&["config"]);
    assert_eq!(out["registered"], true);
    assert_eq!(out["repo"], "acme/widget");
    assert_eq!(out["config"]["taskSources"][0]["type"], "github");
    assert_eq!(
        out["config"]["taskSources"][0]["worktreeName"],
        "{issuekey-lowercase}-{issue}"
    );
    assert_eq!(out["settings"]["ide"], "code");
    assert_eq!(out["config"]["verify"][0], "cargo test");
    assert_eq!(out["warnings"].as_array().unwrap().len(), 0, "{out}");
}

#[test]
fn an_unregistered_repo_says_so_instead_of_failing() {
    let fixture = Fixture::new(r#"{"repos": {}}"#);
    let out = fixture.json(&["config"]);
    assert_eq!(out["registered"], false);
    assert!(out["config"].is_null());
}

#[test]
fn a_report_survives_an_absent_hub_and_can_be_read_back_and_filed() {
    let fixture = Fixture::new(QUIET);
    let sent = fixture.ok(&[
        "send",
        "--from",
        "wid-1-worker",
        "--subject",
        "検索結果の画像が縦に潰れる",
        "--body",
        "## Symptom\nthe image is squashed",
    ]);
    assert!(sent.contains(HUB), "{sent}");
    assert!(sent.contains("The hub is not running"), "{sent}");

    let listed = fixture.json(&["pending", "--json"]);
    assert_eq!(listed["count"], 1);
    let name = listed["messages"][0]["name"].as_str().unwrap().to_string();
    assert_eq!(listed["messages"][0]["from"], "wid-1-worker");
    assert_eq!(listed["messages"][0]["kind"], "report");
    assert_eq!(
        listed["messages"][0]["subject"],
        "検索結果の画像が縦に潰れる"
    );

    let body = fixture.ok(&["pending", "--read", &name]);
    assert!(body.contains("the image is squashed"), "{body}");

    fixture.ok(&["pending", "--ack", &name]);
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

#[test]
fn a_body_can_arrive_on_stdin_so_a_long_report_never_touches_the_command_line() {
    let fixture = Fixture::new(QUIET);
    let mut child = Command::new(BIN)
        .args(["send", "--from", "w", "--subject", "s"])
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"line one\nline two\n")
        .unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    let name = fixture.json(&["pending", "--json"])["messages"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        fixture
            .ok(&["pending", "--read", &name])
            .contains("line two")
    );
}

#[test]
fn an_empty_report_is_refused_rather_than_filed() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.cmd(&["send", "--body", "   "]);
    assert!(!out.status.success());
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

#[test]
fn focus_exits_one_when_no_hub_is_running() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.cmd(&["focus", "--quiet"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn the_launcher_names_the_session_and_lands_in_the_main_checkout() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.ok(&["hub", "--dry-run"]);
    assert!(
        out.contains(&fixture.repo.to_string_lossy().to_string()),
        "{out}"
    );
    assert!(out.contains(&format!("claude -n {HUB}")), "{out}");
    assert!(out.contains("--permission-mode auto"), "{out}");
    assert!(out.contains("adj-hub"), "{out}");
    // A dry run must not leave a record behind claiming a hub is up.
    assert_eq!(fixture.json(&["config"])["registered"], true);
    assert!(
        !fixture
            .state
            .join("hubs")
            .join(format!("{SLUG}.json"))
            .exists()
    );
}

const CODEX: &str = r#"{"notification": "true",
    "defaults": {"ide": "code", "agentRunner": "codex exec {prompt}"},
    "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
              "issueKeys": {"acme/widget": "WID"}}}}"#;

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
fn the_hub_leaves_a_message_in_the_worktree_and_the_worker_reads_it_there() {
    let fixture = Fixture::new(QUIET);
    let worktree = fixture.repo.to_str().unwrap().to_string();
    assert_eq!(
        fixture.ok(&["outbox", "--worktree", &worktree]).trim(),
        "(empty)"
    );

    fixture.ok(&[
        "tell",
        "--worktree",
        &worktree,
        "--subject",
        "[質問 20260908T041500Z] which screen is this about",
        "--body",
        "checking before filing WID-1",
    ]);
    let read = fixture.ok(&["outbox", "--worktree", &worktree]);
    assert!(read.contains(&format!("from {HUB}")), "{read}");
    assert!(read.contains("[質問 20260908T041500Z]"), "{read}");
    assert!(read.contains("checking before filing WID-1"), "{read}");

    // A second message appends rather than replacing the first.
    fixture.ok(&[
        "tell",
        "--worktree",
        &worktree,
        "--subject",
        "二件目",
        "--body",
        "b",
    ]);
    let both = fixture.ok(&["outbox", "--worktree", &worktree]);
    assert_eq!(both.matches("## ").count(), 2, "{both}");

    fixture.ok(&["outbox", "--worktree", &worktree, "--clear"]);
    assert_eq!(
        fixture.ok(&["outbox", "--worktree", &worktree]).trim(),
        "(empty)"
    );
}

#[test]
fn a_running_worker_gets_woken_the_same_way_a_hub_does() {
    let fixture = Fixture::new(
        r#"{"notification": "true", "workerWake": "true",
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
                      "issueKeys": {"acme/widget": "WID"}, "ide": "code"}}}"#,
    );
    let worktree = fixture.repo.to_str().unwrap().to_string();
    assert!(
        fixture
            .ok(&[
                "tell",
                "--worktree",
                &worktree,
                "--subject",
                "s",
                "--body",
                "b"
            ])
            .contains("The worker is not running")
    );

    // Register this process as the worker. A worker's command line says nothing
    // distinctive, so the record's anchor is the process start time.
    let record = fixture.repo.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    let started = Command::new("ps")
        .args(["-o", "lstart=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": std::process::id(),
            "psStarted": String::from_utf8_lossy(&started.stdout).trim(),
        })
        .to_string(),
    )
    .unwrap();
    let out = fixture.ok(&[
        "tell",
        "--worktree",
        &worktree,
        "--subject",
        "s2",
        "--body",
        "b2",
    ]);
    assert!(out.contains("Woke the worker"), "{out}");

    // A stale record — same pid, a start time from another process — must not read as alive.
    std::fs::write(
        &record,
        serde_json::json!({"pid": std::process::id(), "psStarted": "Thu Jan  1 00:00:00 1970"})
            .to_string(),
    )
    .unwrap();
    let stale = fixture.ok(&[
        "tell",
        "--worktree",
        &worktree,
        "--subject",
        "s3",
        "--body",
        "b3",
    ]);
    assert!(stale.contains("The worker is not running"), "{stale}");
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
            "#!/bin/sh\n: > '{0}'\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> '{0}'; done\n",
            recorded.display()
        ),
    )
    .unwrap();
    // A spawn template with `{cwd}` in it takes an argv, so the title and the command are
    // each meant to arrive as exactly one element.
    std::fs::write(
        &fixture.config,
        format!(
            r#"{{"notification": "true", "defaults": {{"ide": "code"}}, "repos": {{}},
                "terminal": {{"spawn": "sh '{}' {{cwd}} {{title}} {{command}}"}}}}"#,
            recorder.display()
        ),
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

#[test]
fn a_hub_that_is_running_gets_woken_and_the_sender_is_told_so() {
    // `true` stands in for whatever pokes the tab; what matters is that a present hub gets
    // the hook run and an absent one does not.
    let fixture = Fixture::new(
        r#"{"notification": "true", "hubWake": "true",
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
                      "issueKeys": {"acme/widget": "WID"}, "ide": "code"}}}"#,
    );
    assert!(
        fixture
            .ok(&["send", "--subject", "s", "--body", "b"])
            .contains("The hub is not running")
    );

    // Register this very process as the hub. Both anchors the presence check uses have to
    // be real: the name has to be in this process's command line, and the start time has to
    // be the one the system reports for it. The forged record used to carry neither — the
    // name was the *binary's* file name, which only matched because this repository's
    // directory is called `agent-adjutant`, and `psStarted` was left out entirely, so the
    // guard against a recycled pid was never once exercised by the test that covers it.
    let name = std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    let forge = |started: &str| {
        std::fs::write(
            &record,
            serde_json::json!({
                "pid": std::process::id(), "hubName": name, "cwd": "/", "psStarted": started,
            })
            .to_string(),
        )
        .unwrap();
    };
    forge(&ps_started(std::process::id()));
    let out = fixture.ok(&["send", "--subject", "s2", "--body", "b2"]);
    assert!(out.contains("Woke the hub"), "{out}");

    // Same live pid, a start time that is not its own: a pid that has been recycled onto
    // another process. Reporting that as present is the one mistake that loses a message.
    forge("Thu Jan  1 00:00:00 1970");
    let recycled = fixture.ok(&["send", "--subject", "s2b", "--body", "b2b"]);
    assert!(recycled.contains("The hub is not running"), "{recycled}");
    forge(&ps_started(std::process::id()));

    // Turning the hook off leaves the same present hub un-poked.
    std::fs::write(
        &fixture.config,
        r#"{"notification": "true", "hubWake": false,
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
                      "issueKeys": {"acme/widget": "WID"}, "ide": "code"}}}"#,
    )
    .unwrap();
    let quiet = fixture.ok(&["send", "--subject", "s3", "--body", "b3"]);
    assert!(quiet.contains("The hub is running"), "{quiet}");
}

#[test]
fn the_worktree_fallback_answers_without_any_other_tool_installed() {
    let fixture = Fixture::new(QUIET);
    let path = fixture.ok(&["worktree-path", "--branch", "someone/WID-1"]);
    assert!(
        path.trim().ends_with(".claude/worktrees/someone-WID-1"),
        "{path}"
    );

    // From a task name it answers all three at once: branch, path, and what to branch from.
    // Two commands for two halves of one decision is how the halves drift apart.
    let out: serde_json::Value = serde_json::from_str(&fixture.ok(&[
        "worktree-path",
        "--name",
        "wid-1",
        "--user",
        "someone",
    ]))
    .unwrap();
    assert_eq!(out["branch"], "someone/wid-1");
    assert_eq!(
        out["path"].as_str().unwrap(),
        format!("{}/.claude/worktrees/someone-wid-1", fixture.repo.display())
    );
    // The checkout to run `git worktree add` *in* — not what to branch from. Called `base`
    // once, and the procedure passed it where git wants a commit-ish.
    assert_eq!(
        out["main"].as_str().unwrap(),
        fixture.repo.to_string_lossy()
    );
    assert!(out.get("base").is_none(), "{out}");
}

#[test]
fn a_task_source_with_its_own_branch_shape_overrides_the_fallback() {
    let fixture = Fixture::new(QUIET);
    let out: serde_json::Value = serde_json::from_str(&fixture.ok(&[
        "worktree-path",
        "--name",
        "wid-1",
        "--user",
        "someone",
        "--pattern",
        "feature/{name}",
    ]))
    .unwrap();
    assert_eq!(out["branch"], "feature/wid-1");
}

#[test]
fn asking_for_neither_a_branch_nor_a_name_is_an_error_not_a_guess() {
    let fixture = Fixture::new(QUIET);
    assert!(!fixture.cmd(&["worktree-path"]).status.success());
}

#[test]
fn every_command_and_tool_the_procedures_name_actually_exists() {
    // The procedures are prose telling an agent which commands to run. Rename a subcommand
    // or a tool and nothing here stops compiling — the agent just gets an error at the one
    // moment it was supposed to be getting work done.
    let help = String::from_utf8(Command::new(BIN).arg("--help").output().unwrap().stdout).unwrap();
    let subcommands: Vec<String> = help
        .lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect();
    assert!(
        subcommands.len() > 10,
        "could not read the subcommand list: {help}"
    );

    let fixture = Fixture::new(QUIET);
    let listed = mcp(&fixture, &[request(1, "tools/list", serde_json::json!({}))]);
    let tools: Vec<String> = listed[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();

    let mut missing: Vec<String> = Vec::new();
    for name in ["adj-hub", "adj-worker", "adj-report"] {
        let text =
            std::fs::read_to_string(format!("{}/commands/{name}.md", env!("CARGO_MANIFEST_DIR")))
                .unwrap();
        // Only backticked mentions: `adjutant work` is a reference, "adjutant sets" is prose.
        // Both names are checked, because the procedures use them interchangeably — and a
        // check that only knew the long one stopped covering the launcher the day it was
        // renamed, without failing.
        for quoted in text.split('`').skip(1).step_by(2) {
            let Some(rest) = quoted
                .strip_prefix("adjutant")
                .or_else(|| quoted.strip_prefix("adj"))
            else {
                continue;
            };
            let referenced = match rest.chars().next() {
                Some('_') => rest.trim_start_matches('_').to_string(),
                Some(' ') => rest.split_whitespace().next().unwrap_or("").to_string(),
                // `adjutant` alone is the tool's own name, not a reference to anything.
                _ => continue,
            };
            let known = match rest.starts_with('_') {
                true => tools.contains(&format!("adjutant_{referenced}")),
                false => subcommands.contains(&referenced),
            };
            if !known && !referenced.is_empty() {
                missing.push(format!("{name}.md: adjutant {referenced}"));
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "the procedures name things that do not exist: {missing:#?}"
    );
}

#[test]
fn the_shipped_example_config_resolves_without_a_single_warning() {
    // The example is the schema documentation. A warning in it means the documentation
    // describes a config the resolver disagrees with.
    let fixture = Fixture::new(&std::fs::read_to_string("config.example.json").unwrap());
    for repo in [
        "example/android-app",
        "example/web",
        "example/service",
        "example/legacy",
    ] {
        let out = fixture.json(&["config", "--repo", repo]);
        assert_eq!(out["registered"], true, "{repo}");
        assert_eq!(
            out["warnings"].as_array().unwrap().len(),
            0,
            "{repo}: {}",
            out["warnings"]
        );
        assert!(
            !out["config"]["taskSources"].as_array().unwrap().is_empty(),
            "{repo}"
        );
    }
}

// ── the MCP server ───────────────────────────────────────────────────

/// Feed the server a batch of requests and collect one reply per line.
fn mcp(fixture: &Fixture, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut child = Command::new(BIN)
        .arg("mcp")
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for request in requests {
            writeln!(stdin, "{request}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect(l))
        .collect()
}

/// The same driver, but the input is bytes — a line that is not valid UTF-8 cannot be
/// written any other way, and that is the whole point of the test below.
fn mcp_raw(fixture: &Fixture, lines: &[&[u8]]) -> Vec<serde_json::Value> {
    let mut child = Command::new(BIN)
        .arg("mcp")
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for line in lines {
            stdin.write_all(line).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect(l))
        .collect()
}

/// What the system says about when a process started, in the same words the binary records.
fn ps_started(pid: u32) -> String {
    let out = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn request(id: u32, method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn tool_result(response: &serde_json::Value) -> serde_json::Value {
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).expect(text)
}

#[test]
fn the_server_handshakes_serves_the_procedures_and_answers_about_the_repo() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp(
        &fixture,
        &[
            request(1, "initialize", serde_json::json!({})),
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            request(2, "prompts/list", serde_json::json!({})),
            request(
                3,
                "prompts/get",
                serde_json::json!({"name": "adj-report", "arguments": {"arguments": "画像が潰れる"}}),
            ),
            request(4, "tools/list", serde_json::json!({})),
            request(
                5,
                "tools/call",
                serde_json::json!({"name": "adjutant_hub_status", "arguments": {}}),
            ),
        ],
    );
    // Five requests, one notification: the notification must not produce a sixth line.
    assert_eq!(replies.len(), 5, "{replies:#?}");

    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "adjutant");
    assert_eq!(replies[1]["result"]["prompts"].as_array().unwrap().len(), 3);
    let procedure = replies[2]["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(procedure.contains("画像が潰れる"));
    assert!(
        !procedure.starts_with("---"),
        "frontmatter leaked into the procedure"
    );
    assert_eq!(replies[3]["result"]["tools"].as_array().unwrap().len(), 7);

    let hub = tool_result(&replies[4]);
    assert_eq!(hub["hubName"], HUB);
    assert_eq!(hub["present"], false);
    assert_eq!(hub["waiting"], 0);
}

#[test]
fn a_report_sent_through_the_server_lands_where_the_cli_looks_for_it() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_send", "arguments": {
                "from": "wid-1-worker",
                "subject": "検索結果の画像が縦に潰れる",
                "body": "## Symptom\nthe image is squashed",
            }}),
        )],
    );
    let sent = tool_result(&replies[0]);
    assert_eq!(sent["present"], false);
    assert_eq!(sent["hubName"], HUB);

    // Same inbox from both directions — that is the whole point of the file-based channel.
    let listed = fixture.json(&["pending", "--json"]);
    assert_eq!(listed["count"], 1);
    assert_eq!(
        listed["messages"][0]["subject"],
        "検索結果の画像が縦に潰れる"
    );
}

#[test]
fn the_config_tool_and_the_config_subcommand_agree() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_config", "arguments": {}}),
        )],
    );
    assert_eq!(tool_result(&replies[0]), fixture.json(&["config"]));
}

/// One byte used to end the session. `lines()` reports a line that is not valid UTF-8 as an
/// error, that was read as end-of-input, and every later call in that session failed with
/// nothing said about why.
#[test]
fn one_undecodable_byte_does_not_take_the_server_down_with_it() {
    let fixture = Fixture::new(QUIET);
    let good = request(1, "ping", serde_json::json!({})).to_string();
    let after = request(2, "ping", serde_json::json!({})).to_string();
    let replies = mcp_raw(
        &fixture,
        &[
            good.as_bytes(),
            // A lone continuation byte: valid JSON is impossible here, and so is UTF-8.
            b"{\"jsonrpc\": \"2.0\", \"id\": 9, \"method\": \"\xff\"}",
            after.as_bytes(),
        ],
    );
    assert_eq!(replies.len(), 3, "{replies:#?}");
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[1]["error"]["code"], -32700);
    // The session is still there afterwards, which is the part that was broken.
    assert_eq!(replies[2]["id"], 2);
    assert!(replies[2]["result"].is_object(), "{replies:#?}");
}

/// Three ways an id can arrive, and they mean three different things.
#[test]
fn an_id_that_is_null_is_a_request_and_an_absent_one_is_not() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp_raw(
        &fixture,
        &[
            // Present and null: a request. Its answer carries `"id": null`.
            br#"{"jsonrpc": "2.0", "id": null, "method": "ping", "params": {}}"#,
            // Absent: a notification, and answering one is a protocol error.
            br#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#,
            // Unparseable: the error has to carry an id too, and null is the one the
            // specification names for "cannot be matched to a request".
            br#"{not json at all"#,
        ],
    );
    assert_eq!(replies.len(), 2, "{replies:#?}");
    assert!(replies[0]["result"].is_object(), "{replies:#?}");
    for reply in &replies {
        // Present and null — not missing. A client matching replies to requests by id has
        // nothing to work with when the field is simply absent.
        assert!(reply.as_object().unwrap().contains_key("id"), "{reply:#?}");
        assert!(reply["id"].is_null(), "{reply:#?}");
    }
    assert_eq!(replies[1]["error"]["code"], -32700);
}

/// Well-formed JSON is not the same thing as a well-formed request, and answering the
/// second kind of mistake with a *parse* error told the client to look in the wrong place.
#[test]
fn a_request_that_is_not_a_request_is_told_which_of_the_two_it_got_wrong() {
    let fixture = Fixture::new(QUIET);
    let replies = mcp_raw(
        &fixture,
        &[
            // Parses; has no method. Silence would be the answer to a notification, and
            // this is not one — a client that gets silence waits for ever.
            br#"{}"#,
            // serde will read a struct out of a sequence, so this used to be accepted and
            // answered as a call to `ping`.
            br#"["2.0", 1, "ping", {}]"#,
            // An id has to be something a reply can be matched by.
            br#"{"jsonrpc": "2.0", "id": true, "method": "ping"}"#,
            // The version is wrong, but this one *is* a request, so its id comes back.
            br#"{"jsonrpc": "1.0", "id": 8, "method": "ping"}"#,
        ],
    );
    assert_eq!(replies.len(), 4, "{replies:#?}");
    for reply in &replies {
        // The structure was wrong, not the syntax.
        assert_eq!(reply["error"]["code"], -32600, "{reply:#?}");
    }
    for reply in &replies[..3] {
        assert!(reply["id"].is_null(), "{reply:#?}");
    }
    assert_eq!(replies[3]["id"], 8);
}
