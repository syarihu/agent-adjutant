//! Worker slots: who holds one, and the marker of a worker still starting.

use super::store::starting_marker_path;
use super::*;

pub fn mark_worker_starting(worktree: &Path) -> Result<(), String> {
    write_json(
        &starting_marker_path(worktree),
        &json!({ "at": now_secs() }),
    )
}

pub fn unmark_worker_starting(worktree: &Path) -> Result<(), String> {
    remove_if_present(&starting_marker_path(worktree))
}

/// Whether `worktree` holds a worker slot: a worker that is there, or one dispatched less
/// than `STARTING_GRACE_SECS` ago that has not said so yet. A worker parked at a gate holds
/// its slot like any other — it is still a process on this machine — and a dead one does not.
///
/// "Dead" means `worker_liveness` said `Gone`, not merely that presence could not be shown:
/// when `ps` cannot answer, or the record has no start time to match against, the worker
/// may well be running, and counting it free is how a limit is overshot. Counting it busy
/// only delays a dispatch, and not for long — a pid that is no longer running is `Gone`
/// whatever else the record lacks.
pub fn holds_worker_slot(worktree: &Path, now: i64) -> bool {
    holds_worker_slot_with(&ProcessTable::each(), worktree, now)
}

/// `holds_worker_slot`, asking `table` when the worker's process started.
pub fn holds_worker_slot_with(table: &ProcessTable, worktree: &Path, now: i64) -> bool {
    let registered = match read_worker(worktree) {
        Recorded::Found(worker) => worker_liveness_with(table, &worker) != Liveness::Gone,
        // Neither names a process that could be running.
        Recorded::Absent | Recorded::Unreadable => false,
    };
    registered || is_starting(worktree, now)
}

/// The half of `holds_worker_slot` that is not a `ps` call, for a caller that already has
/// the worker's status in hand.
pub fn is_starting(worktree: &Path, now: i64) -> bool {
    read_json(&starting_marker_path(worktree))
        .and_then(|marker| marker.get("at").and_then(Value::as_i64))
        // Bounded below as well: a clock set back after the dispatch would otherwise hold
        // the slot for as long as it was moved.
        .is_some_and(|at| (0..STARTING_GRACE_SECS).contains(&(now - at)))
}

/// The worktrees among `worktrees` holding a worker slot, `except` left out — the one about
/// to be dispatched into, which is taking a slot rather than competing for one.
pub fn busy_worktrees(worktrees: &[String], except: Option<&Path>) -> Vec<String> {
    let now = now_secs();
    // Compared resolved, because git answers with the real path and a caller may be holding
    // one through a symlink — `/tmp` against `/private/tmp` on a Mac — and a worktree that
    // failed to match itself would count against its own dispatch.
    let resolved = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let except = except.map(resolved);
    worktrees
        .iter()
        .filter(|path| except.as_deref() != Some(resolved(Path::new(path.as_str())).as_path()))
        .filter(|path| holds_worker_slot(Path::new(path.as_str()), now))
        .cloned()
        .collect()
}
