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

/// The same repository, addressed as one hub of it rather than as itself. Written out for
/// the same reason, and load-bearing for a second one: these two constants differing is
/// what a separate inbox *is*.
const FEATURE: &str = "wid-957";
const FEATURE_SLUG: &str = "acme-widget-wid-957-5283c95d4f4cc314";
const FEATURE_HUB: &str = "adjutant-acme-widget-wid-957-5283c95d4f4cc314";

/// Everything a child of this suite must not inherit from whatever ran `cargo test`.
///
/// That is regularly a tab which *is* a hub, and a hub exports its own answers:
/// `ADJUTANT_HUB` re-addresses every inbox asserted on here, and — since `adj hub
/// --no-dashboard` sets it — `ADJUTANT_STARTUP_DASHBOARD` outranks the `startupDashboard` a
/// fixture has just written into its own config file.
const AMBIENT: [&str; 3] = [
    "ADJUTANT_HUB",
    "ADJUTANT_STARTUP_DASHBOARD",
    // A hub's MCP server beats for the session this names; a test child that inherited it
    // would be beating for the hub `cargo test` was typed in.
    "ADJUTANT_HUB_SESSION",
];

/// Strip `AMBIENT` from a child about to be run.
///
/// One list in one place, because the alternative is what this replaced: the rule had
/// reached thirteen builders by being copied, so when a second variable joined it, it was
/// added to one of them. The other twelve kept inheriting it, and the failure that surfaced
/// blamed neither — `the_config_tool_and_the_config_subcommand_agree` compares a sanitised
/// child against an unsanitised one, so the two resolved the same config to different
/// answers and reported a disagreement between the CLI and the MCP server. A variable added
/// to this array reaches every child at once; one added to a call site reaches one.
///
/// Applied before any deliberate `.env(…)`, so a test that means to hand a child one of
/// these still can. Sanitising first is what makes such a value the test's own rather than
/// the terminal's.
trait Ambient {
    fn hermetic(&mut self) -> &mut Self;
}

impl Ambient for Command {
    fn hermetic(&mut self) -> &mut Self {
        for name in AMBIENT {
            self.env_remove(name);
        }
        self
    }
}

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
            .hermetic()
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
        .hermetic()
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
        .hermetic()
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
        .hermetic()
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
        .hermetic()
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

/// The compatibility lock, from the outside.
///
/// There are hub records, inboxes and archives sitting in `~/.local/state/adjutant` right
/// now, filed under the address a plain `adj` produced before any of this existed. An
/// upgrade that moved that address would leave every running hub unreachable and every
/// queued report unread, with nothing anywhere saying so — so the no-identifier answer has
/// to be the same bytes it always was, and the command line the launcher prints has to be
/// the same line it always printed.
#[test]
fn naming_no_hub_addresses_exactly_what_it_addressed_before() {
    let fixture = Fixture::new(QUIET);
    let info = fixture.json(&["hub-name", "--json"]);
    assert_eq!(info["hubName"], HUB);
    assert_eq!(info["slug"], SLUG);
    // Null rather than a string: nothing was asked for, and the repository's own hub is
    // not an identifier anybody typed.
    assert!(info["hub"].is_null(), "{info}");

    // The launcher's line gains no identifier. A hub that is the repository's own has none
    // to hand down, and an `env ADJUTANT_HUB=` prefix appearing here would be a change to a
    // command line people read, script and paste. (`ADJUTANT_HUB_SESSION` is a different
    // variable, and every hub carries one.)
    let launch = fixture.ok(&["hub", "--dry-run"]);
    assert!(!launch.contains("ADJUTANT_HUB="), "{launch}");
    assert!(launch.contains(&format!("claude -n {HUB}")), "{launch}");

    // Neither does the line that starts a worker.
    let work = fixture.ok(&[
        "work",
        "--worktree",
        fixture.repo.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(!work.contains("--hub"), "{work}");
}

/// The point of the whole change: a second hub is a second *address*, not a second
/// repository.
///
/// Before this, the only way to ask for one was to invent a repository name — which moved
/// the address and emptied the configuration at the same time, because the config has no
/// entry for a repository that does not exist. The settings have to keep coming from
/// `owner/name` while the inbox moves.
#[test]
fn a_hub_identifier_moves_the_address_without_moving_the_configuration() {
    let fixture = Fixture::new(QUIET);
    let info = fixture.json(&["hub-name", "--hub", FEATURE, "--json"]);
    assert_eq!(info["hub"], FEATURE);
    assert_eq!(info["hubName"], FEATURE_HUB);
    assert_eq!(info["slug"], FEATURE_SLUG);
    assert_ne!(info["hubName"], HUB);
    // Still visibly this repository, because that is what a person picks out of a listing
    // of the state directory.
    assert_eq!(info["repo"], "widget");
    assert_eq!(info["nwo"], "acme/widget");

    // The half that must not move. `repos` is keyed by `owner/name`, and it is still
    // looked up by `owner/name`.
    let config = fixture.json(&["config", "--hub", FEATURE]);
    assert_eq!(config["registered"], true);
    assert_eq!(config["repo"], "acme/widget");
    assert_eq!(config["hub"], FEATURE);
    assert_eq!(config["hubName"], FEATURE_HUB);
    assert_eq!(config["config"]["taskSources"][0]["type"], "github");
    assert_eq!(config["config"]["issueKeys"]["acme/widget"], "WID");
    assert_eq!(config["config"]["verify"][0], "cargo test");

    // The half that must move: a report filed against one is not waiting in the other's
    // inbox. Both directories are asked for by path first, so a shared one fails here
    // rather than in the count below.
    let plain_dir = fixture.ok(&["pending", "--path"]).trim().to_string();
    let feature_dir = fixture
        .ok(&["pending", "--path", "--hub", FEATURE])
        .trim()
        .to_string();
    assert_ne!(plain_dir, feature_dir);
    assert!(feature_dir.contains(FEATURE_SLUG), "{feature_dir}");

    let sent = fixture.ok(&[
        "send",
        "--hub",
        FEATURE,
        "--from",
        "wid-957-worker",
        "--subject",
        "検索結果の画像が縦に潰れる",
        "--body",
        "b",
    ]);
    assert!(sent.contains(FEATURE_HUB), "{sent}");
    assert_eq!(
        fixture.json(&["pending", "--json", "--hub", FEATURE])["count"],
        1
    );
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);

    // And a third identifier is a third address, not a shared one.
    assert_ne!(
        fixture.json(&["hub-name", "--hub", "wid-958", "--json"])["hubName"],
        info["hubName"]
    );
}

/// How a hub's own agent comes to know which hub it is.
///
/// Its procedure calls `adjutant_pending` and the rest with no arguments — it is talking
/// about itself. The identifier therefore has to reach those calls without any of them
/// mentioning it, and the environment is the one channel that does: the launcher puts it on
/// the line it `exec`s, the agent inherits it, and the MCP server the agent starts is that
/// agent's own child.
#[test]
fn the_launcher_hands_its_identifier_down_to_the_agent_it_starts() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.ok(&["hub", "--hub", FEATURE, "--dry-run"]);
    assert!(out.contains(&format!("ADJUTANT_HUB={FEATURE}")), "{out}");
    // The session is named for the address it answers at, or a worker looking for it finds
    // the repository's own hub instead.
    assert!(out.contains(&format!("claude -n {FEATURE_HUB}")), "{out}");
    assert!(
        out.contains(&fixture.repo.to_string_lossy().to_string()),
        "{out}"
    );

    // And the other way round: a command run *by* that agent, with no flag, addresses the
    // hub that started it. This is every `adj` call the hub's procedure makes.
    let inherited = Command::new(BIN)
        .args(["hub-name", "--json"])
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .hermetic()
        .env("ADJUTANT_HUB", FEATURE)
        .output()
        .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&inherited.stdout).unwrap();
    assert_eq!(info["hubName"], FEATURE_HUB);
    assert_eq!(info["hub"], FEATURE);
}

/// A worker is dispatched with the hub that dispatched it, and finds it again without
/// being told.
///
/// The identifier cannot travel the way the hub's own does: the tab is opened by the
/// terminal, which is handed a command line and nothing else, so an inherited environment
/// does not survive the trip. It goes onto the command line instead, and from there into
/// the worktree — which is what lets `adj-report` keep promising that a worker never writes
/// an address down.
#[test]
fn a_worker_carries_the_hub_that_dispatched_it_into_its_worktree() {
    let fixture = Fixture::new(CODEX);
    let out = fixture.ok(&[
        "work",
        "--hub",
        FEATURE,
        "--worktree",
        fixture.repo.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(out.contains(&format!("--hub={FEATURE}")), "{out}");

    // A hub dispatching work runs this as its own child and passes no flag at all — it is
    // carrying the answer in its environment. The resolved identifier is what goes on the
    // line, not the flag, or exactly the hub that most needs this loses it.
    let inherited = Command::new(BIN)
        .args([
            "work",
            "--worktree",
            fixture.repo.to_str().unwrap(),
            "--dry-run",
        ])
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .hermetic()
        .env("ADJUTANT_HUB", FEATURE)
        .output()
        .unwrap();
    let line = String::from_utf8_lossy(&inherited.stdout);
    assert!(line.contains(&format!("--hub={FEATURE}")), "{line}");

    // The far end of that trip. `adj worker` writes the identifier into the worktree
    // before it becomes the agent; forged here rather than run, because running it would
    // `exec` an agent over this test.
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-957");
    let added = Command::new("git")
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            FEATURE,
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
    let record = worktree.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({"pid": 1, "title": "WID-957", "hub": FEATURE}).to_string(),
    )
    .unwrap();

    // No flag, no environment: the worker's agent says nothing about where it is sending,
    // and the report still reaches the hub that opened this worktree.
    let sent = Command::new(BIN)
        .args(["send", "--subject", "s", "--body", "b"])
        .current_dir(&worktree)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .hermetic()
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&sent.stdout);
    assert!(
        sent.status.success(),
        "{}",
        String::from_utf8_lossy(&sent.stderr)
    );
    assert!(said.contains(FEATURE_HUB), "{said}");
    assert_eq!(
        fixture.json(&["pending", "--json", "--hub", FEATURE])["count"],
        1
    );
    // And nothing landed in the repository's own inbox, which is the failure this whole
    // arrangement exists to prevent.
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

/// A record left in a worktree says who dispatched *that* worktree. It must never be read
/// by the side that starts or registers a hub.
///
/// A worker that crashed without being closed leaves its record behind. Re-dispatching that
/// task under the repository's own hub would then file the new worker under the hub that ran
/// the old one, and every report it ever sends would go to an inbox that may have no hub
/// reading it — the silent misroute, arrived at from the other direction.
#[test]
fn a_record_left_in_a_worktree_never_decides_which_hub_is_being_started() {
    let fixture = Fixture::new(CODEX);
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-957");
    let added = Command::new("git")
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            FEATURE,
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
    let record = worktree.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({"pid": 1, "title": "WID-957", "hub": FEATURE}).to_string(),
    )
    .unwrap();

    let from_worktree = |args: &[&str]| {
        let out = Command::new(BIN)
            .args(args)
            .current_dir(&worktree)
            .env("ADJUTANT_CONFIG", &fixture.config)
            .env("ADJUTANT_STATE_DIR", &fixture.state)
            .hermetic()
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "adjutant {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    };

    // `adj hub` is run from anywhere in the repository, worktrees included. Typed here it
    // still starts the repository's own hub.
    let launch = from_worktree(&["hub", "--dry-run"]);
    assert!(launch.contains(&format!("claude -n {HUB}")), "{launch}");
    assert!(!launch.contains("ADJUTANT_HUB="), "{launch}");
    assert!(!launch.contains(FEATURE_HUB), "{launch}");

    // And the dispatching pair hands on its own identity rather than the worktree's. `adj
    // worker` matters most: its tab is opened *at* the worktree, so it would be reading the
    // record it is a moment from overwriting.
    let dispatched = from_worktree(&[
        "work",
        "--worktree",
        worktree.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(!dispatched.contains("--hub"), "{dispatched}");

    // The other side of the asymmetry, and the reason it is not simply "ignore the record":
    // a command that *addresses* a hub from in here still reaches the one that dispatched
    // this worktree, which is what a worker's report depends on.
    let addressed: serde_json::Value =
        serde_json::from_str(&from_worktree(&["hub-name", "--json"])).unwrap();
    assert_eq!(addressed["hub"], FEATURE);
    assert_eq!(addressed["hubName"], FEATURE_HUB);
}

/// A record that cannot be read is not a record that says nothing.
///
/// Reading it as "nobody dispatched this worktree" addresses the repository's own hub, and
/// a worker whose worktree *was* dispatched then files every report it ever writes into an
/// inbox that may have no hub reading it. Both ends refuse instead: the CLI the worker's
/// agent types, and the MCP tools it calls.
#[test]
fn a_record_that_cannot_be_read_refuses_to_guess_which_hub_to_address() {
    let fixture = Fixture::new(QUIET);
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-957");
    let added = Command::new("git")
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            FEATURE,
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
    let record = worktree.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    // Cut off mid-string: a write that was interrupted leaves exactly this.
    std::fs::write(&record, r#"{"pid": 1, "title": "WID-957", "hub": "wid-9"#).unwrap();

    let from_worktree = |args: &[&str]| {
        Command::new(BIN)
            .args(args)
            .current_dir(&worktree)
            .env("ADJUTANT_CONFIG", &fixture.config)
            .env("ADJUTANT_STATE_DIR", &fixture.state)
            .hermetic()
            .output()
            .unwrap()
    };

    let refused = from_worktree(&["send", "--subject", "s", "--body", "b"]);
    assert_eq!(refused.status.code(), Some(1));
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(said.contains("worker record"), "{said}");
    // The assertion the fix is for: not filed under the repository's own hub instead.
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);

    // The same refusal on the way in through the server, which is how a worker's agent
    // actually sends. `cwd` and not the server's own directory: one server answers about
    // whichever checkout the session is sitting in.
    let replies = mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_send", "arguments": {
                "from": "wid-957-worker",
                "subject": "s",
                "body": "b",
                "cwd": worktree.to_string_lossy(),
            }}),
        )],
    );
    assert_eq!(replies[0]["result"]["isError"], true);
    let text = replies[0]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("worker record"), "{text}");
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);

    // And the way out, which is why this is an error rather than a dead end: what the
    // caller says outright is settled before the record is opened at all.
    let sent = from_worktree(&["send", "--hub", FEATURE, "--subject", "s", "--body", "b"]);
    assert!(
        sent.status.success(),
        "{}",
        String::from_utf8_lossy(&sent.stderr)
    );
    assert_eq!(
        fixture.json(&["pending", "--json", "--hub", FEATURE])["count"],
        1
    );

    // A command that never asks which hub is not stopped by the same record. `ide` and
    // `worktree-path` read the settings and the checkout and nothing else, and neither
    // takes `--hub` — so routing them through the addressing path would strand them with
    // advice they cannot take, on exactly the worktree somebody is trying to open.
    for args in [
        vec!["ide", "--worktree", worktree.to_str().unwrap(), "--dry-run"],
        vec!["worktree-path", "--name", "wid-957"],
    ] {
        let ran = from_worktree(&args);
        assert!(
            ran.status.success(),
            "{} stopped on a record it never reads: {}",
            args[0],
            String::from_utf8_lossy(&ran.stderr)
        );
    }
}

/// Every flag `work` hands the tab keeps its own description, and `--hub` did not steal one.
///
/// `--hub` was inserted between a description and the flag it described, so clap read the
/// pair as one doc comment on `--hub` and left the flag below it with none. Nothing fails
/// at runtime; the help text just stops explaining two flags.
#[test]
fn every_flag_the_help_lists_describes_itself_and_not_its_neighbour() {
    let fixture = Fixture::new(QUIET);
    let line = |help: &str, flag: &str| {
        help.lines()
            .find(|l| l.contains(flag))
            .unwrap_or_else(|| panic!("no {flag} in:\n{help}"))
            .to_string()
    };

    let work = fixture.ok(&["work", "--help"]);
    assert!(
        line(&work, "--hub <HUB>").contains("Which hub of the repository"),
        "{work}"
    );
    assert!(
        !line(&work, "--hub <HUB>").contains("What the worker is told"),
        "{work}"
    );
    assert!(
        line(&work, "--prompt <PROMPT>").contains("What the worker is told"),
        "{work}"
    );

    let focus = fixture.ok(&["focus", "--help"]);
    assert!(
        line(&focus, "--hub <HUB>").contains("Which hub of the repository"),
        "{focus}"
    );
    assert!(
        !line(&focus, "--hub <HUB>").contains("Say nothing"),
        "{focus}"
    );
    assert!(line(&focus, "--quiet").contains("Say nothing"), "{focus}");

    // And the help says which of the two answers the flag defaults to. A command that
    // *starts* something resolves the identifier without opening a worktree's record — a
    // hub launched from inside a worktree, and a worker registering in the one its tab was
    // opened at, would each read somebody else's. Promising the record there is a promise
    // the code deliberately breaks, and it is the direction of this change that it breaks.
    for command in ["hub", "work", "worker"] {
        let help = fixture.ok(&[command, "--help"]);
        assert!(
            !line(&help, "--hub <HUB>").contains("dispatched this worktree"),
            "{command} offers the worktree's record as its default, and never reads it:\n{help}"
        );
    }
    for command in [
        "send", "pending", "tell", "focus", "hub-stop", "hub-name", "config",
    ] {
        let help = fixture.ok(&[command, "--help"]);
        assert!(
            line(&help, "--hub <HUB>").contains("dispatched this worktree"),
            "{command} reads the worktree's record and does not say so:\n{help}"
        );
    }
}

/// An identifier that starts with a dash still reaches the worker.
///
/// It gets this far only through `ADJUTANT_HUB`, where no flag parser has seen it, and the
/// command line the terminal is handed has none between it and the worker either. As two
/// words, `--hub -x` reads as two options and the tab never starts; as one, it is a value.
#[test]
fn an_identifier_that_reads_like_a_flag_is_handed_down_as_one_argument() {
    let fixture = Fixture::new(CODEX);
    let inherited = Command::new(BIN)
        .args([
            "work",
            "--worktree",
            fixture.repo.to_str().unwrap(),
            "--dry-run",
        ])
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .hermetic()
        .env("ADJUTANT_HUB", "-x")
        .output()
        .unwrap();
    let line = String::from_utf8_lossy(&inherited.stdout);
    assert!(line.contains("--hub=-x"), "{line}");
    // And that spelling is one the far end takes: the same token, parsed.
    assert_eq!(
        fixture.json(&["hub-name", "--json", "--hub=-x"])["hub"],
        "-x"
    );
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

/// Nothing could open a tab for a hub, so only a person at an empty one could start it.
///
/// `--tab` is the route for a caller that is not sitting in a spare tab — the same command,
/// opened rather than exec'd. Without it, `hub` is exactly what it was: this tab becomes the
/// hub, and no terminal is opened at all.
#[test]
fn a_hub_is_opened_in_a_new_tab_only_when_asked_for_one() {
    let fixture = Fixture::new(QUIET);

    // The default. It prints the agent command to run here, and never a command that would
    // open something: `adj hub` appearing in this output would mean the tab route leaked
    // into the one people already use.
    let here = fixture.ok(&["hub", "--dry-run"]);
    assert!(here.contains(&format!("claude -n {HUB}")), "{here}");
    assert!(!here.contains("/adjutant hub"), "{here}");

    // And the new one. The tab is handed the launcher, not the agent, for the same reason
    // `work` hands a tab `adjutant worker`: whoever ends up being the hub has to be the
    // process that wrote down its own PID.
    let tab = fixture.ok(&["hub", "--tab", "--dry-run"]);
    assert!(tab.contains("/adjutant hub"), "{tab}");
    assert!(!tab.contains(&format!("claude -n {HUB}")), "{tab}");
    // In the main checkout, which is where a hub has to be to cut a worktree at all.
    assert!(
        tab.contains(&fixture.repo.to_string_lossy().to_string()),
        "{tab}"
    );
    // The repository's own hub has no identifier to hand down, so nothing is added.
    assert!(!tab.contains("--hub"), "{tab}");
}

/// The side that opens the tab claims nothing.
///
/// A claim records the claiming process's PID, and the process that is going to be the hub
/// is the one in the new tab. Claiming here would name a launcher that exits a moment later,
/// and from then on every check would call a live hub gone and start another beside it.
///
/// Run for real rather than dry, because a dry run claims nothing either way and so could
/// not tell the two apart. The terminal is a stub that only writes down what it was handed.
#[test]
fn opening_a_tab_for_a_hub_leaves_the_claim_to_the_tab() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    // `{cwd}` is what tells `spawn` this template takes arguments rather than a shell line,
    // so `{command}` arrives as words for `echo` instead of a `cd … && …` chain that would
    // start a real hub inside the test suite.
    //
    // `hubRunner` is a no-op for the same reason it is here at all: the tab route never
    // reads it, so under test it changes nothing — but a regression that dropped the route
    // would fall through to the exec path, and this is what stops that from starting a real
    // agent inside the suite instead of failing.
    // Built with `json!` rather than by formatting a string: the redirect target has to be
    // shell-quoted, and a quoted path lands inside a JSON string — so the escaping of the two
    // has to be done by something that knows which is which.
    std::fs::write(
        &fixture.config,
        serde_json::json!({
            "notification": "true",
            "hubRunner": "true {name} {prompt}",
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

    let out = fixture.ok(&["hub", "--tab"]);
    assert!(out.contains("new tab"), "{out}");
    let handed = std::fs::read_to_string(&spawned).expect("the terminal was never asked");
    assert!(handed.contains("/adjutant hub"), "{handed}");
    assert!(
        handed.contains(&fixture.repo.to_string_lossy().to_string()),
        "{handed}"
    );
    // And the tab is handed the command *without* the flag, or it would open a tab of its
    // own, and that one another, and nothing would ever claim anything.
    assert!(!handed.contains("--tab"), "{handed}");

    // The record belongs to whoever ends up running the agent, and that is nobody yet.
    assert!(
        !fixture
            .state
            .join("hubs")
            .join(format!("{SLUG}.json"))
            .exists(),
        "the opening side claimed the hub: {handed}"
    );
}

/// One hub per address, whichever route is taken.
///
/// The presence check comes first and answers both routes the same way: bring the tab that
/// exists forward. Opening a second one would leave two sessions answering to one name, with
/// the record naming whichever of them claimed last.
#[test]
fn a_hub_that_is_already_running_is_brought_forward_rather_than_opened_again() {
    let fixture = Fixture::new(QUIET);
    // This very process stands in for the running hub, the same way the wake test does it:
    // both anchors have to be real, so the recorded name is this executable's and the start
    // time is the one the system reports for it.
    let name = std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    let written = serde_json::json!({
        "pid": std::process::id(), "hubName": name, "cwd": "/",
        "psStarted": ps_started(std::process::id()),
    })
    .to_string();
    std::fs::write(&record, &written).unwrap();

    let out = fixture.ok(&["hub", "--tab", "--dry-run"]);
    assert!(out.contains("is already running"), "{out}");
    assert!(!out.contains("/adjutant hub"), "{out}");
    // And the record it found is the record it leaves.
    assert_eq!(std::fs::read_to_string(&record).unwrap(), written);
}

/// The record is looked for where the hub keeps it, not where the command was typed.
///
/// A relative `ADJUTANT_STATE_DIR` resolves against the working directory, so a look taken
/// before the move to the main checkout reads a directory that holds nothing. The claim used
/// to cover for that by answering `Taken` a few lines later; the tab route never reaches the
/// claim, so a missed record there opens a tab for a hub that is already running.
#[test]
fn a_relative_state_directory_is_read_from_the_checkout_not_from_where_it_was_typed() {
    let fixture = Fixture::new(QUIET);
    let name = std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    // Relative, and under the main checkout — which is the only place it resolves to the
    // same directory twice.
    let relative = "state-here";
    let record = fixture
        .repo
        .join(relative)
        .join("hubs")
        .join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": std::process::id(), "hubName": name, "cwd": "/",
            "psStarted": ps_started(std::process::id()),
        })
        .to_string(),
    )
    .unwrap();

    // Typed somewhere else inside the checkout, so a look taken before the move lands in a
    // directory that was never written to.
    let elsewhere = fixture.repo.join("somewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let out = Command::new(BIN)
        .args(["hub", "--tab", "--dry-run"])
        .current_dir(&elsewhere)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", relative)
        .hermetic()
        .output()
        .unwrap();
    let said =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("is already running"), "{said}");
    assert!(!said.contains("/adjutant hub"), "{said}");
}

/// A hub the check could not match on is brought forward, not opened a second tab beside.
///
/// `hub_status` matches a record with no start time to anchor on by looking for the name in
/// the command line, and a hub that has replaced its own no longer carries it. The claim in
/// the tab would answer `Taken` and bring that hub forward — the same place, reached the
/// long way round, after this side has already said it started one and exited 0 for a hub
/// that was up the whole time.
#[test]
fn a_hub_the_presence_check_could_not_match_is_brought_forward_rather_than_opened_beside() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_spawn_stub_config(&fixture, &spawned);

    // This process stands in for the hub: a live pid, under a name the check will not find
    // in its command line, and **no start time** — which is what sends the check to the
    // name and leaves the claim's own reading answering `Alive`.
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    let written = serde_json::json!({
        "pid": std::process::id(), "hubName": "adjutant-acme-widget-renamed", "cwd": "/",
    })
    .to_string();
    std::fs::write(&record, &written).unwrap();

    let out = fixture.cmd(&["hub", "--tab"]);
    let said =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("is already running"), "{said}");
    assert!(!said.contains("new tab"), "{said}");
    assert!(
        !spawned.exists(),
        "a tab was opened for a hub that was already running"
    );
    // And the record it found is the record it leaves.
    assert_eq!(std::fs::read_to_string(&record).unwrap(), written);
}

/// A record whose liveness cannot be established is not a free name, and `--tab` is the one
/// route that would have taken it anyway.
///
/// `hub_status` answers `present: false` to "nobody is there" and to "cannot tell" alike.
/// The route that claims has `claim_hub` behind it to draw the line; the tab route leaves
/// the claim to the tab, so the line is drawn before the tab is opened or not at all — and
/// not at all is the worst of the two, because the caller `--tab` exists for is an agent
/// reading an exit code, and `exit 0` beside "started in a new tab" is read as a hub that
/// is now running.
#[test]
fn a_hub_whose_record_cannot_be_read_is_not_opened_in_a_tab() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_spawn_stub_config(&fixture, &spawned);

    let record = forge_unreadable_hub_record(&fixture);
    let before = std::fs::read_to_string(&record).unwrap();

    // For real, not a dry run: a dry run opens nothing either way, so only this can tell a
    // refusal from a tab that was opened.
    let out = fixture.cmd(&["hub", "--tab"]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success(), "{said}");
    assert!(
        said.contains("cannot tell whether the hub recorded in"),
        "{said}"
    );
    // Which record, so the person sent to look has somewhere to look.
    assert!(
        said.contains(&record.to_string_lossy().to_string()),
        "{said}"
    );
    assert!(
        !spawned.exists(),
        "a tab was opened for a hub nobody can account for"
    );
    // And the record it could not read is the record it leaves.
    assert_eq!(std::fs::read_to_string(&record).unwrap(), before);
}

/// The other way the answer goes missing: the record parses, and `ps` will not answer.
///
/// Two sources, one state. A fix that only asked whether the JSON parses would leave this
/// one opening tabs, and it is the likelier of the two on a machine under load or with a
/// locked-down `ps`.
#[test]
fn a_hub_is_not_opened_in_a_tab_when_ps_cannot_answer_for_the_record() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_spawn_stub_config(&fixture, &spawned);

    // A perfectly good record, anchored on this process — asked before `ps` is taken away,
    // which is the same order the launcher wrote one in.
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    let written = serde_json::json!({
        "pid": std::process::id(), "hubName": "adjutant-someone-else", "cwd": "/",
        "psStarted": ps_started(std::process::id()),
    })
    .to_string();
    std::fs::write(&record, &written).unwrap();

    // A `ps` that fails *with something on stderr*: that is what tells "I could not do
    // that" from "no such process", and only the second is an answer.
    let stubs = fixture.repo.join("stub-bin");
    std::fs::create_dir_all(&stubs).unwrap();
    let ps = stubs.join("ps");
    std::fs::write(
        &ps,
        "#!/bin/sh\necho 'ps: cannot do that here' >&2\nexit 1\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&ps, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Prepended rather than replacing: the binary still has to find the real `git`.
    let path = format!(
        "{}:{}",
        stubs.to_string_lossy(),
        std::env::var("PATH").unwrap_or_default()
    );

    let out = Command::new(BIN)
        .args(["hub", "--tab"])
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .env("PATH", &path)
        .hermetic()
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success(), "{said}");
    assert!(
        said.contains("cannot tell whether the hub recorded in"),
        "{said}"
    );
    assert!(
        said.contains(&record.to_string_lossy().to_string()),
        "{said}"
    );
    assert!(
        !spawned.exists(),
        "a tab was opened while `ps` was answering nothing"
    );
    assert_eq!(std::fs::read_to_string(&record).unwrap(), written);
}

/// The route that claims refuses the same state, and refuses it the way it always did.
///
/// This is not new behaviour — `claim_hub` has always stopped here — and that is the point
/// of pinning it: the tab route was made to agree with this one, so a later change that
/// moved the words or the exit code would have the two disagreeing again.
#[test]
fn the_route_that_claims_refuses_an_unreadable_record_as_it_always_did() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_spawn_stub_config(&fixture, &spawned);
    let record = forge_unreadable_hub_record(&fixture);

    // No `--tab`, and not a dry run, so this is the exec path — which `hubRunner: true`
    // keeps from starting a real agent if the refusal ever stops happening.
    let out = fixture.cmd(&["hub"]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success(), "{said}");
    assert!(
        said.contains("cannot tell whether the hub recorded in"),
        "{said}"
    );
    assert!(
        said.contains(&record.to_string_lossy().to_string()),
        "{said}"
    );
}

/// And the route that claims nothing still says what it would run.
///
/// A dry run on the default route prints a command and stops; it writes nothing, claims
/// nothing and opens nothing, so there is nothing there for an unreadable record to be
/// dangerous to. The refusal is the tab route's, and this is what says so: move it up a
/// line, to cover both routes, and this goes red.
#[test]
fn a_dry_run_that_claims_nothing_still_says_what_it_would_run() {
    let fixture = Fixture::new(QUIET);
    forge_unreadable_hub_record(&fixture);
    let out = fixture.ok(&["hub", "--dry-run"]);
    assert!(out.contains(&format!("claude -n {HUB}")), "{out}");
}

/// The tab is opened for one hub of the repository, and has to be told which.
///
/// The identifier cannot ride the environment across: the terminal is handed a command line
/// and nothing else. So it goes onto the line — and it is the *resolved* one, because the
/// caller this exists for is a hub opening a tab as its own child, passing no flag at all.
#[test]
fn a_tab_opened_for_a_hub_is_told_which_hub_it_is_opening() {
    let fixture = Fixture::new(QUIET);
    let asked = fixture.ok(&["hub", "--tab", "--hub", FEATURE, "--dry-run"]);
    assert!(asked.contains(&format!("--hub={FEATURE}")), "{asked}");
    // Named for the address it will answer at, so the tab is findable as that hub.
    assert!(asked.contains("adjutant-acme-widget-wid-957"), "{asked}");

    let inherited = Command::new(BIN)
        .args(["hub", "--tab", "--dry-run"])
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .hermetic()
        .env("ADJUTANT_HUB", FEATURE)
        .output()
        .unwrap();
    let line = String::from_utf8_lossy(&inherited.stdout);
    assert!(line.contains(&format!("--hub={FEATURE}")), "{line}");

    // Extra arguments survive the trip too: they are the trailing argument at both ends, so
    // the separator has to go back on the line the tab is handed.
    let extra = fixture.ok(&["hub", "--tab", "--dry-run", "--", "--resume"]);
    assert!(extra.contains("hub -- --resume"), "{extra}");
}

/// What `--no-dashboard` has to reach is the hub's own procedure, which asks the MCP server
/// for its settings — so the flag becomes a variable on the line the agent is `exec`ed
/// with, and the resolver folds it in before anybody reads `startupDashboard`.
#[test]
fn a_hub_told_not_to_collect_carries_that_down_to_the_agent_it_becomes() {
    let fixture = Fixture::new(QUIET);
    let off = fixture.ok(&["hub", "--dry-run", "--no-dashboard"]);
    assert!(off.contains("ADJUTANT_STARTUP_DASHBOARD=0"), "{off}");
    let on = fixture.ok(&["hub", "--dry-run", "--dashboard"]);
    assert!(on.contains("ADJUTANT_STARTUP_DASHBOARD=1"), "{on}");

    // Neither flag adds anything at all. The line a plain `adj hub` prints is one people
    // read, script and paste, and an override appearing in it would be a change to that
    // line for every user who never asked about the dashboard.
    let plain = fixture.ok(&["hub", "--dry-run"]);
    assert!(!plain.contains("ADJUTANT_STARTUP_DASHBOARD"), "{plain}");

    // Both at once is a contradiction with no sensible winner, so clap refuses it rather
    // than letting declaration order decide.
    let both = fixture.cmd(&["hub", "--dry-run", "--dashboard", "--no-dashboard"]);
    assert!(!both.status.success(), "{both:?}");
}

/// The `--tab` route never reaches the environment the other one builds: the terminal is
/// handed a command line and nothing else, exactly as with `--hub`. So the flag is
/// forwarded as a flag, and above the `--` — below it, clap at the far end would take it as
/// a trailing argument and append it to the *agent's* command instead of parsing it.
#[test]
fn a_tab_opened_for_a_hub_is_told_whether_to_collect_too() {
    let fixture = Fixture::new(QUIET);
    let off = fixture.ok(&["hub", "--tab", "--dry-run", "--no-dashboard"]);
    assert!(off.contains("--no-dashboard"), "{off}");
    // The variable belongs to the hub the tab starts, not to the launcher that opens it:
    // this line runs `adj hub`, and that invocation builds its own environment.
    assert!(!off.contains("ADJUTANT_STARTUP_DASHBOARD"), "{off}");

    let on = fixture.ok(&["hub", "--tab", "--dry-run", "--dashboard"]);
    assert!(on.contains("--dashboard"), "{on}");

    let extra = fixture.ok(&["hub", "--tab", "--dry-run", "--no-dashboard", "--", "-r"]);
    let line = extra.find("--no-dashboard").unwrap();
    assert!(line < extra.find("-- -r").unwrap(), "{extra}");

    let plain = fixture.ok(&["hub", "--tab", "--dry-run"]);
    assert!(!plain.contains("dashboard"), "{plain}");
}

/// The machine says collect and this repository says don't.
const DASHBOARD_PER_REPO: &str = r#"{"notification": "true", "startupDashboard": true,
    "defaults": {"ide": "code"},
    "repos": {"acme/widget": {"startupDashboard": false, "taskSource": "github",
              "issueRepo": "acme/widget", "issueKeys": {"acme/widget": "WID"}}}}"#;

/// The same, with nothing said about the repository.
const DASHBOARD_PER_MACHINE: &str = r#"{"notification": "true", "startupDashboard": false,
    "defaults": {"ide": "code"},
    "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
              "issueKeys": {"acme/widget": "WID"}}}}"#;

/// The middle rung: `defaults` says don't, the top level says do, and the entry says nothing.
/// Both neighbours disagree with it, so the answer can only have come from `defaults` itself.
const DASHBOARD_PER_DEFAULTS: &str = r#"{"notification": "true", "startupDashboard": true,
    "defaults": {"ide": "code", "startupDashboard": false},
    "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
              "issueKeys": {"acme/widget": "WID"}}}}"#;

/// Every answer `settings.startupDashboard` can give, read back out of the binary.
///
/// `adj config` is what the hub's procedure actually reads, so this is the only place the
/// whole trip is visible: the file, the level that won it, and the flag that outranks both.
///
/// The unit tests take that decision apart — which level wins, and what the flag does to the
/// winner — and check the halves separately. Nothing checked that the resolver still joins
/// them: hand the deciding function `None` for the configured value instead of the level it
/// picked, and every one of those tests stays green while a repository that turned the
/// collection off gets it anyway.
///
/// Both directions are asserted, because a resolver that ignored the file would be right
/// half the time by accident — `true` is also the default, so a fixture that only ever says
/// `false` would not tell "read it" from "never read it and defaulted".
#[test]
fn what_the_config_file_says_about_the_dashboard_reaches_the_resolved_settings() {
    // A machine that has never heard of the setting collects, which is what every existing
    // config has to keep doing.
    assert_eq!(
        Fixture::new(QUIET).json(&["config"])["settings"]["startupDashboard"],
        true
    );

    // The most specific level wins, as it does for every other machine setting — and the
    // two levels are made to disagree so that the answer names which one was read.
    let per_repo = Fixture::new(DASHBOARD_PER_REPO);
    assert_eq!(
        per_repo.json(&["config"])["settings"]["startupDashboard"],
        false
    );

    // And a machine that says it once, for every repository it holds, is read the same way.
    // A resolver that only ever looked at the entry would pass the case above.
    let per_machine = Fixture::new(DASHBOARD_PER_MACHINE);
    assert_eq!(
        per_machine.json(&["config"])["settings"]["startupDashboard"],
        false
    );

    // The rung between those two. `defaults` is the level the other two cases step over
    // without touching, so a break in it — `pick` skipping the middle, or `defaults` losing
    // to the top level — passes everything above and is found by nobody. With the entry
    // silent and the two outer levels disagreeing, `false` here names `defaults` and only
    // `defaults`, which makes this one assertion pin the whole order: entry > defaults > root.
    let per_defaults = Fixture::new(DASHBOARD_PER_DEFAULTS);
    assert_eq!(
        per_defaults.json(&["config"])["settings"]["startupDashboard"],
        false
    );

    // The flag still outranks the file it disagrees with, asked of the same fixture that
    // says `false` so that `true` coming back can only have come from the flag.
    let overridden = Command::new(BIN)
        .args(["config"])
        .current_dir(&per_repo.repo)
        .env("ADJUTANT_CONFIG", &per_repo.config)
        .env("ADJUTANT_STATE_DIR", &per_repo.state)
        .hermetic()
        .env("ADJUTANT_STARTUP_DASHBOARD", "1")
        .output()
        .unwrap();
    let answer: serde_json::Value =
        serde_json::from_slice(&overridden.stdout).expect("config printed no JSON");
    assert_eq!(answer["settings"]["startupDashboard"], true);
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

/// A hub record that is there and is not a record. Both readings of it — the one behind
/// `present` and the one behind a claim — have to meet it, so it is written as bytes rather
/// than as a JSON document with something wrong inside.
fn forge_unreadable_hub_record(fixture: &Fixture) -> PathBuf {
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(&record, "{ this was a hub record once").unwrap();
    record
}

/// A terminal that writes down what it was handed instead of opening anything, and a hub
/// runner that is `true`.
///
/// The runner matters even where the test never means to reach it: a regression that let a
/// refusal through would fall down to the exec path, and the net is what turns that into a
/// failing test rather than a real agent started inside the suite.
fn write_spawn_stub_config(fixture: &Fixture, spawned: &Path) {
    std::fs::write(
        &fixture.config,
        serde_json::json!({
            "notification": "true",
            "hubRunner": "true {name} {prompt}",
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
    // Sanitised like every other child, though a help dump reads neither config nor
    // environment: an exemption is a thing the next reader has to re-derive.
    let help = String::from_utf8(
        Command::new(BIN)
            .arg("--help")
            .hermetic()
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
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
        .hermetic()
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
        .hermetic()
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

#[test]
fn a_gate_can_be_closed_without_delivering_to_the_worker() {
    let fixture = Fixture::new(QUIET);
    let payload = serde_json::json!({
        "kind": "plan",
        "title": "設計方針の確認",
        "worktree": fixture.repo.to_str().unwrap(),
    });
    let payload_file = fixture.repo.join("gate.json");
    std::fs::write(&payload_file, payload.to_string()).unwrap();

    let opened = fixture.json(&[
        "gate",
        "open",
        "--file",
        payload_file.to_str().unwrap(),
        "--json",
    ]);
    let gate_id = opened["gate"]["id"].as_str().unwrap();

    let list = fixture.json(&["gate", "list", "--json"]);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], gate_id);

    // Close the gate directly (e.g. human responded in tab)
    let closed = fixture.json(&[
        "gate",
        "close",
        "--id",
        gate_id,
        "--comment",
        "dealt with in tab",
        "--json",
    ]);
    assert_eq!(closed["gate"]["id"], gate_id);
    assert_eq!(closed["gate"]["decision"], "closed");
    assert_eq!(closed["gate"]["comment"], "dealt with in tab");
    assert!(closed["closed"].as_bool().unwrap());

    // The gate is no longer open
    let list_after = fixture.json(&["gate", "list", "--json"]);
    assert!(list_after.as_array().unwrap().is_empty());

    // And nothing was written to outbox (no worker message delivered)
    let outbox = fixture.ok(&["outbox", "--worktree", fixture.repo.to_str().unwrap()]);
    assert_eq!(outbox.trim(), "(empty)");
}

// ── --resume ─────────────────────────────────────────────────────────

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
    let out = Command::new(BIN)
        .args(["worker", "--resume"])
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .hermetic()
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

    let out = Command::new(BIN)
        .args(["work", "--resume", "--worktree", &worktree, "--dry-run"])
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .hermetic()
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
    assert!(line.contains("--title WID-1"), "{line}");
    assert!(!line.contains("--hub"), "{line}");
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
fn the_hub_tells_its_mcp_server_which_session_it_is() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.ok(&["hub", "--dry-run"]);
    assert!(
        out.contains(&format!("ADJUTANT_HUB_SESSION={SLUG}/")),
        "{out}"
    );
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
    let out = Command::new(BIN)
        .arg("mcp")
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .hermetic()
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
    Command::new(BIN)
        .arg("mcp")
        .current_dir(&quiet.repo)
        .env("ADJUTANT_CONFIG", &quiet.config)
        .env("ADJUTANT_STATE_DIR", &quiet.state)
        .hermetic()
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
            "sh -c 'echo \"[$ADJUTANT_HUB_SESSION]\"' > {} ; true {{sessionId}} {{prompt}}",
            shell_quoted(&seen.to_string_lossy())
        )
        .into(),
    );
    let out = Command::new(BIN)
        .args(["worker", "--worktree", fixture.repo.to_str().unwrap()])
        .current_dir(&fixture.repo)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", &fixture.state)
        .hermetic()
        .env("ADJUTANT_HUB_SESSION", format!("{SLUG}/the-hubs-session"))
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
fn skill_formats_for_specified_agent() {
    let fixture = Fixture::new(QUIET);
    let claude = fixture.ok(&["skill", "adj-hub", "--agent", "claude"]);
    let agy = fixture.ok(&["skill", "adj-hub", "--agent", "agy"]);

    assert!(claude.contains("AskUserQuestion"));
    assert!(!agy.contains("AskUserQuestion"));
    assert!(agy.contains("ask_question"));
}

#[test]
fn skill_rejects_unknown_agent() {
    let fixture = Fixture::new(QUIET);
    let out = Command::new(BIN)
        .args(["skill", "adj-hub", "--agent", "invalid-agent"])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("invalid value 'invalid-agent'"));
}

#[test]
fn install_mcp_rejects_unknown_target() {
    let fixture = Fixture::new(QUIET);
    let out = Command::new(BIN)
        .args(["install-mcp", "--target", "invalid-target"])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("target must be claude-code, agy, or json"));
}
