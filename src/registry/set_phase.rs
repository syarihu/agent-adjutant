//! Writing down which step a worker is in.

use super::store::{unreadable_worker_record, write_worker_record};
use super::*;

/// Write down which step the worker in `worktree` is in, and when it got there.
///
/// Into the worker's own record rather than the task's, because the phase belongs to this
/// run of the worker: a worker started again in the same worktree starts without one, as its
/// record is written fresh. The time is what the board measures "stuck" from.
pub fn set_worker_phase(worktree: &Path, phase: &str) -> Result<(), String> {
    if !PHASES.contains(&phase) {
        return Err(format!(
            "no such phase: {phase} (one of {})",
            PHASES.join(", ")
        ));
    }
    let mut record = match read_worker_record(worktree) {
        Recorded::Found(record) => record,
        Recorded::Unreadable => return Err(unreadable_worker_record(worktree)),
        Recorded::Absent => {
            return Err(format!(
                "no worker is registered in {}: `adj phase` is for the worker running there",
                worktree.display()
            ));
        }
    };
    append_phase(&mut record, phase, now_secs());
    write_worker_record(worktree, &record)
}
