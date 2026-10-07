//! Reading the agent session ledger.

use super::store::{agent_session_path, agent_sessions_dir, read_agent_session};
use super::*;

/// The row of one session, with nothing judged about whether its process is still there.
/// An id that is not a file name is no row at all.
pub fn agent_session(root: &Path, session_id: &str) -> Recorded<AgentSession> {
    if !valid_session_id(session_id) {
        return Recorded::Absent;
    }
    read_agent_session(&agent_session_path(root, session_id))
}

/// Every row whose process is not known to be gone, the most recently seen first. An error when
/// the directory cannot be listed: an empty answer must mean there are none.
pub fn agent_sessions(root: &Path) -> Result<Vec<AgentSession>, String> {
    agent_sessions_with(root, &ProcessTable::snapshot())
}

/// `agent_sessions`, asking `table` when a process started (rule 7).
///
/// Leaves out the rows the sweep would remove, and deletes nothing: a sweep only runs when some
/// session sends `SessionStart` or `Stop`, and a closed tab must not show as `running` until
/// then. Rows that cannot be read are skipped; a directory that is not there is no rows.
pub fn agent_sessions_with(root: &Path, table: &ProcessTable) -> Result<Vec<AgentSession>, String> {
    let now = now_secs();
    let mut rows = every_row(root)?;
    rows.retain(|row| !agent_session_dead(row, table, now));
    rows.sort_by(|a, b| {
        b.last_event_at
            .cmp(&a.last_event_at)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    Ok(rows)
}

/// The session id and `updatedAt` of every row that says `waiting`, whether or not its process
/// is still there: a read that has to be cheap and says only who is asking for a person. The
/// caller joins it to the sessions it lists, which is what tells a live one from a dead one.
/// An error when the directory cannot be listed.
pub fn waiting_agent_sessions(root: &Path) -> Result<Vec<(String, i64)>, String> {
    Ok(every_row(root)?
        .into_iter()
        .filter(|row| row.status == Some(AgentStatus::Waiting))
        .filter_map(|row| Some((row.session_id, row.updated_at?)))
        .collect())
}

/// Every row that can be read, in no order. Rows that cannot be read are skipped; a directory
/// that is not there is no rows.
fn every_row(root: &Path) -> Result<Vec<AgentSession>, String> {
    let dir = agent_sessions_dir(root);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot list {}: {e}", dir.display())),
    };
    let mut rows = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot list {}: {e}", dir.display()))?
            .path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        if let Recorded::Found(row) = read_agent_session(&path) {
            rows.push(row);
        }
    }
    Ok(rows)
}
