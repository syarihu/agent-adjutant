//! Joining a worker to a task and a hub.

use super::store::{unreadable_worker_record, worker_session_path, write_worker_record};
use super::*;

/// Join the worker in `worktree` to a task and to the hub that task belongs to.
///
/// Written into the record the worker already has rather than a new one: the process fields
/// are what say it is still the same worker, and only the answers to "which task" and "which
/// hub" change. `hub` is `None` for the repository's own hub, which a record says by having
/// no key. A worker linked to a task is at work on it, so a record with no phase yet gets
/// `implement` — the card needs one to show, and nothing else has said otherwise. A `phase`
/// the caller names is entered whatever the record says: the person linking a session knows
/// where it stands, and an earlier phase must not decide for them.
///
/// The saved session is rewritten too, because `all_repo_hubs` counts a hub's children from
/// both, and one that kept naming the old hub would keep it from closing.
pub fn relink_worker(
    worktree: &Path,
    hub: Option<&str>,
    task: &str,
    phase: Option<&str>,
) -> Result<(), String> {
    let mut record = match read_worker_record(worktree) {
        Recorded::Found(record) => record,
        Recorded::Unreadable => return Err(unreadable_worker_record(worktree)),
        Recorded::Absent => {
            return Err(format!("no worker is registered in {}", worktree.display()));
        }
    };
    record.task = Some(task.to_string());
    record.hub = said(hub);
    if record.hub.is_none() {
        // A hub of another type is in `other`, and the repository's own hub is no key at all.
        record.other.remove("hub");
    }
    match phase {
        Some(phase) => append_phase(&mut record, phase, now_secs()),
        None if record.phase.is_none() => append_phase(&mut record, "implement", now_secs()),
        None => {}
    }
    // The saved session first and the record last: the record is what the board and the
    // worker read, so a failure in the second write must not leave it naming a task the
    // caller then takes back. The session is put back if the record cannot be written.
    let saved_path = worker_session_path(worktree);
    let previous = std::fs::read_to_string(&saved_path).ok();
    if let Some(saved) = worker_session(worktree) {
        rewrite_worker_session(
            worktree,
            &saved,
            saved.title.as_deref().unwrap_or_default(),
            hub,
            Some(task),
        )?;
    }
    if let Err(e) = write_worker_record(worktree, &record) {
        if let Some(previous) = previous {
            let _ = std::fs::write(&saved_path, previous);
        }
        return Err(e);
    }
    Ok(())
}
