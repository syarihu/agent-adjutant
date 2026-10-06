//! The one join from a hub or worker to its row in the agent session ledger.

use super::liveness::agent_session_dead;
use super::*;

/// What a hub or worker knows of its agent, to find the row by: the session id it was started
/// with, and the pid and start time its record holds.
#[derive(Debug, Clone, Copy, Default)]
pub struct AgentIdentity<'a> {
    pub session_id: Option<&'a str>,
    pub pid: Option<u32>,
    pub ps_started: Option<&'a str>,
}

/// The row of the agent a hub or worker started, by the session id first and then by the
/// process.
///
/// The id is only what the agent was started with: `/clear` makes a new session in the same
/// process and the hooks then report the new id, and a runner without `{sessionId}` has no id
/// to begin with. The process is what stays the same, so a row whose id is not the one given
/// is still the agent's when its pid and start time are the record's. A pid alone is not
/// enough, because one is reused once its process is gone (the start time is how liveness
/// tells them apart, too); with no start time there is no second step. When several rows are
/// one process's, the most recently seen is the one it is on now.
///
/// Rows whose process is gone are never returned. An error only when the ledger cannot be
/// listed: an answer of `None` means there is no such row.
// Its first caller is the board (a later issue); until then only the tests ask.
#[cfg_attr(not(test), allow(dead_code))]
pub fn agent_session_of(
    root: &Path,
    table: &ProcessTable,
    identity: &AgentIdentity,
) -> Result<Option<AgentSession>, String> {
    if let Some(id) = identity.session_id
        && let Recorded::Found(row) = agent_session(root, id)
        && !agent_session_dead(&row, table, now_secs())
    {
        return Ok(Some(row));
    }
    let (Some(pid), Some(started)) = (identity.pid, anchor_of(identity.ps_started)) else {
        return Ok(None);
    };
    // Newest first, so the first match is the one the process is on now.
    Ok(agent_sessions_with(root, table)?
        .into_iter()
        .find(|row| row.pid == Some(pid) && anchor_of(row.ps_started.as_deref()) == Some(started)))
}

#[cfg(test)]
mod tests;
