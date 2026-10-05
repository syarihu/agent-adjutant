use std::path::Path;

use crate::kernel::identity::RepoInfo;
use crate::registry::{live_resident, note_board, recorded_version};

/// How a restart ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restarted {
    /// Nothing was running. `port` is the one to start on; nothing has been started yet.
    WasNotRunning { port: u16 },
    /// The resident was stopped, but a supervisor had already started it again as `current`.
    Supervised { stopped: u32, current: u32 },
    /// The resident `stopped` was replaced by `pid`, which asked for `wanted` and bound
    /// `bound`. `versions` is the old and new one when they differ.
    Replaced {
        stopped: u32,
        pid: u32,
        wanted: u16,
        bound: u16,
        versions: Option<(String, String)>,
    },
}

/// Why a restart failed: before anything was stopped, or after, when nothing is running now —
/// which the caller says before the error, as it says every stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartFailed {
    Stop(String),
    Start { stopped: u32, error: String },
}

impl std::fmt::Display for RestartFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestartFailed::Stop(error) => f.write_str(error),
            RestartFailed::Start { stopped, error } => write!(
                f,
                "stopped pid {stopped}, but the new server did not start; nothing is running now: {error}"
            ),
        }
    }
}

/// Stop the resident and start it again, on the port it had unless `port` says otherwise. The
/// new one is this binary, which is what lets a reinstall take effect. Hubs and workers are
/// other processes and go on running. Prints nothing; when nothing was running it starts
/// nothing either, and says so.
pub fn restart(
    root: &Path,
    port: Option<u16>,
    here: Option<&RepoInfo>,
) -> Result<Restarted, RestartFailed> {
    let old_version = recorded_version(root);
    let Some((stopped, old_port)) =
        crate::board::stop_resident(root, true).map_err(RestartFailed::Stop)?
    else {
        return Ok(Restarted::WasNotRunning {
            port: port.unwrap_or(crate::board::DEFAULT_PORT),
        });
    };
    // Catches a supervisor that was quicker than this check and nothing more: one that
    // respawns after it still races the start below, so restart is not for a supervised server.
    if let Some((current, _)) = live_resident(root) {
        return Ok(Restarted::Supervised { stopped, current });
    }
    let wanted = port.unwrap_or(old_port);
    let bound = crate::board::launch_resident(root, wanted)
        .map_err(|error| RestartFailed::Start { stopped, error })?;
    let pid = live_resident(root).map_or(0, |(pid, _)| pid);
    if let Some(repo) = here {
        note_board(root, repo);
    }
    let versions = match (old_version, recorded_version(root)) {
        (Some(old), Some(new)) if old != new => Some((old, new)),
        _ => None,
    };
    Ok(Restarted::Replaced {
        stopped,
        pid,
        wanted,
        bound,
        versions,
    })
}
