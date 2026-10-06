//! Reading the agent session ledger.

use super::store::{agent_session_path, agent_sessions_dir, read_agent_session};
use super::*;

/// The row of one session, with nothing judged about whether its process is still there.
/// An id that is not a file name is no row at all.
// Not called yet: the join from a hub or worker to its row is the first caller.
#[allow(dead_code)]
pub fn agent_session(root: &Path, session_id: &str) -> Recorded<AgentSession> {
    if !valid_session_id(session_id) {
        return Recorded::Absent;
    }
    read_agent_session(&agent_session_path(root, session_id))
}

/// Every row whose process is not known to be gone, the most recently seen first.
pub fn agent_sessions(root: &Path) -> Vec<AgentSession> {
    agent_sessions_with(root, &ProcessTable::snapshot())
}

/// `agent_sessions`, asking `table` when a process started (rule 7).
///
/// Leaves out the rows the sweep would remove, and deletes nothing: a sweep only runs when some
/// session sends `SessionStart` or `Stop`, and a closed tab must not show as `running` until
/// then. Rows that cannot be read are skipped.
pub fn agent_sessions_with(root: &Path, table: &ProcessTable) -> Vec<AgentSession> {
    let now = now_secs();
    let Ok(entries) = std::fs::read_dir(agent_sessions_dir(root)) else {
        return Vec::new();
    };
    let mut rows: Vec<AgentSession> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| match read_agent_session(&path) {
            Recorded::Found(row) => Some(row),
            Recorded::Absent | Recorded::Unreadable => None,
        })
        .filter(|row| !agent_session_dead(row, table, now))
        .collect();
    rows.sort_by(|a, b| {
        b.last_event_at
            .cmp(&a.last_event_at)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    rows
}
