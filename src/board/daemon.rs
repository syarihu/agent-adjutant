use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{Resident, board_url, jobs, token};
use crate::gate;
use crate::registry::{
    addresses, forget_server, live_resident, note_board, record_server, recorded_server,
};
use crate::task;

mod restart;
mod start;
mod status;
mod stop;
pub use restart::*;
pub use start::*;
pub use status::*;
pub use stop::*;

pub const DEFAULT_PORT: u16 = 4577;

/// `port` on the loopback address, or any free port when `port` is taken — a second hub of
/// the same repository, a hub of another one, or something that is not ours at all. Only
/// "in use" falls back: any other failure would fail on a free port too.
pub fn bind_preferring(port: u16) -> std::io::Result<TcpListener> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => TcpListener::bind(("127.0.0.1", 0)),
        bound => bound,
    }
}

// ── the resident server ──────────────────────────────────────────────
//
// One process per state directory that serves every repository's board, each under
// `/b/<slug>/`, whether or not a hub is running. It finds the repositories through an
// address book — `boards/<slug>.json`, written by whoever learns where a repository is —
// and builds each board's context from that on first use.

fn server_lock_path(root: &Path) -> PathBuf {
    root.join("server.lock")
}

fn server_log_path(root: &Path) -> PathBuf {
    root.join("server.log")
}

/// The state directory the resident commands read: a relative `ADJUTANT_STATE_DIR` is taken
/// against the main checkout of the repository this was typed in, as every command of that
/// repository takes it, and outside any against the working directory.
pub fn resident_root(here: Option<&crate::kernel::identity::RepoInfo>) -> PathBuf {
    crate::registry::state_root(here.map(|repo| Path::new(&repo.main)))
}

/// Spawn the resident and wait until it says where it is: the port it bound.
pub fn launch_resident(root: &Path, port: u16) -> Result<u16, String> {
    let child = spawn_resident(root, port)?;
    wait_for_resident(root, child)
}

/// `path` opened for appending, readable by its owner alone. The log is written by a server
/// that holds a secret, and a file another user on the machine can read is a way to it — so a
/// log an earlier version made with looser permissions is tightened too.
pub fn private_log(path: &Path) -> Result<std::fs::File, String> {
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
fn spawn_resident(root: &Path, port: u16) -> Result<std::process::Child, String> {
    let log = server_log_path(root);
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let open_log = || private_log(&log);
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this binary: {e}"))?;
    resident_command(&exe, port, root, open_log()?.into(), open_log()?.into())
        .spawn()
        .map_err(|e| format!("cannot start adj server: {e}"))
}

/// The command that starts the resident, built apart from the spawn so its environment can be
/// read. The resident is detached and stands elsewhere, so a relative `ADJUTANT_STATE_DIR` is
/// handed to it as `root`, the absolute directory this process reads.
pub fn resident_command(
    exe: &Path,
    port: u16,
    root: &Path,
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
        command.env(crate::infra::env::STATE_DIR_ENV, root);
    }
    command
}

/// Wait for the resident to say where it is — it writes `server.json` once it is listening.
fn wait_for_resident(root: &Path, mut child: std::process::Child) -> Result<u16, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some((_, port)) = live_resident(root) {
            return Ok(port);
        }
        // A child that has exited is not yet a failure: it exits when another resident holds
        // the lock, and that one may be a moment from writing its record. Asked for its status
        // either way, which is also what reaps it.
        let _ = child.try_wait();
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "adj server did not start; see {}",
                server_log_path(root).display()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// The resident, locked, bound and recorded but not yet answering. Holds `server.lock` for as
/// long as it lives.
pub struct BoundResident {
    lock: std::fs::File,
    listener: TcpListener,
    root: PathBuf,
    token: String,
    pub asked: u16,
    pub bound: u16,
}

/// Take `server.lock`, bind `port` (or any free port when it is taken), and write the record
/// that says where the resident is. Prints nothing.
pub fn bind_resident(root: &Path, port: u16) -> Result<BoundResident, String> {
    let lock_path = server_lock_path(root);
    // Held for the process's lifetime and released by the system when it ends, however it
    // ends — the same lock `registry::take_over` takes, for the same reason.
    let Some(lock) = crate::infra::fs::try_lock(&lock_path)? else {
        return Err(match live_resident(root) {
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
    let token = token(root)?;
    record_server(root, bound)?;
    seed_boards(root);
    Ok(BoundResident {
        lock,
        listener,
        root: root.to_path_buf(),
        token,
        asked: port,
        bound,
    })
}

impl BoundResident {
    pub fn index_url(&self) -> String {
        board_url(self.bound, &self.token)
    }

    /// Poll the pull requests, sweep the gates and answer connections with `handle`, for as long
    /// as the process lives.
    pub fn run(self, handle: fn(&Resident, TcpStream) -> std::io::Result<()>) {
        let BoundResident {
            lock,
            listener,
            root,
            token,
            bound,
            ..
        } = self;
        // Built here rather than in `bind_resident`: asking tmux for its version is a process to
        // wait for, and neither the record (a starting `adj server start` waits for it) nor the
        // serving line (read from the log right after the record) should wait on it.
        let resident = Arc::new(Resident {
            root,
            token,
            port: bound,
            boards: Mutex::default(),
            tmux: board_terminal_tmux(),
            terminals: Arc::default(),
            pr_poll: Arc::default(),
            waits: Arc::default(),
        });
        {
            let resident = Arc::clone(&resident);
            let poll = Arc::clone(&resident.pr_poll);
            std::thread::spawn(move || {
                poll.run(|| {
                    let all = addresses(&resident.root);
                    // Only a board with a card on a PR is opened for it: opening one asks git
                    // where the checkout is, which is not worth doing every round for a board
                    // with nothing to look after.
                    let cards = all
                        .iter()
                        .filter(|a| {
                            task::list(&resident.root, &a.slug).iter().any(|t| {
                                t.pr.is_some()
                                    && !matches!(
                                        t.status,
                                        task::Status::Done | task::Status::Cancelled
                                    )
                            })
                        })
                        .filter_map(|a| resident.board(&a.slug))
                        .map(|server| server.ctx.clone())
                        .collect();
                    // The branches of a repository's sessions are the same whichever of its
                    // boards is asked, and the address already names the checkout: one board is
                    // opened for each distinct checkout (the repository's own, else the first
                    // of its hubs), and a board already open is only looked up.
                    let mut mains: Vec<&str> = Vec::new();
                    let mut branches = Vec::new();
                    for a in all.iter().filter(|a| a.hub.is_none()).chain(all.iter()) {
                        if mains.contains(&a.main.as_str()) {
                            continue;
                        }
                        if let Some(server) = resident.board(&a.slug) {
                            mains.push(&a.main);
                            branches.push(server.ctx.clone());
                        }
                    }
                    jobs::PollBoards { cards, branches }
                });
            });
        }
        {
            let resident = Arc::clone(&resident);
            std::thread::spawn(move || {
                jobs::sweep_gates::run(|| {
                    // Only a board holding a gate a worker opened and waits on is opened for it:
                    // opening one asks git where the checkout is, which is not worth doing every
                    // two seconds for a board with nothing to sweep.
                    addresses(&resident.root)
                        .iter()
                        .filter(|a| {
                            gate::list(&resident.root, &a.slug, gate::Shelf::Open)
                                .iter()
                                .any(|g| g.wait && !g.answered_by_hub())
                        })
                        .filter_map(|a| resident.board(&a.slug))
                        .map(|server| server.ctx.clone())
                        .collect()
                });
            });
        }
        {
            let resident = Arc::clone(&resident);
            std::thread::spawn(move || {
                let waits = Arc::clone(&resident.waits);
                waits.run(&resident.root.clone(), || {
                    // Asked only when a wait is due, so a board is opened (which asks git where
                    // its checkout is) for a wait and not every two seconds.
                    let addresses = addresses(&resident.root);
                    let boards: Vec<_> = addresses
                        .iter()
                        .filter_map(|a| resident.board(&a.slug))
                        .collect();
                    // A board that could not be opened is a board not looked at.
                    let complete = boards.len() == addresses.len();
                    (boards, complete)
                });
            });
        }
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let resident = Arc::clone(&resident);
                    // A thread per connection, for the reason `Board::run` gives.
                    std::thread::spawn(move || {
                        if let Err(e) = handle(&resident, stream) {
                            eprintln!("adj server: connection error: {e}");
                        }
                    });
                }
                Err(e) => eprintln!("adj server: accept error: {e}"),
            }
        }
        drop(lock);
    }
}

/// How long a stop waits for the resident to exit before giving up.
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Ask the resident to exit and wait for it. The pid and bound port of what was stopped, or
/// `None` when nothing was running — a record left by a killed server is forgotten on the way.
/// `restarting` only changes the timeout message, which then says no other is started.
pub fn stop_resident(root: &Path, restarting: bool) -> Result<Option<(u32, u16)>, String> {
    let named = recorded_server(root);
    let Some((pid, port)) = live_resident(root) else {
        if let Some((pid, started)) = &named {
            forget_server(root, *pid, started.as_deref());
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
        Some(started) => crate::registry::ps_started(pid).as_deref() == Some(started.as_str()),
        None => live_resident(root).is_some_and(|(p, _)| p == pid),
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
    forget_server(root, pid, started.as_deref());
    Ok(Some((pid, port)))
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

/// The repository this process stands in, if it stands in one. Whether it is one is not the
/// business of the commands that ask.
pub fn checkout_here() -> Option<crate::kernel::identity::RepoInfo> {
    crate::registry::resolve(None, None).ok()
}

/// Seed the address book from where this process stands and from the hub records already on
/// disk, so that the boards of hubs started before the resident are there from the first
/// request.
pub(super) fn seed_boards(root: &Path) {
    if let Some(repo) = checkout_here() {
        note_board(root, &repo);
    }
    for (slug, record) in crate::registry::hub_records(root) {
        let Some(cwd) = record.cwd.as_deref() else {
            continue;
        };
        if let Ok(repo) =
            crate::kernel::identity::resolve_in(Some(Path::new(cwd)), None, record.hub.as_deref())
            && repo.slug == slug
        {
            note_board(root, &repo);
        }
    }
}
