//! End-to-end tests: run the real binary against a throwaway repository and a throwaway
//! state directory.
//!
//! Everything here is hermetic on purpose. `ADJUTANT_CONFIG` and `ADJUTANT_STATE_DIR` point
//! into tempdirs, so a test can never read the developer's own config or drop a fixture
//! report into a hub that is actually running. Anything that would open a window, start an
//! agent or notify a human is exercised through `--dry-run`.
//!
//! Shared by every test file here. Each file in `tests/` is its own crate, and not all of them
//! use everything, so unused items and re-exports are allowed rather than warned about.

#![allow(dead_code, unused_imports)]

pub use std::io::{BufRead, Write};
pub use std::path::{Path, PathBuf};
pub use std::process::{Command, Stdio};

pub const BIN: &str = env!("CARGO_BIN_EXE_adjutant");

/// Every fixture repository has the same origin, so its address is fixed too. Written out
/// rather than derived from `adjutant::repo`, so that a change to how a slug is built shows
/// up here as a failing test instead of as two implementations agreeing with each other.
pub const SLUG: &str = "acme-widget-898449509108182c";
pub const HUB: &str = "adjutant-acme-widget-898449509108182c";

/// The same repository, addressed as one hub of it rather than as itself. Written out for
/// the same reason, and load-bearing for a second one: these two constants differing is
/// what a separate inbox *is*.
pub const FEATURE: &str = "wid-957";
pub const FEATURE_SLUG: &str = "acme-widget-wid-957-5283c95d4f4cc314";
pub const FEATURE_HUB: &str = "adjutant-acme-widget-wid-957-5283c95d4f4cc314";

/// Everything a child of this suite must not inherit from whatever ran `cargo test`.
///
/// That is regularly a tab which *is* a hub, and a hub exports its own answers:
/// `ADJUTANT_HUB` re-addresses every inbox asserted on here, and — since `adj hub
/// --no-dashboard` sets it — `ADJUTANT_STARTUP_DASHBOARD` outranks the `startupDashboard` a
/// fixture has just written into its own config file.
pub const AMBIENT: [&str; 7] = [
    "ADJUTANT_HUB",
    "ADJUTANT_STARTUP_DASHBOARD",
    // A hub's MCP server beats for the session this names; a test child that inherited it
    // would be beating for the hub `cargo test` was typed in.
    "ADJUTANT_HUB_SESSION",
    // And would serve that hub's board, on a real port, from inside the test.
    "ADJUTANT_HUB_SERVE",
    // `cargo test` run from a git hook has these pointing at the developer's own checkout,
    // and a fixture set up with git would be set up there instead of in its tempdir.
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
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
pub trait Ambient {
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

pub struct Fixture {
    pub _dir: tempfile::TempDir,
    pub repo: PathBuf,
    pub state: PathBuf,
    pub config: PathBuf,
}

impl Fixture {
    pub fn new(config_json: &str) -> Self {
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
                .hermetic()
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

    /// The binary, pointed at this fixture and cleared of `AMBIENT`, not yet run.
    ///
    /// For a test that has to change something before it runs — another working directory,
    /// a variable of its own, stdin. Everything set after this call lands on top of the
    /// sanitised environment, so a value the test hands over is the test's own rather than
    /// the terminal's (see `AMBIENT`).
    pub fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .current_dir(&self.repo)
            .env("ADJUTANT_CONFIG", &self.config)
            .env("ADJUTANT_STATE_DIR", &self.state)
            .hermetic();
        command
    }

    pub fn cmd(&self, args: &[&str]) -> std::process::Output {
        self.command(args).output().unwrap()
    }

    pub fn ok(&self, args: &[&str]) -> String {
        let out = self.cmd(args);
        assert!(
            out.status.success(),
            "adjutant {args:?} failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    pub fn json(&self, args: &[&str]) -> serde_json::Value {
        serde_json::from_str(&self.ok(args)).unwrap()
    }
}

/// A config that notifies by doing nothing. Every test that sends uses it: the built-in
/// notifier puts a banner on the developer's screen, which is not something a test suite
/// gets to do.
pub const QUIET: &str = r#"{
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

pub const CODEX: &str = r#"{"notification": "true",
    "defaults": {"ide": "code", "agentRunner": "codex exec {prompt}"},
    "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
              "issueKeys": {"acme/widget": "WID"}}}}"#;

/// POSIX single-quoting, for a value going into a shell line.
///
/// The crate's own `sh_quote` is not reachable from an integration test. Quoting matters
/// here for the same reason it matters in the tool: a `TMPDIR` with a space in it is the
/// machine's business, not a defect in what is under test.
pub fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Feed the server a batch of requests and collect one reply per line.
pub fn mcp(fixture: &Fixture, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
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

/// What the system says about when a process started, in the same words the binary records.
pub fn ps_started(pid: u32) -> String {
    let out = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub fn request(id: u32, method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}
