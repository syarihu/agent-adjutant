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

pub use std::io::{BufRead, Read, Write};
pub use std::path::{Path, PathBuf};
pub use std::process::{Command, Stdio};

pub const BIN: &str = env!("CARGO_BIN_EXE_adjutant");

/// Every fixture repository has the same origin, so its address is fixed too. Written out
/// rather than derived from `adjutant::kernel::identity`, so that a change to how a slug is built shows
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
pub const AMBIENT: [&str; 16] = [
    "ADJUTANT_HUB",
    "ADJUTANT_STARTUP_DASHBOARD",
    // A hub's MCP server beats for the session this names; a test child that inherited it
    // would be beating for the hub `cargo test` was typed in.
    "ADJUTANT_HUB_SESSION",
    // And would serve that hub's board, on a real port, from inside the test.
    "ADJUTANT_HUB_SERVE",
    // Tmux socket and session override ambient settings so integration tests don't touch
    // the developer's live tmux session.
    "ADJUTANT_TMUX_SOCKET",
    "ADJUTANT_TMUX_SESSION",
    // A worker records the tmux pane it runs in from these, so a `cargo test` typed inside
    // tmux would have it ask the developer's own server where it is.
    "TMUX",
    "TMUX_PANE",
    // `cargo test` run from a git hook has these pointing at the developer's own checkout,
    // and a fixture set up with git would be set up there instead of in its tempdir.
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    // The resident server polls GitHub on its own, so a test child that inherited a login would
    // reach GitHub with the developer's account. These are how `gh` finds one; the fixture
    // also points `GH_CONFIG_DIR` at an empty directory (see `Fixture::command`).
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "GH_HOST",
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
            // Where `gh` keeps its login: an empty one has none, so a `gh` that a test did not
            // stub finds no account to use rather than the developer's.
            .env("GH_CONFIG_DIR", self._dir.path().join("gh-config"))
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
    mcp_in(fixture, &fixture.repo, requests)
}

/// Feed the server running in a specified directory a batch of requests.
pub fn mcp_in(
    fixture: &Fixture,
    dir: impl AsRef<Path>,
    requests: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let mut child = Command::new(BIN)
        .arg("mcp")
        .current_dir(dir)
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

// ── the resident server and an isolated tmux, shared by the tests that need one ──

/// A resident server running in the foreground, killed when this goes out of scope so that a
/// failing assertion leaves nothing listening.
pub struct Resident {
    pub child: std::process::Child,
    pub port: u16,
    pub token: String,
    /// The line it printed when it came up.
    pub said: String,
}

impl Resident {
    pub fn start(fixture: &Fixture) -> Resident {
        Resident::start_with(fixture, &[])
    }

    pub fn start_with(fixture: &Fixture, env: &[(&str, &str)]) -> Resident {
        let child = fixture
            .command([
                "server",
                "start",
                "--foreground",
                "--no-open",
                "--port",
                "0",
            ])
            .envs(env.iter().copied())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // The guard first, so that whatever is wrong with what it says does not leave it running.
        let mut resident = Resident {
            child,
            port: 0,
            token: String::new(),
            said: String::new(),
        };
        let mut said = String::new();
        std::io::BufReader::new(resident.child.stdout.as_mut().unwrap())
            .read_line(&mut said)
            .unwrap();
        // Not a terminal, so the line names no token: it is what lands in a log.
        let url = said
            .split("serving on ")
            .nth(1)
            .unwrap_or_else(|| panic!("no URL in {said:?}"))
            .trim();
        let host = url
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        resident.port = host.rsplit(':').next().unwrap().parse().unwrap();
        resident.token = std::fs::read_to_string(fixture.state.join("dashboard-token"))
            .unwrap()
            .trim()
            .to_string();
        resident.said = said;
        resident
    }

    /// A POST the way the page makes one: the token in a header and the server's own origin.
    pub fn post(&self, path: &str, body: &str) -> (u16, String) {
        post(self.port, &self.token, path, body)
    }

    /// A GET with the token in the query, as `(status, body)`.
    pub fn get(&self, path: &str) -> (u16, String) {
        get(self.port, &self.token, path)
    }
}

impl Drop for Resident {
    fn drop(&mut self) {
        // Its `sh -c "tmux …"` children in flight would outlive it and race the test's
        // teardown, so they go with it. Collected first: once it is dead they are reparented.
        let mut pids = vec![self.child.id() as i32];
        let mut next = 0;
        while next < pids.len() {
            let out = Command::new("pgrep")
                .args(["-P", &pids[next].to_string()])
                .output();
            if let Ok(out) = out {
                pids.extend(
                    String::from_utf8_lossy(&out.stdout)
                        .split_whitespace()
                        .filter_map(|p| p.parse::<i32>().ok()),
                );
            }
            next += 1;
        }
        for pid in pids {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        let _ = self.child.wait();
    }
}

pub fn get(port: u16, token: &str, path: &str) -> (u16, String) {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let query = if token.is_empty() {
        String::new()
    } else {
        format!("?token={token}")
    };
    write!(
        stream,
        "GET {path}{query} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    let (head, body) = answer.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, body.to_string())
}

pub fn post(port: u16, token: &str, path: &str, body: &str) -> (u16, String) {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
         X-Adjutant-Token: {token}\r\nOrigin: http://127.0.0.1:{port}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    let (head, body) = answer.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, body.to_string())
}

/// A tmux server of its own, started here so that its panes can never be login shells.
///
/// A command-less pane (`new-session -d`, or the spawn template's `new-session` when the
/// session is missing) runs the default shell as a login shell. When a test ends while such a
/// pane is still being spawned, `kill-server` can take the server away under it, and the shell
/// is left holding a pty for good. A pane that runs `cat` instead gets EIO from a slave with
/// no master and exits, so even that orphan holds nothing.
pub struct IsolatedTmux {
    pub socket: String,
    pub session: String,
    // Under TMPDIR, so that the guard in `scripts/check-test-leaks.sh` sees a server that
    // outlives its test. Dropped after `Drop::drop`, i.e. after the server is gone.
    _cwd: tempfile::TempDir,
}

/// Where tmux puts a `-L` socket: `$TMUX_TMPDIR` (not `TMPDIR`), else `/tmp`.
fn tmux_socket_dir() -> PathBuf {
    let base = std::env::var_os("TMUX_TMPDIR")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/tmp".into());
    PathBuf::from(base).join(format!("tmux-{}", unsafe { libc::getuid() }))
}

/// A name no other test, or copy of this binary, shares. The pid is what lets a later run
/// tell a socket left by a killed test process from one that is in use (see `sweep_dead`).
fn unique(name: &str) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    format!(
        "adj-test-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// Kill the servers of test processes that no longer exist, which `Drop` never got to.
fn sweep_dead() {
    let Ok(entries) = std::fs::read_dir(tmux_socket_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix("adj-test-") else {
            continue;
        };
        let mut parts = rest.rsplitn(3, '-');
        let (_, Some(pid)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(pid) = pid.parse::<i32>() else {
            continue;
        };
        let alive = unsafe { libc::kill(pid, 0) } == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
        if !alive {
            kill_server(&name);
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Run `kill-server` until nothing answers. A tmux client that was already on its way in can
/// find the socket dead, remove it and start a fresh server after the first kill, so one
/// attempt is not enough.
fn kill_server(socket: &str) {
    for _ in 0..10 {
        // Its wording for "nothing there" differs between versions, its exit status does not.
        let killed = Command::new("tmux")
            .hermetic()
            .args(["-L", socket, "kill-server"])
            .output()
            .is_ok_and(|out| out.status.success());
        if !killed {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // A client that found the socket refusing removes it, which leaves a server that is still
    // winding down with no way to be asked to stop. Only its command line or title names it,
    // and a title (`tmux: server (<dir>/<socket>)`) ends the name differently, so the name is
    // followed by a terminator that `-1` does not share with `-10`.
    let _ = Command::new("pkill")
        .args(["-KILL", "-f", &format!("{socket}([ )]|$)")])
        .output();
}

impl IsolatedTmux {
    pub fn new(name: &str) -> Option<Self> {
        let out = Command::new("tmux").arg("-V").output().ok()?;
        if !out.status.success() {
            return None;
        }
        static SWEPT: std::sync::Once = std::sync::Once::new();
        SWEPT.call_once(sweep_dead);
        let socket = unique(name);
        let cwd = tempfile::tempdir().unwrap();
        let started = Command::new("tmux")
            .hermetic()
            .current_dir(cwd.path())
            .args(["-L", &socket, "-f", "/dev/null", "start-server"])
            .args([";", "set", "-s", "exit-empty", "off"])
            .args([";", "set", "-g", "default-shell", "/bin/sh"])
            .args([";", "set", "-g", "default-command", "exec cat"])
            .output()
            .unwrap();
        assert!(
            started.status.success(),
            "tmux start-server: {}",
            String::from_utf8_lossy(&started.stderr)
        );
        Some(IsolatedTmux {
            socket,
            session: "adjutant-test".to_string(),
            _cwd: cwd,
        })
    }

    pub fn tmux_cmd(&self, args: &[&str]) -> std::process::Output {
        Command::new("tmux")
            .hermetic()
            // Where the guard can see a session made through this, whatever it is started in.
            .current_dir(self._cwd.path())
            .arg("-L")
            .arg(&self.socket)
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for IsolatedTmux {
    fn drop(&mut self) {
        kill_server(&self.socket);
        let _ = std::fs::remove_file(tmux_socket_dir().join(&self.socket));
    }
}

/// A raw `adjutant serve` (or any other child a test spawns by hand), killed when this goes
/// out of scope so that a failing assertion leaves nothing listening.
pub struct Reaped(pub std::process::Child);

impl Reaped {
    /// `Child::wait_with_output`, which needs the child by value.
    pub fn wait_with_output(self) -> std::process::Output {
        let this = std::mem::ManuallyDrop::new(self);
        // Read out once and never dropped here, so `Drop` does not kill what is waited for.
        let child = unsafe { std::ptr::read(&this.0) };
        child.wait_with_output().unwrap()
    }
}

impl std::ops::Deref for Reaped {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Reaped {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// ── what a server asked of the tools it runs ──

/// `git`, `ps` and `tmux` wrappers that write down each call and then run the real tool, for a
/// test that asserts on what the server did *not* ask. Put `path()` in the server's `PATH`.
pub struct Spy {
    bin: PathBuf,
    log: PathBuf,
}

impl Spy {
    pub fn new(dir: &Path) -> Spy {
        let bin = dir.join("spybin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = dir.join("spy.log");
        for tool in ["git", "ps", "tmux"] {
            let found = Command::new("sh")
                .args(["-c", &format!("command -v {tool}")])
                .output()
                .unwrap();
            let real = String::from_utf8_lossy(&found.stdout).trim().to_string();
            if real.is_empty() {
                continue;
            }
            stub_bin(
                &bin,
                tool,
                &format!(
                    "#!/bin/sh\necho \"{tool} $*\" >> '{}'\nexec '{real}' \"$@\"\n",
                    log.display()
                ),
            );
        }
        Spy { bin, log }
    }

    pub fn path(&self) -> String {
        path_with(&self.bin)
    }

    /// Forget what was asked so far, such as by a server still coming up.
    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.log);
    }

    /// Each call since the last `clear`, as `tool arguments`.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

// ── helpers shared between test files ──

pub fn set_config(fixture: &Fixture, key: &str, value: serde_json::Value) {
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fixture.config).unwrap()).unwrap();
    config[key] = value;
    std::fs::write(&fixture.config, config.to_string()).unwrap();
}

/// A GET whose own query is `query`, with the token added after it.
pub fn get_with_query(resident: &Resident, path: &str, query: &str) -> (u16, String) {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", resident.port)).unwrap();
    write!(
        stream,
        "GET {path}?{query}&token={} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        resident.token
    )
    .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    let (head, body) = answer.split_once("\r\n\r\n").unwrap();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        body.to_string(),
    )
}

/// A worker record for `pid`, carrying the start time the system reports for it — the
/// anchor that tells that process from whatever the pid is handed to next.
pub fn forge_worker_record(worktree: &Path, pid: u32) -> PathBuf {
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
pub struct Sleeper {
    shell: std::process::Child,
    /// `None` only while `new` is still assembling one. The value exists before the pid is
    /// read so that a panic during construction still runs `Drop`: a value that never
    /// finished being built is never dropped, and both processes would be left behind.
    pid: Option<u32>,
}

impl Sleeper {
    pub fn new() -> Sleeper {
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

    pub fn pid(&self) -> u32 {
        self.pid.expect("the stand-in worker has no pid")
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        // By closing the pipe, never by pid. Two tests kill the stand-in through the close
        // template, and the shell reaps it at once, so its pid is back with the system by the
        // time this runs — with the suite running in parallel, a second `kill` could reach
        // whatever holds that number now. Closing the pipe needs no pid: `cat` reads end of
        // file and exits, the shell reaps it and leaves `wait`, and waiting on the shell
        // then returns. A stand-in that is already gone changes nothing.
        drop(self.shell.stdin.take());
        let _ = self.shell.wait();
    }
}

pub fn tool_result(response: &serde_json::Value) -> serde_json::Value {
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).expect(text)
}

/// `GET /api/state` from the board, whole.
pub fn fetch_state(url: &str) -> serde_json::Value {
    let rest = url.strip_prefix("http://").unwrap();
    let (host, query) = rest.split_once('/').unwrap();
    let token = query.split("token=").nth(1).unwrap();
    let mut stream = std::net::TcpStream::connect(host).unwrap();
    write!(
        stream,
        "GET /api/state?token={token} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut answer = String::new();
    std::io::Read::read_to_string(&mut stream, &mut answer).unwrap();
    let body = answer.split_once("\r\n\r\n").unwrap().1;
    serde_json::from_str(body).unwrap()
}

/// A board serving this fixture, with `env` on top of it, and the URL it printed.
pub fn serve_board(fixture: &Fixture, env: &[(&str, &str)]) -> (Reaped, String) {
    let mut by_hand = Reaped(
        fixture
            .command(["serve", "--port", "0", "--no-open"])
            .envs(env.iter().copied())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut said = String::new();
    std::io::BufReader::new(by_hand.stdout.as_mut().unwrap())
        .read_line(&mut said)
        .unwrap();
    let url = said.split(" — ").nth(1).unwrap().trim().to_string();
    (by_hand, url)
}

/// Polls until `done` holds, or panics after about ten seconds with what was last seen.
pub fn wait_until<T: std::fmt::Debug>(what: &str, mut seen: impl FnMut() -> (bool, T)) -> T {
    let mut last = None;
    for _ in 0..100 {
        let (done, value) = seen();
        if done {
            return value;
        }
        last = Some(value);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("{what}: last saw {last:?}");
}

/// A stand-in for the tool `name`: `script` written to `dir/name` and made executable, `dir`
/// made first if it is not there. Put `path_with(dir)` in the child's `PATH`.
pub fn stub_bin(dir: &Path, name: &str, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join(name);
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// `dir` ahead of the inherited `PATH`: prepended rather than replacing, so a child still
/// finds the real `git`.
pub fn path_with(dir: &Path) -> String {
    format!(
        "{}:{}",
        dir.to_string_lossy(),
        std::env::var("PATH").unwrap_or_default()
    )
}
