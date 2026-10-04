//! Clearing a hub's or a worker's record.

use super::store::{hub_record_path, worker_record_path};
use super::*;

pub fn unregister_hub(slug: &str) -> Result<(), String> {
    remove_if_present(&hub_record_path(slug))
}

/// Remove the hub record only while it still names the process `pid` started at `started`.
/// `Ok(true)` when the record is gone afterwards — removed, or already absent — and
/// `Ok(false)` when it names another process, which registered since and is not this call's
/// to clear.
///
/// Under the lock a takeover takes, so that it cannot read a record a claim is part-way
/// through replacing: unlike the launcher's own cleanup, whoever calls this is acting on a
/// hub it did not start, some time after it looked.
pub fn unregister_hub_if(slug: &str, pid: u32, started: Option<&str>) -> Result<bool, String> {
    let path = hub_record_path(slug);
    let lock_path = path.with_extension("claiming");
    let _lock = crate::infra::fs::lock(&lock_path)?;
    let named = match read_hub_record(slug) {
        Recorded::Absent => return Ok(true),
        Recorded::Unreadable => return Ok(false),
        Recorded::Found(record) => record,
    };
    let same = named.pid == Some(u64::from(pid)) && recorded_anchor(&named) == anchor_of(started);
    if !same {
        return Ok(false);
    }
    remove_if_present(&path)?;
    Ok(true)
}

/// Remove the hub record only while it still names no process. `Ok(true)` when it is gone
/// afterwards — removed, or already absent — and `Ok(false)` when it names a pid (a hub
/// claimed the name since the caller looked) or cannot be read.
///
/// Under the same lock as `unregister_hub_if`, for a caller that has stopped or checked a
/// hub and must not delete the record of one that registered in the meantime.
pub fn unregister_hub_if_unnamed(slug: &str) -> Result<bool, String> {
    let path = hub_record_path(slug);
    let lock_path = path.with_extension("claiming");
    let _lock = crate::infra::fs::lock(&lock_path)?;
    match read_hub_record(slug) {
        Recorded::Absent => Ok(true),
        Recorded::Unreadable => Ok(false),
        Recorded::Found(record) if record.names_a_pid() => Ok(false),
        Recorded::Found(_) => remove_if_present(&path).map(|()| true),
    }
}

pub fn unregister_worker(worktree: &Path) -> Result<(), String> {
    remove_if_present(&worker_record_path(worktree))
}

/// Clear a worktree's record, but only while it still names `worker`.
///
/// Anything but `Yes` means the record was left alone, and the caller has something to say
/// about a worktree it was about to call free. The three answers are kept apart because two
/// of them are different sentences to a person: "somebody else is working here" and "this
/// file cannot be read" are not the same news.
///
/// **`Yes` is not evidence that `worker` is dead.** This removes a note about a process; it
/// never looks at the process. Establishing the death is `worker_liveness`'s job and the
/// caller's responsibility, and doing it in the other order — clear the record, then read
/// the record to see whether the worker is gone — is the mistake this pair is shaped to
/// prevent. The order is not enforced by the types: these are `pub` inside a private
/// module, reachable only from this crate's own commands, and a token type bought here
/// would be a ceremony with no outside caller to protect.
///
/// Read-then-remove, so a registration landing in the gap between the two still loses its
/// record. The gap is left open on purpose: closing it means a lock on a path the hub's own
/// claim protocol writes, which is the one piece of concurrency here that has been argued
/// over and settled. What the gap costs is a record, which the next worker's launcher
/// rewrites; what a lock would cost is that settlement.
pub fn unregister_worker_if(worktree: &Path, worker: &WorkerIdentity) -> Result<Cleared, String> {
    match read_worker(worktree) {
        Recorded::Found(named) if &named == worker => {
            unregister_worker(worktree)?;
            Ok(Cleared::Yes)
        }
        // Already gone: whoever removed it wanted what this call wanted.
        Recorded::Absent => Ok(Cleared::Yes),
        Recorded::Found(_) => Ok(Cleared::AnotherWorker),
        Recorded::Unreadable => Ok(Cleared::Unreadable),
    }
}
