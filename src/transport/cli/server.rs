//! The resident server process.

use std::io::IsTerminal;
use std::path::Path;

use serde_json::json;

use crate::board::view::boards;
use crate::board::{
    self, RestartFailed, Restarted, Started, Status, bind_resident, board_url, checkout_here,
    resident_board_url, resident_root, token,
};
use crate::kernel::identity::RepoInfo;
use crate::transport::board_http::handle_resident;

/// `adj server start`. Detached unless `foreground`, which is what a service manager and the
/// tests run.
pub fn server_start(port: u16, foreground: bool, open: bool) -> Result<i32, String> {
    let here = checkout_here();
    let root = resident_root(here.as_ref());
    if foreground {
        return serve_resident(&root, port, open);
    }
    match board::start(&root, port, here.as_ref())? {
        Started::Already { pid, port } => {
            let shown = shown_url(&root, port, here.as_ref())?;
            println!("adj server: already running (pid {pid}) — {shown}");
            Ok(0)
        }
        Started::Launched { port } => say_serving(&root, port, open, here.as_ref()),
    }
}

/// Start the resident detached and say where it is.
fn start_detached(
    root: &Path,
    port: u16,
    open: bool,
    here: Option<&RepoInfo>,
) -> Result<i32, String> {
    // Already: another resident won the race since the caller looked, and it is serving all the
    // same.
    let port = match board::start(root, port, here)? {
        Started::Already { port, .. } | Started::Launched { port } => port,
    };
    say_serving(root, port, open, here)
}

/// What a start that launched says.
fn say_serving(root: &Path, port: u16, open: bool, here: Option<&RepoInfo>) -> Result<i32, String> {
    let index = resident_index_url(root, port)?;
    println!("adj server: serving on {index}");
    if let Some(repo) = here {
        println!(
            "adj server: {} — {}",
            repo.nwo,
            shown_url(root, port, here)?
        );
    }
    if open {
        open_browser(&shown_url(root, port, here)?);
    }
    Ok(0)
}

fn resident_index_url(root: &Path, port: u16) -> Result<String, String> {
    Ok(board_url(port, &token(root)?))
}

/// The URL worth showing: the board of the checkout this stands in, else the index.
fn shown_url(root: &Path, port: u16, here: Option<&RepoInfo>) -> Result<String, String> {
    match here {
        Some(repo) => Ok(resident_board_url(port, &repo.slug, &token(root)?)),
        None => resident_index_url(root, port),
    }
}

/// The resident itself: holds `server.lock` for as long as it runs, binds, says where it is,
/// and answers.
fn serve_resident(root: &Path, port: u16, open: bool) -> Result<i32, String> {
    let bound = bind_resident(root, port)?;
    let (asked, on) = (bound.asked, bound.bound);
    if asked != 0 && on != asked {
        eprintln!("adj server: 127.0.0.1:{asked} is taken; serving on {on} instead");
    }
    let index = bound.index_url();
    // The token goes to a terminal and nowhere else: detached, or under a service manager,
    // stdout is a log file, and a log is kept, attached to bug reports and read by others. The
    // process that started it prints the whole URL to the person who asked.
    if std::io::stdout().is_terminal() {
        println!("adj server: serving on {index}");
    } else {
        println!("adj server: serving on http://127.0.0.1:{on}/");
    }
    if open {
        open_browser(&index);
    }
    bound.run(handle_resident);
    Ok(0)
}

/// `adj server stop`. Only the server: a hub is a session of its own and goes on running.
pub fn server_stop() -> Result<i32, String> {
    let root = resident_root(checkout_here().as_ref());
    match board::stop(&root)? {
        Some(board::Stopped { pid, .. }) => println!("stopped adj server (pid {pid})"),
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
    let root = resident_root(here.as_ref());
    let restarted = board::restart(&root, port, here.as_ref()).map_err(|failed| {
        if let RestartFailed::Start { stopped, .. } = &failed {
            println!("adj server: stopped pid {stopped}");
        }
        failed.to_string()
    })?;
    match restarted {
        Restarted::WasNotRunning { port } => {
            println!("adj server was not running; starting it");
            start_detached(&root, port, open, here.as_ref())
        }
        Restarted::Supervised { stopped, current } => {
            println!("adj server: stopped pid {stopped}");
            println!("adj server: its supervisor already started it again (pid {current})");
            Ok(0)
        }
        Restarted::Replaced {
            stopped,
            pid,
            wanted,
            bound,
            versions,
        } => {
            println!("adj server: stopped pid {stopped}");
            let shown = shown_url(&root, bound, here.as_ref())?;
            let version =
                versions.map_or_else(String::new, |(old, new)| format!(" ({old} -> {new})"));
            println!("adj server: restarted (pid {pid}){version} — {shown}");
            // 0 asks for any port, so there is nothing for the bound one to differ from.
            if wanted != 0 && bound != wanted {
                println!("adj server: port {wanted} was taken; now on {bound}");
            }
            if open {
                open_browser(&shown);
            }
            Ok(0)
        }
    }
}

/// `adj server status`. Exit 1 when there is no resident, so a script can ask.
pub fn server_status(as_json: bool) -> Result<i32, String> {
    let root = resident_root(checkout_here().as_ref());
    let Some(Status { pid, port, token }) = board::status(&root) else {
        if as_json {
            println!("{}", json!({ "running": false }));
        } else {
            println!("adj server is not running");
        }
        return Ok(1);
    };
    let token = token.ok_or("the dashboard token is missing")?;
    let index = board_url(port, &token);
    let boards = boards(&root, port, &token);
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
    for row in &boards {
        let hub = row.hub.as_deref().unwrap_or("-");
        let state = if row.hub_present {
            "running"
        } else {
            "stopped"
        };
        println!("{} (hub {hub}, {state}) — {}", row.nwo, row.url);
    }
    Ok(0)
}

pub(super) fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    let _ = std::process::Command::new(opener).arg(url).spawn();
}
