//! End-to-end tests: run the real binary against a throwaway repository and a throwaway
//! state directory.
//!
//! Everything here is hermetic on purpose. `ADJUTANT_CONFIG` and `ADJUTANT_STATE_DIR` point
//! into tempdirs, so a test can never read the developer's own config or drop a fixture
//! report into a hub that is actually running. Anything that would open a window, start an
//! agent or notify a human is exercised through `--dry-run`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
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
fn a_report_says_which_worktree_it_came_from_not_which_repository() {
    // The address a `done` request acts on. `from` is free text — the sender picks it — so
    // a hub asked to close a tab and remove a worktree was acting on a path typed into the
    // body of the message. This header is derived from where the sender actually is.
    let fixture = Fixture::new(QUIET);
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-1");
    let added = Command::new("git")
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
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );

    let sent = Command::new(BIN)
        .args([
            "send",
            "--kind",
            "done",
            "--subject",
            "終わったのだ",
            "--body",
            "b",
        ])
        .current_dir(&worktree)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .output()
        .unwrap();
    assert!(
        sent.status.success(),
        "{}",
        String::from_utf8_lossy(&sent.stderr)
    );

    let listed = fixture.json(&["pending", "--json"]);
    let message = &listed["messages"][0];
    // The worktree it was sent from, and specifically not the main checkout: answering with
    // the checkout would identify the repository, which the inbox already knew.
    assert_eq!(message["worktree"], worktree.to_string_lossy().to_string());
    assert_ne!(
        message["worktree"],
        fixture.repo.to_string_lossy().to_string()
    );
    assert_eq!(message["kind"], "done");

    let name = message["name"].as_str().unwrap().to_string();
    let read = fixture.ok(&["pending", "--read", &name]);
    assert!(
        read.contains(&format!("worktree: {}", worktree.display())),
        "{read}"
    );

    // A sender git cannot place in a worktree still gets its message delivered, and the
    // header is simply absent. A bare clone is the case that reaches this: `git worktree
    // list` answers there — so the repository, and with it the inbox, is found — while
    // `rev-parse --show-toplevel` has nothing to say.
    let bare = fixture.repo.parent().unwrap().join("bare.git");
    for args in [
        vec!["init", "-q", "--bare", bare.to_str().unwrap()],
        vec![
            "-C",
            bare.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            "git@github.com:acme/widget.git",
        ],
    ] {
        assert!(
            Command::new("git")
                .args(&args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let placeless_send = Command::new(BIN)
        .args(["send", "--subject", "s2", "--body", "b2"])
        .current_dir(&bare)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .output()
        .unwrap();
    assert!(
        placeless_send.status.success(),
        "{}",
        String::from_utf8_lossy(&placeless_send.stderr)
    );
    let listed = fixture.json(&["pending", "--json"]);
    let placeless = listed["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["subject"] == "s2")
        .unwrap();
    assert_eq!(placeless["worktree"], serde_json::Value::Null);
    let read = fixture.ok(&["pending", "--read", placeless["name"].as_str().unwrap()]);
    assert!(!read.contains("worktree:"), "{read}");

    // And an inbox that already held four-header messages when this shipped keeps working.
    let older =
        "---\nfrom: wid-2\nkind: report\nsubject: 前からあるやつ\nat: 20260908T041500Z\n---\n\nb\n";
    let dir = fixture.state.join("inbox").join(SLUG);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("20260908T041500Z-report.md"), older).unwrap();
    let listed = fixture.json(&["pending", "--json"]);
    let old = listed["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["subject"] == "前からあるやつ")
        .unwrap();
    assert_eq!(old["worktree"], serde_json::Value::Null);
    assert_eq!(old["from"], "wid-2");
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

/// A config whose close template is `command`, for this fixture's repository.
///
/// Assembled with `json!` rather than pasted into a literal: these templates carry paths,
/// and a path with a quote or a backslash in it turns a hand-escaped config file into a
/// test that fails for a reason it is not about.
fn closing_with(close: impl Into<serde_json::Value>) -> String {
    serde_json::json!({
        "notification": "true",
        "terminal": { "close": close.into() },
        "repos": { "acme/widget": {
            "taskSource": "github", "issueRepo": "acme/widget",
            "issueKeys": { "acme/widget": "WID" }, "ide": "code"
        }}
    })
    .to_string()
}

/// POSIX single-quoting, for a value going into a shell line.
///
/// The crate's own `sh_quote` is not reachable from an integration test. Quoting matters
/// here for the same reason it matters in the tool: a `TMPDIR` with a space in it is the
/// machine's business, not a defect in what is under test.
fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// A worker record for `pid`, carrying the start time the system reports for it — the
/// anchor that tells that process from whatever the pid is handed to next.
fn forge_worker_record(worktree: &Path, pid: u32) -> PathBuf {
    let record = worktree.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({"pid": pid, "title": "WID-957", "psStarted": ps_started(pid)})
            .to_string(),
    )
    .unwrap();
    record
}

/// A process standing in for a worker: it stays until something kills it, and really does
/// disappear when something does.
///
/// Not simply a `sleep` spawned by the test. That would be the test's own child, and a
/// child nobody has waited for stays a zombie once it dies — `ps -o lstart=` answers for a
/// zombie exactly as it answers for a live process, so a test that killed one would be
/// asserting that `close` sees a death at the one moment the system still reports life, and
/// would pass against an implementation that never checked at all.
///
/// So it is a *grandchild*: spawned under a shell that then sits in `wait`, which gives it
/// a live parent to reap it the instant it goes. `ps` says "no such process" from then on.
///
/// It blocks on a pipe this test holds rather than sleeping for a while, so nothing here
/// depends on a stand-in outliving the rest of the test — a timeout would be a second way
/// for a correct implementation to fail, on a slow enough machine. Closing the pipe is also
/// what cleans up after a test run that was killed outright: the read reaches end of file
/// and the process exits on its own. `cat <&3` rather than plain `cat` because a shell
/// points a background job's stdin at `/dev/null` unless the redirect is written out, and
/// `cat` on `/dev/null` is a stand-in that exits immediately.
struct Sleeper {
    shell: std::process::Child,
    /// `None` only while `new` is still assembling one. Armed early so that a panic during
    /// construction still runs `Drop`: a value that never finished being built is never
    /// dropped, and both processes would be left behind.
    pid: Option<u32>,
}

impl Sleeper {
    fn new() -> Sleeper {
        let shell = Command::new("sh")
            .args(["-c", "exec 3<&0; cat <&3 >/dev/null & echo $!; wait"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The shell announces the kill on stderr, which is this test's own doing and
            // not something a reader of the suite's output should have to explain.
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut sleeper = Sleeper { shell, pid: None };
        let mut line = String::new();
        std::io::BufReader::new(sleeper.shell.stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        sleeper.pid = Some(line.trim().parse().unwrap());
        assert!(
            !ps_started(sleeper.pid()).is_empty(),
            "the stand-in worker was not running to begin with"
        );
        sleeper
    }

    fn pid(&self) -> u32 {
        self.pid.expect("the stand-in worker has no pid")
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        // The worker first, and by pid: it is the shell's background child, so killing the
        // shell would leave it running with nobody to reap it. Killing it while the shell
        // still waits is what gets it reaped, which is the whole point of the shape.
        if let Some(pid) = self.pid {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        // Then the shell, which is on its way out of `wait` anyway.
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }
}

#[test]
fn closing_a_worktree_nobody_is_working_in_succeeds_and_says_so() {
    // The hub calls this on its way to `git worktree remove`, so arriving twice — or
    // arriving after the worker stopped on its own — has to be a success. Made an error, a
    // cleanup that is already half done can never be finished.
    let fixture = Fixture::new(QUIET);
    let never_existed = fixture.repo.join("worktrees").join("wid-1");
    let out = fixture.ok(&["close", "--worktree", never_existed.to_str().unwrap()]);
    assert!(out.contains("no worker is running"), "{out}");

    // A record whose process is long gone gets the same answer, and the record goes with
    // it: left there, the next reader is told a worker is present in a worktree that has
    // none.
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = fixture.repo.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({"pid": std::process::id(), "title": "WID-957",
                           "psStarted": "Thu Jan  1 00:00:00 1970"})
        .to_string(),
    )
    .unwrap();
    let stale = fixture.ok(&["close", "--worktree", &worktree]);
    assert!(stale.contains("no worker is running"), "{stale}");
    assert!(!record.exists(), "the record left behind survived");

    assert_eq!(
        fixture.ok(&["close", "--worktree", &worktree, "--quiet"]),
        ""
    );

    // A record that names nobody is *not* the same as no record. It is a file somebody
    // wrote, in a worktree somebody may be working in, that this cannot read — and reading
    // it as "free" is the fail-open this command exists to avoid.
    for content in ["{ not json", "{}", r#"{"pid": null}"#] {
        std::fs::write(&record, content).unwrap();
        let out = fixture.cmd(&["close", "--worktree", &worktree]);
        let said = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            !out.status.success(),
            "{content} was read as a free worktree"
        );
        assert!(said.contains("cannot be read as naming a worker"), "{said}");
        assert!(record.exists(), "{content} was cleared away");
    }
}

#[test]
fn closing_a_running_worker_shows_the_command_first_and_takes_the_record_with_it() {
    // The close template stands in for whatever disposes of a tab, the way the wake tests
    // stand in for whatever pokes one.
    let fixture = Fixture::new(&closing_with("true --pid {pid} --tty {tty}"));
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = forge_worker_record(&fixture.repo, worker.pid());

    // A dry run shows the command and runs nothing, and still answers about the worktree:
    // the procedures chain this into `&& git worktree remove`.
    let planned = fixture.cmd(&["close", "--worktree", &worktree, "--dry-run"]);
    let planned_out = String::from_utf8_lossy(&planned.stdout).to_string();
    assert!(
        planned_out.contains(&format!("--pid {}", worker.pid())),
        "{planned_out}"
    );
    assert!(
        !planned.status.success(),
        "a dry run said the worktree is free"
    );
    assert!(record.exists(), "a dry run cleared the record");

    // A close that failed is not a closed tab. The caller is on its way to `git worktree
    // remove`, so this has to be a non-zero exit and the record has to stay.
    std::fs::write(&fixture.config, closing_with("false")).unwrap();
    let refused = fixture.cmd(&["close", "--worktree", &worktree]);
    assert!(!refused.status.success());
    assert!(record.exists(), "a tab that never closed lost its record");
    // The premise of the phase below, and of the two above it: nothing so far was supposed
    // to touch the worker, and a stand-in that had already died would make the rest of this
    // test pass for the wrong reason.
    assert!(
        !ps_started(worker.pid()).is_empty(),
        "the stand-in worker died before the close that is meant to kill it"
    );

    // And the whole way through. The template kills the process the way closing its tab
    // would, and only then is the worktree reported free and the record cleared.
    std::fs::write(&fixture.config, closing_with("kill {pid}")).unwrap();
    let done = fixture.ok(&["close", "--worktree", &worktree]);
    assert!(done.contains("closed the tab"), "{done}");
    assert!(!record.exists(), "the record outlived the tab it named");
    assert!(
        ps_started(worker.pid()).is_empty(),
        "close reported a death the system disagrees with"
    );
}

#[test]
fn a_close_that_leaves_the_worker_running_clears_nothing_and_says_so() {
    // What the close command reports is not what the caller needs to know: a cancelled
    // confirmation dialog and a template resolved to the wrong pane both report a close
    // having disposed of nothing. `true` is exactly that command.
    let fixture = Fixture::new(&closing_with("true"));
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = forge_worker_record(&fixture.repo, worker.pid());

    let out = fixture.cmd(&["close", "--worktree", &worktree]);
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "a live worker was reported as cleared away: {said}"
    );
    assert!(
        said.contains(&format!("pid {} is still there", worker.pid())),
        "{said}"
    );
    assert!(record.exists(), "the record of a live worker was cleared");
    assert!(
        !ps_started(worker.pid()).is_empty(),
        "the stand-in worker died"
    );
}

#[test]
fn a_record_that_vanished_is_not_evidence_the_worker_died() {
    // The record is a note about a process, not the process. This close command removes the
    // note and leaves the worker alone, which is what a template pointed at the wrong thing
    // does from here. Ask the *record* whether the worker went, and it answers "gone".
    let fixture = Fixture::new(QUIET);
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = forge_worker_record(&fixture.repo, worker.pid());
    std::fs::write(
        &fixture.config,
        closing_with(format!("rm -f {}", shell_quoted(&record.to_string_lossy()))),
    )
    .unwrap();

    let out = fixture.cmd(&["close", "--worktree", &worktree]);
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "a worktree whose record was deleted was called free: {said}"
    );
    assert!(
        said.contains(&format!("pid {} is still there", worker.pid())),
        "{said}"
    );
    assert!(
        !ps_started(worker.pid()).is_empty(),
        "the stand-in worker died"
    );
    // The premise. Without it, a close template that quietly stopped removing the record
    // would leave this test indistinguishable from the one above it: still passing, and no
    // longer about anything.
    assert!(
        !record.exists(),
        "the close command did not remove the record this test is about"
    );
}

#[test]
fn a_record_with_no_start_time_is_not_acted_on() {
    // `register_worker` writes `psStarted: null` when `ps` would not answer at that moment.
    // Without it there is nothing to tell this worker from the next process to be handed
    // that pid — and a tab is closed on the answer, so "the number is in use, close it" is
    // not good enough.
    let fixture = Fixture::new(&closing_with("true"));
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = fixture.repo.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    for named in [
        serde_json::json!({"pid": worker.pid(), "title": "WID-957", "psStarted": null}),
        serde_json::json!({"pid": worker.pid(), "title": "WID-957"}),
    ] {
        std::fs::write(&record, named.to_string()).unwrap();
        let out = fixture.cmd(&["close", "--worktree", &worktree]);
        let said = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(!out.status.success(), "{named} was acted on: {said}");
        assert!(
            said.contains(&format!("cannot tell whether pid {}", worker.pid())),
            "{said}"
        );
        assert!(record.exists(), "{named} was cleared away");
    }
}

#[test]
fn closing_can_be_turned_off_and_then_nothing_is_cleared() {
    // The config's promise for every one of these keys is that `false` turns the behaviour
    // off, which is a different answer from leaving it out. Read as unset, `"close": false`
    // would reach the built-in closer and dispose of the tab it was meant to protect.
    //
    // And with nothing closing tabs, nothing may report a worktree as finished with: the
    // person who turned this off is doing the closing by hand.
    let fixture = Fixture::new(&closing_with(false));
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = forge_worker_record(&fixture.repo, worker.pid());

    let out = fixture.cmd(&["close", "--worktree", &worktree]);
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "a worktree nobody closed was called free: {said}"
    );
    assert!(said.contains("turned off"), "{said}");
    assert!(
        record.exists(),
        "the record was cleared without a tab being closed"
    );
    assert!(
        !ps_started(worker.pid()).is_empty(),
        "the worker was closed anyway"
    );
}

#[test]
fn a_record_naming_another_worker_is_left_where_it_is() {
    // The window between seeing a worker go and clearing its record: a new worker registers
    // in the same worktree in between. Clearing the record then reports a free worktree
    // about somebody who has only just started, and the next call agrees with it, because
    // by then there is no record at all.
    //
    // The template does both halves in the order that hurts: it kills the worker being
    // closed and registers the newcomer before `close` gets to look again.
    let fixture = Fixture::new(QUIET);
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = forge_worker_record(&fixture.repo, worker.pid());
    let newcomer = serde_json::json!({
        "pid": std::process::id(), "title": "WID-958",
        "psStarted": ps_started(std::process::id()),
    })
    .to_string();
    std::fs::write(
        &fixture.config,
        closing_with(format!(
            "kill {{pid}} && printf %s {} > {}",
            shell_quoted(&newcomer),
            shell_quoted(&record.to_string_lossy())
        )),
    )
    .unwrap();

    let out = fixture.cmd(&["close", "--worktree", &worktree]);
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "the newcomer's worktree was reported free: {said}"
    );
    assert!(said.contains("another worker has registered"), "{said}");
    // Both halves of the premise: the worker being closed really went, and what was left
    // behind really is the newcomer's record.
    assert!(
        ps_started(worker.pid()).is_empty(),
        "the worker this test kills is still running"
    );
    let left = std::fs::read_to_string(&record).unwrap();
    assert!(
        left.contains(&std::process::id().to_string()),
        "the newcomer's record was cleared: {left}"
    );
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
    // With no `cwd` given, the sender is where the server is standing.
    assert_eq!(
        listed["messages"][0]["worktree"],
        fixture.repo.to_string_lossy().to_string()
    );

    // And with one, it is where the *caller* is standing. This is the case that matters:
    // one server is started per session and then asked about whichever worktree the worker
    // is working in, so the server's own directory is nobody's address.
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-2");
    let added = Command::new("git")
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "wid-2",
            worktree.to_str().unwrap(),
        ])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_send", "arguments": {
                "from": "wid-2-worker",
                "kind": "done",
                "subject": "終わったのだ",
                "body": "b",
                "cwd": worktree.to_string_lossy(),
            }}),
        )],
    );
    let listed = fixture.json(&["pending", "--json"]);
    let done = listed["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "done")
        .unwrap();
    assert_eq!(done["worktree"], worktree.to_string_lossy().to_string());
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
