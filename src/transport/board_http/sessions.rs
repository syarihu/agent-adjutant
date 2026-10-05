//! The session routes whose operations answer a value rather than JSON: reopening, restarting
//! and opening a session, and its worktree's git state. Each reads the request, calls the
//! board, and words the answer.

use serde_json::{Value, json};

use crate::board::session::{OpenedIn, open, restart, resume};
use crate::board::view::{find_session, session_git};
use crate::board::{Server, input_of, settings_now};

/// The body is refused before the session is looked up, as the operation did when it read it.
pub(super) fn resume_session(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    input_of(body)?;
    let done = resume(server, id)?;
    Ok(json!({
        "resumed": true,
        "description": done.description,
        "hub": done.hub,
        "hubRunning": done.hub_running,
    }))
}

pub(super) fn restart_session(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    input_of(body)?;
    let done = restart(server, id)?;
    Ok(json!({
        "restarted": true,
        "wasRunning": done.was_running,
        "description": done.description,
        "hub": done.hub,
        "hubRunning": done.hub_running,
    }))
}

pub(super) fn open_session(server: &Server, id: &str) -> Result<Value, String> {
    let opened = open(server, id)?;
    let terminal = match opened.terminal {
        OpenedIn::Attach => "terminal.attach",
        OpenedIn::ITerm2 => "iTerm2",
    };
    Ok(json!({
        "opened": true,
        "description": format!("opened {id} in {terminal}"),
        "session": opened.session,
        "window": opened.window,
    }))
}

pub(super) fn git_of_session(server: &Server, id: &str) -> Result<Value, String> {
    match session_git(server, id)? {
        Some(state) => {
            serde_json::to_value(state).map_err(|e| format!("cannot describe the worktree: {e}"))
        }
        // Only a gone worktree comes here, so the usual path still looks the session up once.
        None => {
            let session = find_session(server, &settings_now(server), id)?;
            Err(format!("{} does not exist", session.worktree))
        }
    }
}
