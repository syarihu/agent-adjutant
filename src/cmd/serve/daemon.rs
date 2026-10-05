//! The resident server process.

use std::io::IsTerminal;
use std::path::Path;

use serde_json::json;

use super::index::boards_json;
use super::resident;
use crate::board::{
    DEFAULT_PORT, bind_resident, board_url, checkout_here, launch_resident, resident_board_url,
    resident_root, stop_resident, stored_token, token,
};
use crate::registry::{live_resident, note_board, recorded_version};

/// `adj server start`. Detached unless `foreground`, which is what a service manager and the
/// tests run.
pub fn server_start(port: u16, foreground: bool, open: bool) -> Result<i32, String> {
    let here = checkout_here();
    let root = resident_root(here.as_ref());
    if foreground {
        return serve_resident(&root, port, open);
    }
    if let Some((pid, port)) = live_resident(&root) {
        if let Some(repo) = &here {
            note_board(&root, repo);
        }
        let shown = shown_url(&root, port, here.as_ref())?;
        println!("adj server: already running (pid {pid}) — {shown}");
        return Ok(0);
    }
    start_detached(&root, port, open, here.as_ref())
}

/// Start the resident detached and say where it is. The caller has already found none running.
fn start_detached(
    root: &Path,
    port: u16,
    open: bool,
    here: Option<&crate::kernel::identity::RepoInfo>,
) -> Result<i32, String> {
    let port = launch_resident(root, port)?;
    let index = resident_index_url(root, port)?;
    println!("adj server: serving on {index}");
    if let Some(repo) = here {
        note_board(root, repo);
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
fn shown_url(
    root: &Path,
    port: u16,
    here: Option<&crate::kernel::identity::RepoInfo>,
) -> Result<String, String> {
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
    bound.run(resident::handle_resident);
    Ok(0)
}

/// `adj server stop`. Only the server: a hub is a session of its own and goes on running.
pub fn server_stop() -> Result<i32, String> {
    let root = resident_root(checkout_here().as_ref());
    match stop_resident(&root, false)? {
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
    let root = resident_root(here.as_ref());
    let old_version = recorded_version(&root);
    let Some((old_pid, old_port)) = stop_resident(&root, true)? else {
        println!("adj server was not running; starting it");
        return start_detached(&root, port.unwrap_or(DEFAULT_PORT), open, here.as_ref());
    };
    println!("adj server: stopped pid {old_pid}");
    // Catches a supervisor that was quicker than this check and nothing more: one that
    // respawns after it still races the start below, so restart is not for a supervised server.
    if let Some((current, _)) = live_resident(&root) {
        println!("adj server: its supervisor already started it again (pid {current})");
        return Ok(0);
    }
    let wanted = port.unwrap_or(old_port);
    let bound = launch_resident(&root, wanted).map_err(|e| {
        format!(
            "stopped pid {old_pid}, but the new server did not start; nothing is running now: {e}"
        )
    })?;
    let new_pid = live_resident(&root).map_or(0, |(pid, _)| pid);
    if let Some(repo) = &here {
        note_board(&root, repo);
    }
    let shown = shown_url(&root, bound, here.as_ref())?;
    let version = match (old_version, recorded_version(&root)) {
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
    let root = resident_root(checkout_here().as_ref());
    let Some((pid, port)) = live_resident(&root) else {
        if as_json {
            println!("{}", json!({ "running": false }));
        } else {
            println!("adj server is not running");
        }
        return Ok(1);
    };
    let token = stored_token(&root).ok_or("the dashboard token is missing")?;
    let index = board_url(port, &token);
    let boards = boards_json(&root, port, &token);
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

pub(super) fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    let _ = std::process::Command::new(opener).arg(url).spawn();
}
