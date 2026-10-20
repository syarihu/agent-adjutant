//! Writing down that the person chose to keep a session whose pull request is merged.

use super::store::{unreadable_worker_record, write_worker_record};
use super::*;

/// Write down, in the worker's own record, that the person chose to keep the session in
/// `worktree` instead of cleaning it up, and when.
///
/// Into the worker's own record rather than the task's, for the same reason as a phase: the
/// choice belongs to this run of the worker. A worker started again in the same worktree
/// writes its record fresh and starts without it, and removing the worktree takes it away.
pub fn keep_from_cleanup(worktree: &Path) -> Result<(), String> {
    let mut record = match read_worker_record(worktree) {
        Recorded::Found(record) => record,
        Recorded::Unreadable => return Err(unreadable_worker_record(worktree)),
        Recorded::Absent => {
            return Err(format!(
                "no worker is registered in {}: there is nothing to keep",
                worktree.display()
            ));
        }
    };
    record.cleanup_kept_at = Some(now_secs());
    write_worker_record(worktree, &record)
}
