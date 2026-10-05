use std::path::Path;

use crate::kernel::identity::RepoInfo;
use crate::registry::{live_resident, note_board};

/// What `start` found or did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    /// A resident was already running, on this pid and port.
    Already { pid: u32, port: u16 },
    /// None was running, so one was launched; this is the port it bound.
    Launched { port: u16 },
}

/// Start the resident unless one runs, and note `here`'s board either way. Prints nothing.
pub fn start(root: &Path, port: u16, here: Option<&RepoInfo>) -> Result<Started, String> {
    let started = match live_resident(root) {
        Some((pid, port)) => Started::Already { pid, port },
        None => Started::Launched {
            port: crate::board::launch_resident(root, port)?,
        },
    };
    if let Some(repo) = here {
        note_board(root, repo);
    }
    Ok(started)
}
