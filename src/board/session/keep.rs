use std::path::Path;

use crate::board::view::board_session;
use crate::board::{Server, settings_now};
use crate::registry;

/// How a request to keep a session came out.
pub enum Keep {
    Kept,
    /// The board has no session by that id.
    NoSuchSession,
    /// A session that is not a worker's, or has no worktree, has nothing to keep.
    Refused(String),
    /// The worker's record could not be written.
    Failed(String),
}

/// Write down that the person chose to keep the worker session `id` instead of cleaning it up.
///
/// The session is looked up in the board's own records and the worktree comes from there, never
/// from the request. Everything is checked before the record is touched, so a refusal leaves
/// nothing behind.
pub fn keep_session(server: &Server, id: &str) -> Keep {
    let settings = settings_now(server);
    let Some(session) = board_session(server, &settings, id) else {
        return Keep::NoSuchSession;
    };
    if session.kind != "worker" {
        return Keep::Refused("only a worker session can be kept".to_string());
    }
    if session.worktree.trim().is_empty() {
        return Keep::Refused("the session has no worktree".to_string());
    }
    match registry::keep_from_cleanup(Path::new(&session.worktree)) {
        Ok(()) => Keep::Kept,
        Err(e) => Keep::Failed(e),
    }
}
