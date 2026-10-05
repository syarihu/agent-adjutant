use std::path::Path;

/// The resident that was asked to exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped {
    pub pid: u32,
    pub port: u16,
}

/// Ask the resident to exit. `None` when nothing was running — a record left by a killed one
/// is forgotten on the way. Only the server: a hub is a session of its own and goes on running.
pub fn stop(root: &Path) -> Result<Option<Stopped>, String> {
    Ok(crate::board::stop_resident(root, false)?.map(|(pid, port)| Stopped { pid, port }))
}
