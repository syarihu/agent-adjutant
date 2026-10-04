//! The resident server process.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::{messaging, task};

use super::auth::{stored_token, token};
use super::index::{boards_json, checkout_here, seed_boards};
use super::registry::{addresses, board_url, note_board, resident_board_url};
use super::resident::{Resident, handle_resident};
use super::{DEFAULT_PORT, bind_preferring};

// Moved to `registry`; re-exported until #331 so the `serve` callers keep their paths.
pub(super) use crate::registry::live_resident;
pub use crate::registry::resident_running;
use crate::registry::{recorded_version, server_record_path};

// ── the resident server ──────────────────────────────────────────────
//
// One process per state directory that serves every repository's board, each under
// `/b/<slug>/`, whether or not a hub is running. It finds the repositories through an
// address book — `boards/<slug>.json`, written by whoever learns where a repository is —
// and builds each board's context from that on first use.

fn server_lock_path() -> PathBuf {
    crate::infra::paths::state_dir().join("server.lock")
}

fn server_log_path() -> PathBuf {
    crate::infra::paths::state_dir().join("server.log")
}

/// `adj server start`. Detached unless `foreground`, which is what a service manager and the
/// tests run.
pub fn server_start(port: u16, foreground: bool, open: bool) -> Result<i32, String> {
    if foreground {
        return serve_resident(port, open);
    }
    let here = checkout_here();
    if let Some((pid, port)) = live_resident() {
        if let Some(repo) = &here {
            note_board(repo);
        }
        let shown = shown_url(port, here.as_ref())?;
        println!("adj server: already running (pid {pid}) — {shown}");
        return Ok(0);
    }
    start_detached(port, open, here.as_ref())
}

/// Start the resident detached and say where it is. The caller has already found none running.
fn start_detached(
    port: u16,
    open: bool,
    here: Option<&crate::kernel::identity::RepoInfo>,
) -> Result<i32, String> {
    let port = launch_resident(port)?;
    let index = resident_index_url(port)?;
    println!("adj server: serving on {index}");
    if let Some(repo) = here {
        note_board(repo);
        println!("adj server: {} — {}", repo.nwo, shown_url(port, here)?);
    }
    if open {
        open_browser(&shown_url(port, here)?);
    }
    Ok(0)
}

/// Spawn the resident and wait until it says where it is: the port it bound.
fn launch_resident(port: u16) -> Result<u16, String> {
    let child = spawn_resident(port)?;
    wait_for_resident(child)
}

fn resident_index_url(port: u16) -> Result<String, String> {
    Ok(board_url(port, &token()?))
}

/// The URL worth showing: the board of the checkout this stands in, else the index.
fn shown_url(
    port: u16,
    here: Option<&crate::kernel::identity::RepoInfo>,
) -> Result<String, String> {
    match here {
        Some(repo) => Ok(resident_board_url(port, &repo.slug, &token()?)),
        None => resident_index_url(port),
    }
}

/// `path` opened for appending, readable by its owner alone. The log is written by a server
/// that holds a secret, and a file another user on the machine can read is a way to it — so a
/// log an earlier version made with looser permissions is tightened too.
pub(super) fn private_log(path: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("cannot restrict {}: {e}", path.display()))?;
    Ok(file)
}

/// The resident, started in a process group of its own so that closing the terminal it was
/// started from does not take it along, with what it says written to `server.log`.
fn spawn_resident(port: u16) -> Result<std::process::Child, String> {
    let log = server_log_path();
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let open_log = || private_log(&log);
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this binary: {e}"))?;
    resident_command(&exe, port, open_log()?.into(), open_log()?.into())
        .spawn()
        .map_err(|e| format!("cannot start adj server: {e}"))
}

/// The command that starts the resident, built apart from the spawn so its environment can be
/// read. The resident is detached and stands elsewhere, so a relative `ADJUTANT_STATE_DIR` is
/// handed to it as the absolute directory this process reads now.
pub(in crate::cmd) fn resident_command(
    exe: &Path,
    port: u16,
    stdout: std::process::Stdio,
    stderr: std::process::Stdio,
) -> std::process::Command {
    use std::os::unix::process::CommandExt;

    let mut command = std::process::Command::new(exe);
    command
        .args(["server", "start", "--foreground", "--no-open", "--port"])
        .arg(port.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        // What the agent is started without, for the same reason: this one answers for every
        // repository and must not inherit the identity of the hub it was started from.
        .env_remove(crate::infra::env::HUB_SESSION_ENV)
        .env_remove(crate::infra::env::HUB_SERVE_ENV)
        .env_remove(crate::infra::env::HUB_ENV)
        .process_group(0);
    for name in crate::infra::git::REPOSITORY_LOCATION_ENV {
        command.env_remove(name);
    }
    if std::env::var_os(crate::infra::env::STATE_DIR_ENV).is_some_and(|v| !v.is_empty()) {
        command.env(
            crate::infra::env::STATE_DIR_ENV,
            crate::infra::paths::state_dir_absolute(),
        );
    }
    command
}

/// Wait for the resident to say where it is — it writes `server.json` once it is listening.
fn wait_for_resident(mut child: std::process::Child) -> Result<u16, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some((_, port)) = live_resident() {
            return Ok(port);
        }
        // A child that has exited is not yet a failure: it exits when another resident holds
        // the lock, and that one may be a moment from writing its record. Asked for its status
        // either way, which is also what reaps it.
        let _ = child.try_wait();
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "adj server did not start; see {}",
                server_log_path().display()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// The resident itself: holds `server.lock` for as long as it runs, binds, says where it is,
/// and answers.
fn serve_resident(port: u16, open: bool) -> Result<i32, String> {
    let lock_path = server_lock_path();
    // Held for the process's lifetime and released by the system when it ends, however it
    // ends — the same lock `messaging::take_over` takes, for the same reason.
    let Some(lock) = crate::infra::fs::try_lock(&lock_path)? else {
        return Err(match live_resident() {
            Some((pid, _)) => format!("another adj server is running (pid {pid})"),
            None => "another adj server is running".to_string(),
        });
    };
    let listener =
        bind_preferring(port).map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))?;
    let bound = listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| format!("cannot read the server's port: {e}"))?;
    if port != 0 && bound != port {
        eprintln!("adj server: 127.0.0.1:{port} is taken; serving on {bound} instead");
    }
    let token = token()?;
    let pid = std::process::id();
    let record = json!({
        "pid": pid,
        "psStarted": messaging::ps_started(pid),
        "port": bound,
        "startedAt": crate::infra::clock::utc_stamp(crate::infra::clock::now_secs()),
        "version": env!("CARGO_PKG_VERSION"),
    });
    crate::infra::fs::write_json(&server_record_path(), &record)?;
    seed_boards();
    let index = board_url(bound, &token);
    // The token goes to a terminal and nowhere else: detached, or under a service manager,
    // stdout is a log file, and a log is kept, attached to bug reports and read by others. The
    // process that started it prints the whole URL to the person who asked.
    if std::io::stdout().is_terminal() {
        println!("adj server: serving on {index}");
    } else {
        println!("adj server: serving on http://127.0.0.1:{bound}/");
    }
    if open {
        open_browser(&index);
    }
    let resident = Arc::new(Resident {
        token,
        port: bound,
        boards: Mutex::default(),
        tmux: board_terminal_tmux(),
        terminals: Arc::default(),
        pr_poll: Arc::default(),
    });
    {
        let resident = Arc::clone(&resident);
        let poll = Arc::clone(&resident.pr_poll);
        std::thread::spawn(move || {
            poll.run(|| {
                // Only a board with a card on a PR is opened for it: opening one asks git where
                // the checkout is, which is not worth doing every round for a board with nothing
                // to look after.
                let state_dir = crate::infra::paths::state_dir();
                addresses()
                    .iter()
                    .filter(|a| {
                        task::list(&task::dir(&state_dir, &a.slug)).iter().any(|t| {
                            t.pr.is_some()
                                && !matches!(t.status, task::Status::Done | task::Status::Cancelled)
                        })
                    })
                    .filter_map(|a| resident.board(&a.slug))
                    .map(|server| server.ctx.clone())
                    .collect()
            });
        });
    }
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let resident = Arc::clone(&resident);
                // A thread per connection, for the reason `Board::run` gives.
                std::thread::spawn(move || {
                    if let Err(e) = handle_resident(&resident, stream) {
                        eprintln!("adj server: connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("adj server: accept error: {e}"),
        }
    }
    drop(lock);
    Ok(0)
}

/// What `server.json` names, whether or not that process is still there.
fn recorded_resident() -> Option<(u32, Option<String>)> {
    let record = crate::infra::fs::read_json(&server_record_path())?;
    let pid = record.get("pid").and_then(Value::as_u64)? as u32;
    let started = record
        .get("psStarted")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((pid, started))
}

/// Whether `record` still names the process `pid` started at `started`. The rule for removing
/// `server.json`: a supervisor that restarts the server may already have written the next
/// record, and that one is not ours to remove.
pub(super) fn names_resident(
    record: Option<&(u32, Option<String>)>,
    pid: u32,
    started: Option<&str>,
) -> bool {
    record.is_some_and(|(p, s)| *p == pid && s.as_deref() == started)
}

fn forget_resident(pid: u32, started: Option<&str>) {
    if names_resident(recorded_resident().as_ref(), pid, started) {
        let _ = std::fs::remove_file(server_record_path());
    }
}

/// How long a stop waits for the resident to exit before giving up.
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Ask the resident to exit and wait for it. The pid and bound port of what was stopped, or
/// `None` when nothing was running — a record left by a killed server is forgotten on the way.
/// `restarting` only changes the timeout message, which then says no other is started.
fn stop_resident(restarting: bool) -> Result<Option<(u32, u16)>, String> {
    let named = recorded_resident();
    let Some((pid, port)) = live_resident() else {
        if let Some((pid, started)) = &named {
            forget_resident(*pid, started.as_deref());
        }
        return Ok(None);
    };
    let started = named.and_then(|(_, s)| s);
    let status = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .map_err(|e| format!("cannot run kill: {e}"))?;
    if !status.success() {
        return Err(format!("cannot stop adj server (pid {pid})"));
    }
    let deadline = std::time::Instant::now() + STOP_WAIT;
    // The process this record named, not whatever the record names by now: a supervisor may
    // already have started the next one.
    let still_there = || match &started {
        Some(started) => messaging::ps_started(pid).as_deref() == Some(started.as_str()),
        None => live_resident().is_some_and(|(p, _)| p == pid),
    };
    while still_there() {
        if std::time::Instant::now() >= deadline {
            let tail = if restarting {
                format!("; not starting another (kill -KILL {pid} to force)")
            } else {
                String::new()
            };
            return Err(format!(
                "adj server (pid {pid}) did not stop within {}s{tail}",
                STOP_WAIT.as_secs()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    forget_resident(pid, started.as_deref());
    Ok(Some((pid, port)))
}

/// `adj server stop`. Only the server: a hub is a session of its own and goes on running.
pub fn server_stop() -> Result<i32, String> {
    match stop_resident(false)? {
        Some((pid, _)) => println!("stopped adj server (pid {pid})"),
        None => println!("adj server is not running"),
    }
    Ok(0)
}

/// `adj server restart`: stop the resident and start it again, on the port it had unless
/// `port` says otherwise. The new one is this binary, which is what lets a reinstall take
/// effect. Hubs and workers are other processes and go on running; board tabs reconnect by
/// themselves, which is why the browser is opened only on request.
pub fn server_restart(port: Option<u16>, open: bool) -> Result<i32, String> {
    let here = checkout_here();
    let old_version = recorded_version();
    let Some((old_pid, old_port)) = stop_resident(true)? else {
        println!("adj server was not running; starting it");
        return start_detached(port.unwrap_or(DEFAULT_PORT), open, here.as_ref());
    };
    println!("adj server: stopped pid {old_pid}");
    // Catches a supervisor that was quicker than this check and nothing more: one that
    // respawns after it still races the start below, so restart is not for a supervised server.
    if let Some((current, _)) = live_resident() {
        println!("adj server: its supervisor already started it again (pid {current})");
        return Ok(0);
    }
    let wanted = port.unwrap_or(old_port);
    let bound = launch_resident(wanted).map_err(|e| {
        format!(
            "stopped pid {old_pid}, but the new server did not start; nothing is running now: {e}"
        )
    })?;
    let new_pid = live_resident().map_or(0, |(pid, _)| pid);
    if let Some(repo) = &here {
        note_board(repo);
    }
    let shown = shown_url(bound, here.as_ref())?;
    let version = match (old_version, recorded_version()) {
        (Some(old), Some(new)) if old != new => format!(" ({old} -> {new})"),
        _ => String::new(),
    };
    println!("adj server: restarted (pid {new_pid}){version} — {shown}");
    // 0 asks for any port, so there is nothing for the bound one to differ from.
    if wanted != 0 && bound != wanted {
        println!("adj server: port {wanted} was taken; now on {bound}");
    }
    if open {
        open_browser(&shown);
    }
    Ok(0)
}

/// `adj server status`. Exit 1 when there is no resident, so a script can ask.
pub fn server_status(as_json: bool) -> Result<i32, String> {
    let Some((pid, port)) = live_resident() else {
        if as_json {
            println!("{}", json!({ "running": false }));
        } else {
            println!("adj server is not running");
        }
        return Ok(1);
    };
    let token = stored_token().ok_or("the dashboard token is missing")?;
    let index = board_url(port, &token);
    let boards = boards_json(port, &token);
    if as_json {
        let out = json!({
            "running": true,
            "pid": pid,
            "port": port,
            "url": index,
            "boards": boards,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(0);
    }
    println!("adj server: running (pid {pid}) on port {port}");
    println!("index: {index}");
    for board in &boards {
        let hub = board["hub"].as_str().unwrap_or("-");
        let state = if board["hubPresent"].as_bool().unwrap_or(false) {
            "running"
        } else {
            "stopped"
        };
        println!(
            "{} (hub {hub}, {state}) — {}",
            board["nwo"].as_str().unwrap_or_default(),
            board["url"].as_str().unwrap_or_default()
        );
    }
    Ok(0)
}

/// The tmux version when the board terminal can be offered at all: a unix machine with tmux 3.1
/// or later. Older tmux has neither `window-size latest` nor the hook that ends the terminal with
/// its window, so the board would follow tmux on to another agent's window when the target closes.
fn board_terminal_tmux() -> Option<(u32, u32)> {
    #[cfg(unix)]
    {
        crate::infra::terminal::tmux_version().filter(|&v| v >= (3, 1))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

pub(super) fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    let _ = std::process::Command::new(opener).arg(url).spawn();
}
