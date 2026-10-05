use std::path::Path;

use crate::registry::live_resident;

/// The resident that is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub pid: u32,
    pub port: u16,
    /// `None` when the token file is missing; the caller decides that is an error.
    pub token: Option<String>,
}

/// The running resident, or `None` when there is none.
pub fn status(root: &Path) -> Option<Status> {
    let (pid, port) = live_resident(root)?;
    Some(Status {
        pid,
        port,
        token: crate::board::stored_token(root),
    })
}
