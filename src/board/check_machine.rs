//! Looking at the machine before the server runs, and again while it does: whether tmux, `gh`
//! and its login, the adjutant MCP registration and the config are in a state the rest of the
//! program can work with. Each is a `Check` the transports word (`transport::wording`); this file
//! only finds the facts.
//!
//! Nothing here writes. It runs `tmux -V`, `gh --version` and `gh auth status`, and reads the
//! agents' own config files; it never starts a tmux server and never runs an agent.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::kernel::config::{ResolveEnv, Resolved};

/// How long `tmux -V` and `gh --version` get. Both answer at once on a working machine.
const QUICK: Duration = Duration::from_secs(5);

/// How long `gh auth status` gets: it asks GitHub when the token is not cached.
const AUTH: Duration = Duration::from_secs(15);

/// A re-run asked for within this many seconds of the last one is answered with the last one.
const RECHECK_AFTER_SECS: i64 = 10;

/// What a check looked at. `wire` is the name the page and `--json` use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckId {
    Config,
    Tmux,
    TmuxVersion,
    Gh,
    GhAuth,
    Mcp,
}

impl CheckId {
    pub fn wire(&self) -> &'static str {
        match self {
            CheckId::Config => "config",
            CheckId::Tmux => "tmux",
            CheckId::TmuxVersion => "tmux-version",
            CheckId::Gh => "gh",
            CheckId::GhAuth => "gh-auth",
            CheckId::Mcp => "mcp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub id: CheckId,
    pub ok: bool,
    /// Whether a failure stops the server from starting.
    pub fatal: bool,
    /// What was found, for the wording: the tmux version, the parse error with its path, which
    /// agents the MCP server is registered with, a file that could not be read.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineChecks {
    pub checks: Vec<Check>,
    /// The tmux version when the board terminal can be offered: tmux ran and is 3.1 or later.
    /// Older tmux has neither `window-size latest` nor the hook that ends the terminal with its
    /// window, so the board would follow tmux on to another agent's window when the target
    /// closes. Always `None` where there is no unix.
    pub tmux: Option<(u32, u32)>,
    /// The config the check read, for the wording.
    pub config_path: String,
    /// Whether the built-in tmux backend is the one in use, which decides whether no tmux stops
    /// the server.
    pub tmux_backend: bool,
    /// Unix seconds.
    pub checked_at: i64,
}

impl MachineChecks {
    /// The failed checks that stop the server from starting.
    pub fn fatal(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| !c.ok && c.fatal)
    }

    /// Every failed check, fatal or not.
    pub fn failing(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| !c.ok)
    }
}

/// What the check asks of the machine, handed in so that a test runs no process and reads no
/// real config. A plain struct, not a trait: there is one real set and the tests' own.
pub struct Probes<'a> {
    /// Run a program with arguments and a deadline: `Ok((exited cleanly, stdout))`, and `Err`
    /// when it could not be run or did not finish in time.
    #[allow(clippy::type_complexity)]
    pub run: &'a dyn Fn(&str, &[&str], Duration) -> Result<(bool, String), String>,
    /// The resolved config for a repository, as `resolve_config` gives it.
    pub resolve: &'a dyn Fn(&str) -> Result<Resolved, String>,
    /// Claude Code's `.claude.json`.
    pub claude_config: Option<PathBuf>,
    /// Antigravity's `mcp_config.json`.
    pub agy_config: Option<PathBuf>,
    /// Codex's `config.toml`.
    pub codex_config: Option<PathBuf>,
}

/// The machine as it is now. `nwo` picks the repository's entry of the config, or is empty
/// outside a repository.
pub fn check_machine(nwo: &str) -> MachineChecks {
    let home = crate::infra::paths::home_dir();
    let dir_from = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let claude_config = Some(match dir_from(crate::infra::env::CLAUDE_CONFIG_DIR_ENV) {
        Some(dir) => dir.join(".claude.json"),
        None => home.join(".claude.json"),
    });
    let codex_config = Some(match dir_from(crate::infra::env::CODEX_HOME_ENV) {
        Some(dir) => dir.join("config.toml"),
        None => home.join(".codex").join("config.toml"),
    });
    let agy_config = Some(home.join(".gemini").join("config").join("mcp_config.json"));
    check_machine_with(
        nwo,
        &Probes {
            run: &run_program,
            resolve: &crate::kernel::config::resolve_config,
            claude_config,
            agy_config,
            codex_config,
        },
    )
}

pub fn check_machine_with(nwo: &str, probes: &Probes<'_>) -> MachineChecks {
    let mut checks = Vec::new();

    let (settings, config_path) = match (probes.resolve)(nwo) {
        Ok(resolved) => {
            checks.push(Check {
                id: CheckId::Config,
                ok: true,
                fatal: true,
                detail: None,
            });
            (resolved.settings, resolved.config_path)
        }
        Err(e) => {
            checks.push(Check {
                id: CheckId::Config,
                ok: false,
                fatal: true,
                detail: Some(e),
            });
            // The rest is checked against the built-ins, which is what a missing config gets.
            let (_, _, settings, _) = crate::kernel::config::resolve_from_value(
                &serde_json::json!({}),
                nwo,
                &ResolveEnv::default(),
            );
            let path = crate::kernel::config::config_path();
            (settings, path.to_string_lossy().to_string())
        }
    };

    let tmux_backend = crate::infra::terminal::watches_start(&settings.terminal);
    let tmux_out = (probes.run)("tmux", &["-V"], QUICK)
        .ok()
        .filter(|(clean, _)| *clean)
        .map(|(_, out)| out);
    checks.push(Check {
        id: CheckId::Tmux,
        ok: tmux_out.is_some(),
        fatal: tmux_backend,
        detail: None,
    });
    let mut tmux = None;
    if let Some(out) = &tmux_out {
        let shown = out.trim();
        let version = crate::infra::terminal::parse_tmux_version(out).filter(|&v| v >= (3, 1));
        if cfg!(unix) {
            tmux = version;
        }
        checks.push(Check {
            id: CheckId::TmuxVersion,
            ok: version.is_some(),
            fatal: false,
            detail: Some(
                shown
                    .strip_prefix("tmux")
                    .unwrap_or(shown)
                    .trim()
                    .to_string(),
            ),
        });
    }

    let gh_ran = (probes.run)("gh", &["--version"], QUICK).is_ok_and(|(clean, _)| clean);
    checks.push(Check {
        id: CheckId::Gh,
        ok: gh_ran,
        fatal: false,
        detail: None,
    });
    if gh_ran {
        // Only the exit status is kept: what `gh auth status` prints can show part of a token.
        let (ok, detail) = match (probes.run)("gh", &["auth", "status"], AUTH) {
            Ok((clean, _)) => (clean, None),
            Err(_) => (false, Some("gh auth status did not answer".to_string())),
        };
        checks.push(Check {
            id: CheckId::GhAuth,
            ok,
            fatal: false,
            detail,
        });
    }

    checks.push(mcp_check(probes));

    MachineChecks {
        checks,
        tmux,
        config_path,
        tmux_backend,
        checked_at: crate::infra::clock::now_secs(),
    }
}

/// Whether one agent's config registers the adjutant MCP server.
enum Registered {
    Yes,
    No,
    /// The file is there and could not be read as what it is: the answer is not known.
    Unknown(String),
}

fn mcp_check(probes: &Probes<'_>) -> Check {
    let name = crate::kernel::runner::MCP_SERVER_NAME;
    let found = [
        (
            "Claude Code",
            registered(&probes.claude_config, |text| json_registers(text, name)),
        ),
        (
            "Antigravity",
            registered(&probes.agy_config, |text| json_registers(text, name)),
        ),
        (
            "Codex",
            registered(&probes.codex_config, |text| Ok(toml_registers(text, name))),
        ),
    ];
    let agents: Vec<&str> = found
        .iter()
        .filter(|(_, r)| matches!(r, Registered::Yes))
        .map(|(agent, _)| *agent)
        .collect();
    if !agents.is_empty() {
        return Check {
            id: CheckId::Mcp,
            ok: true,
            fatal: false,
            detail: Some(agents.join(", ")),
        };
    }
    let unknown: Vec<String> = found
        .into_iter()
        .filter_map(|(_, r)| match r {
            Registered::Unknown(why) => Some(why),
            _ => None,
        })
        .collect();
    Check {
        id: CheckId::Mcp,
        ok: false,
        fatal: false,
        detail: (!unknown.is_empty()).then(|| unknown.join("; ")),
    }
}

fn registered(path: &Option<PathBuf>, reads: impl Fn(&str) -> Result<bool, String>) -> Registered {
    let Some(path) = path else {
        return Registered::No;
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Registered::No,
        Err(e) => return Registered::Unknown(unreadable(path, &e)),
    };
    match reads(&text) {
        Ok(true) => Registered::Yes,
        Ok(false) => Registered::No,
        Err(e) => Registered::Unknown(unreadable(path, &e)),
    }
}

fn unreadable(path: &Path, why: &dyn std::fmt::Display) -> String {
    format!("could not read {}: {why}", path.display())
}

fn json_registers(text: &str, name: &str) -> Result<bool, String> {
    let doc: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(doc
        .get("mcpServers")
        .and_then(|servers| servers.get(name))
        .is_some_and(serde_json::Value::is_object))
}

/// A line scan for the table header: the file is only asked whether the table is there, and a
/// TOML parser is a dependency this does not need.
fn toml_registers(text: &str, name: &str) -> bool {
    let plain = format!("[mcp_servers.{name}]");
    let quoted = format!("[mcp_servers.\"{name}\"]");
    text.lines().any(|line| {
        // A comment may follow the header on its line.
        let line = line.split('#').next().unwrap_or_default().trim();
        line == plain || line == quoted
    })
}

/// Run `program` without a shell and with no input, and give it `deadline` to finish; one that
/// does not is killed. `Ok((exited cleanly, stdout))`. `Err` when it could not be started
/// (including when it is not installed) or did not finish in time.
fn run_program(program: &str, args: &[&str], deadline: Duration) -> Result<(bool, String), String> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    // Read on a thread so a program that writes more than a pipe holds is not stuck waiting
    // for us while we wait for it.
    let mut stdout = child.stdout.take();
    // Handed back over a channel rather than joined: a grandchild that inherited the pipe keeps
    // it open after the program exits, and the read must not outlive the deadline with it.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(stdout) = stdout.as_mut() {
            let _ = stdout.read_to_end(&mut bytes);
        }
        let _ = tx.send(String::from_utf8_lossy(&bytes).to_string());
    });
    let end = std::time::Instant::now() + deadline;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() >= end => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{program} did not finish in {}s",
                    deadline.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("cannot wait for {program}: {e}")),
        }
    };
    let left = end.saturating_duration_since(std::time::Instant::now());
    let out = rx
        .recv_timeout(left.max(Duration::from_millis(100)))
        .map_err(|_| format!("{program} did not finish in {}s", deadline.as_secs()))?;
    Ok((status.success(), out))
}

/// The latest `MachineChecks` of a resident, shared with every board it opens so that a tmux
/// installed or upgraded while the server runs shows up without a restart.
#[derive(Default)]
pub struct MachineState {
    /// The repository the checks read the config for; empty outside one.
    nwo: String,
    latest: Mutex<Option<MachineChecks>>,
    /// Set while a check runs, so that two re-runs never overlap.
    running: AtomicBool,
}

/// Lets go of `MachineState::running` however the run ends.
struct Running<'a>(&'a AtomicBool);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl MachineState {
    pub fn new(nwo: &str, first: Option<MachineChecks>) -> Arc<MachineState> {
        Arc::new(MachineState {
            nwo: nwo.to_string(),
            latest: Mutex::new(first),
            running: AtomicBool::new(false),
        })
    }

    pub fn latest(&self) -> Option<MachineChecks> {
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The tmux version the board terminal may use, read when it is asked.
    pub fn tmux(&self) -> Option<(u32, u32)> {
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|checks| checks.tmux)
    }

    pub fn set(&self, checks: MachineChecks) {
        *self.latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(checks);
    }

    /// Run the check now and keep its result, unless one is already running: then the latest is
    /// returned as it is.
    pub fn refresh(&self) -> Option<MachineChecks> {
        self.run_unless_running(|| check_machine(&self.nwo))
    }

    /// `refresh`, unless the last result is less than ten seconds old: the page's button must
    /// not be a way to run `gh auth status` in a loop.
    pub fn recheck(&self) -> Option<MachineChecks> {
        let latest = self.latest();
        match &latest {
            Some(c) if crate::infra::clock::now_secs() - c.checked_at < RECHECK_AFTER_SECS => {
                latest
            }
            _ => self.refresh(),
        }
    }

    /// `refresh` with the check handed in, so that the guard can be tested.
    fn run_unless_running(&self, check: impl FnOnce() -> MachineChecks) -> Option<MachineChecks> {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return self.latest();
        }
        let _running = Running(&self.running);
        self.set(check());
        self.latest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn resolved(config: serde_json::Value) -> Resolved {
        let (registered, config_found, settings, warnings) =
            crate::kernel::config::resolve_from_value(
                &config,
                "acme/widget",
                &ResolveEnv::default(),
            );
        Resolved {
            registered,
            config_path: "/cfg/config.json".to_string(),
            warnings,
            config: config_found,
            settings,
        }
    }

    /// What the fake machine has: the versions programs print, or `None` for not installed.
    struct Machine {
        tmux: Option<&'static str>,
        gh: bool,
        auth: Result<bool, ()>,
    }

    impl Machine {
        fn healthy() -> Machine {
            Machine {
                tmux: Some("tmux 3.4"),
                gh: true,
                auth: Ok(true),
            }
        }
    }

    struct Dirs {
        _dir: tempfile::TempDir,
        claude: PathBuf,
        agy: PathBuf,
        codex: PathBuf,
    }

    fn dirs() -> Dirs {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        Dirs {
            claude: root.join("claude.json"),
            agy: root.join("mcp_config.json"),
            codex: root.join("config.toml"),
            _dir: dir,
        }
    }

    fn check_with(
        machine: &Machine,
        config: Result<serde_json::Value, String>,
        dirs: &Dirs,
    ) -> (MachineChecks, Vec<String>) {
        let calls = RefCell::new(Vec::new());
        let run = |program: &str, args: &[&str], _: Duration| {
            calls
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            match (program, args) {
                ("tmux", ["-V"]) => machine
                    .tmux
                    .map(|v| (true, format!("{v}\n")))
                    .ok_or("not installed".to_string()),
                ("gh", ["--version"]) if machine.gh => Ok((true, "gh version 2\n".to_string())),
                ("gh", ["auth", "status"]) => match machine.auth {
                    Ok(clean) => Ok((clean, "token: gho_secret".to_string())),
                    Err(()) => Err("timed out".to_string()),
                },
                _ => Err("not installed".to_string()),
            }
        };
        let resolve = |_: &str| config.clone().map(resolved);
        let checks = check_machine_with(
            "acme/widget",
            &Probes {
                run: &run,
                resolve: &resolve,
                claude_config: Some(dirs.claude.clone()),
                agy_config: Some(dirs.agy.clone()),
                codex_config: Some(dirs.codex.clone()),
            },
        );
        (checks, calls.into_inner())
    }

    fn find(checks: &MachineChecks, id: CheckId) -> Option<&Check> {
        checks.checks.iter().find(|c| c.id == id)
    }

    fn register_in_claude(dirs: &Dirs) {
        std::fs::write(
            &dirs.claude,
            r#"{"mcpServers":{"adjutant":{"command":"adj"}}}"#,
        )
        .unwrap();
    }

    #[test]
    fn a_healthy_machine_has_nothing_failing() {
        let dirs = dirs();
        register_in_claude(&dirs);
        let (checks, _) = check_with(&Machine::healthy(), Ok(serde_json::json!({})), &dirs);
        assert_eq!(checks.failing().count(), 0, "{checks:?}");
        assert_eq!(checks.tmux.is_some(), cfg!(unix));
        assert_eq!(checks.config_path, "/cfg/config.json");
    }

    #[test]
    fn a_config_that_does_not_parse_is_fatal_and_the_rest_is_still_checked() {
        let dirs = dirs();
        let err = "cannot parse /cfg/config.json: expected value at line 1".to_string();
        let (checks, calls) = check_with(&Machine::healthy(), Err(err.clone()), &dirs);
        let config = find(&checks, CheckId::Config).unwrap();
        assert!(!config.ok && config.fatal);
        assert_eq!(config.detail.as_deref(), Some(err.as_str()));
        assert_eq!(checks.fatal().count(), 1);
        assert!(find(&checks, CheckId::Tmux).unwrap().ok);
        assert!(calls.contains(&"gh auth status".to_string()), "{calls:?}");
        // The built-in settings say tmux, so the backend is the tmux one.
        assert!(checks.tmux_backend);
    }

    #[test]
    fn no_tmux_is_fatal_with_the_tmux_backend_and_a_warning_with_iterm2() {
        let dirs = dirs();
        let machine = Machine {
            tmux: None,
            ..Machine::healthy()
        };
        let (checks, _) = check_with(&machine, Ok(serde_json::json!({})), &dirs);
        let tmux = find(&checks, CheckId::Tmux).unwrap();
        assert!(!tmux.ok && tmux.fatal);
        assert!(find(&checks, CheckId::TmuxVersion).is_none());
        assert_eq!(checks.tmux, None);

        let iterm2 = serde_json::json!({"defaults": {"terminal": {"preset": "iterm2"}}});
        let (checks, _) = check_with(&machine, Ok(iterm2), &dirs);
        let tmux = find(&checks, CheckId::Tmux).unwrap();
        assert!(!tmux.ok && !tmux.fatal, "{tmux:?}");
        assert!(!checks.tmux_backend);
        assert_eq!(checks.fatal().count(), 0);
    }

    #[test]
    fn a_spawn_template_makes_a_missing_tmux_a_warning() {
        let dirs = dirs();
        let machine = Machine {
            tmux: None,
            ..Machine::healthy()
        };
        let config = serde_json::json!({"defaults": {"terminal": {"spawn": "true"}}});
        let (checks, _) = check_with(&machine, Ok(config), &dirs);
        assert!(!find(&checks, CheckId::Tmux).unwrap().fatal);
    }

    #[test]
    fn tmux_older_than_3_1_is_a_warning_and_offers_no_board_terminal() {
        let dirs = dirs();
        let machine = Machine {
            tmux: Some("tmux 2.9"),
            ..Machine::healthy()
        };
        let (checks, _) = check_with(&machine, Ok(serde_json::json!({})), &dirs);
        let version = find(&checks, CheckId::TmuxVersion).unwrap();
        assert!(!version.ok && !version.fatal);
        assert_eq!(version.detail.as_deref(), Some("2.9"));
        assert_eq!(checks.tmux, None);
    }

    #[test]
    #[cfg(unix)]
    fn tmux_3_4_offers_the_board_terminal() {
        let dirs = dirs();
        let (checks, _) = check_with(&Machine::healthy(), Ok(serde_json::json!({})), &dirs);
        assert_eq!(checks.tmux, Some((3, 4)));
        assert!(find(&checks, CheckId::TmuxVersion).unwrap().ok);
    }

    #[test]
    fn a_missing_gh_fails_and_its_login_is_not_asked() {
        let dirs = dirs();
        let machine = Machine {
            gh: false,
            ..Machine::healthy()
        };
        let (checks, calls) = check_with(&machine, Ok(serde_json::json!({})), &dirs);
        let gh = find(&checks, CheckId::Gh).unwrap();
        assert!(!gh.ok && !gh.fatal);
        assert!(find(&checks, CheckId::GhAuth).is_none());
        assert!(!calls.contains(&"gh auth status".to_string()), "{calls:?}");
    }

    #[test]
    fn gh_that_is_not_logged_in_fails_and_nothing_it_printed_is_kept() {
        let dirs = dirs();
        let machine = Machine {
            auth: Ok(false),
            ..Machine::healthy()
        };
        let (checks, _) = check_with(&machine, Ok(serde_json::json!({})), &dirs);
        let auth = find(&checks, CheckId::GhAuth).unwrap();
        assert!(!auth.ok && !auth.fatal);
        assert!(!format!("{checks:?}").contains("gho_secret"));
    }

    #[test]
    fn gh_that_does_not_answer_fails_and_says_so() {
        let dirs = dirs();
        let machine = Machine {
            auth: Err(()),
            ..Machine::healthy()
        };
        let (checks, _) = check_with(&machine, Ok(serde_json::json!({})), &dirs);
        let auth = find(&checks, CheckId::GhAuth).unwrap();
        assert!(!auth.ok);
        assert!(auth.detail.as_deref().unwrap().contains("did not answer"));
    }

    #[test]
    fn the_mcp_server_registered_in_any_one_agent_passes() {
        let healthy = Machine::healthy();
        let config = || Ok(serde_json::json!({}));

        let d = dirs();
        register_in_claude(&d);
        let (checks, _) = check_with(&healthy, config(), &d);
        let mcp = find(&checks, CheckId::Mcp).unwrap();
        assert!(mcp.ok);
        assert_eq!(mcp.detail.as_deref(), Some("Claude Code"));

        let d = dirs();
        std::fs::write(&d.agy, r#"{"mcpServers":{"adjutant":{}}}"#).unwrap();
        let (checks, _) = check_with(&healthy, config(), &d);
        assert_eq!(
            find(&checks, CheckId::Mcp).unwrap().detail.as_deref(),
            Some("Antigravity")
        );

        for header in [
            "[mcp_servers.adjutant]",
            "  [mcp_servers.\"adjutant\"]  ",
            "[mcp_servers.adjutant] # adj",
        ] {
            let d = dirs();
            std::fs::write(
                &d.codex,
                format!("model = \"x\"\n\n{header}\ncommand = \"adj\"\n"),
            )
            .unwrap();
            let (checks, _) = check_with(&healthy, config(), &d);
            assert_eq!(
                find(&checks, CheckId::Mcp).unwrap().detail.as_deref(),
                Some("Codex")
            );
        }

        let d = dirs();
        register_in_claude(&d);
        std::fs::write(&d.codex, "[mcp_servers.adjutant]\n").unwrap();
        let (checks, _) = check_with(&healthy, config(), &d);
        assert_eq!(
            find(&checks, CheckId::Mcp).unwrap().detail.as_deref(),
            Some("Claude Code, Codex")
        );
    }

    #[test]
    fn no_registration_fails_and_other_servers_do_not_count() {
        let d = dirs();
        std::fs::write(&d.claude, r#"{"mcpServers":{"other":{}}}"#).unwrap();
        std::fs::write(
            &d.codex,
            "[mcp_servers.adjutant_other]\n# [mcp_servers.adjutant]\n",
        )
        .unwrap();
        let (checks, _) = check_with(&Machine::healthy(), Ok(serde_json::json!({})), &d);
        let mcp = find(&checks, CheckId::Mcp).unwrap();
        assert!(!mcp.ok && !mcp.fatal);
        assert_eq!(
            mcp.detail, None,
            "files that are absent or read fine say nothing"
        );
    }

    #[test]
    fn a_config_file_that_cannot_be_parsed_is_named_not_read_as_not_registered() {
        let d = dirs();
        std::fs::write(&d.claude, "{ not json").unwrap();
        let (checks, _) = check_with(&Machine::healthy(), Ok(serde_json::json!({})), &d);
        let mcp = find(&checks, CheckId::Mcp).unwrap();
        assert!(!mcp.ok);
        let detail = mcp.detail.as_deref().unwrap();
        assert!(detail.contains("could not read"), "{detail}");
        assert!(detail.contains(&d.claude.display().to_string()), "{detail}");
    }

    #[test]
    fn a_registration_elsewhere_passes_even_when_one_file_cannot_be_read() {
        let d = dirs();
        std::fs::write(&d.claude, "{ not json").unwrap();
        std::fs::write(&d.agy, r#"{"mcpServers":{"adjutant":{}}}"#).unwrap();
        let (checks, _) = check_with(&Machine::healthy(), Ok(serde_json::json!({})), &d);
        assert!(find(&checks, CheckId::Mcp).unwrap().ok);
    }

    fn checks_at(checked_at: i64) -> MachineChecks {
        MachineChecks {
            checks: Vec::new(),
            tmux: Some((3, 4)),
            config_path: String::new(),
            tmux_backend: true,
            checked_at,
        }
    }

    #[test]
    fn a_state_with_no_result_has_no_tmux() {
        let state = MachineState::default();
        assert_eq!(state.tmux(), None);
        assert!(state.latest().is_none());
        state.set(checks_at(1));
        assert_eq!(state.tmux(), Some((3, 4)));
    }

    #[test]
    fn a_run_is_not_started_while_another_is_in_flight() {
        let state = MachineState::new("", Some(checks_at(5)));
        let inner = state.run_unless_running(|| {
            // The nested request finds the guard taken and gets the latest as it is.
            let seen = state.run_unless_running(|| panic!("two runs overlapped"));
            assert_eq!(seen.unwrap().checked_at, 5);
            checks_at(6)
        });
        assert_eq!(inner.unwrap().checked_at, 6);
        // The guard is let go afterwards.
        assert_eq!(
            state
                .run_unless_running(|| checks_at(7))
                .unwrap()
                .checked_at,
            7
        );
    }

    #[test]
    fn a_recheck_within_ten_seconds_gives_the_last_result() {
        let now = crate::infra::clock::now_secs();
        let state = MachineState::new("", Some(checks_at(now)));
        assert_eq!(state.recheck().unwrap().checked_at, now);
    }
}
