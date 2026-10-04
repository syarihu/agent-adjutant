use crate::infra::terminal;
use crate::kernel::config::Settings;
use crate::lifecycle::{GONE_BUDGET, GONE_POLL, settled};
use crate::registry;
use std::path::Path;

/// What closing the worker in a worktree came to, for the caller to say or act on.
///
/// One variant per place the decision ended, so a command can word it for a person and the
/// board can reduce it to a yes or no without either re-deciding anything.
pub enum WorkerClose {
    /// Nothing is registered in the worktree.
    NoWorker,
    /// A record is there and cannot be read as naming a worker; nothing was cleared.
    Unreadable,
    /// The worker is gone and its record was (or, on a dry run, would be) cleared.
    Gone { dry_run: bool },
    /// The worker is gone, but the record was not this call's to clear.
    GoneLeftAlone(registry::Cleared),
    /// Whether the worker's process is still running could not be established.
    CannotTell { pid: u32 },
    /// A dry run: what would be run to close the tab.
    WouldClose(terminal::Performed),
    /// The close command was not run (`terminal.close` is off, say).
    NotRun(terminal::Performed),
    /// The tab was closed, the worker is gone and its record was cleared.
    Closed(terminal::Performed),
    /// The tab was closed and the worker is gone, but the record was not this call's to clear.
    ClosedLeftAlone {
        pid: u32,
        cleared: registry::Cleared,
    },
    /// The close command ran and the worker is still there.
    StillRunning { pid: u32 },
    /// The close command ran and whether the worker is gone cannot be established.
    GoneUnknown { pid: u32 },
}

impl WorkerClose {
    /// Whether the worktree is free of a worker.
    ///
    /// `false` means a worker may still be sitting there. The caller is on its way to removing
    /// this worktree, so that answer has to reach a shell as an exit code rather than as a
    /// sentence in the output — and everything this cannot establish answers `false`, because
    /// the cost of the two mistakes is not symmetric: a cleanup that stops is finished by hand,
    /// a cleanup that carries on deletes work nobody can get back.
    ///
    /// A dry run is `false` as well: the exit code still answers about the worktree rather
    /// than about the plan — a live worker was found, and nothing has been closed. `focus`
    /// sets the precedent: its dry run reports the state it looked at. Answering "safe to
    /// remove" here is how `close --dry-run && git worktree remove` deletes a live worker's
    /// checkout.
    pub fn is_free(&self) -> bool {
        matches!(
            self,
            WorkerClose::NoWorker | WorkerClose::Gone { .. } | WorkerClose::Closed(_)
        )
    }
}

/// Close the tab the worker in a worktree is sitting in, and report how it went.
///
/// Everything here is about **one** worker, read out of the record once and carried
/// through: the gate, the tab that gets closed, the process that has to be gone afterwards
/// and the record that may then be cleared. Asked separately, each of those questions can
/// be answered about a different worker — the next one registering in the same worktree —
/// and the answers then compose into a worktree that is deleted while somebody is using it.
pub fn close(settings: &Settings, worktree: &Path, dry_run: bool) -> Result<WorkerClose, String> {
    // No `is_dir` check, deliberately unlike `tell`: this runs during cleanup, so a
    // worktree that has already been removed is the ordinary way to arrive here twice
    // rather than a mistake worth failing over.
    let worker = match registry::read_worker(worktree) {
        // Nothing registered here is the job already done. A hub that calls this twice, or
        // calls it on a worker that stopped on its own, has to get on with the cleanup.
        registry::Recorded::Absent => return Ok(WorkerClose::NoWorker),
        registry::Recorded::Unreadable => return Ok(WorkerClose::Unreadable),
        registry::Recorded::Found(worker) => worker,
    };
    let pid = worker.pid;
    match registry::worker_liveness(&worker) {
        registry::Liveness::Gone => {
            // The worker this record named is gone, so the record is the only thing left to
            // clear — and only while it is still that worker's.
            let cleared = match dry_run {
                true => registry::Cleared::Yes,
                false => registry::unregister_worker_if(worktree, &worker)?,
            };
            return Ok(match cleared {
                registry::Cleared::Yes => WorkerClose::Gone { dry_run },
                other => WorkerClose::GoneLeftAlone(other),
            });
        }
        registry::Liveness::CannotTell => return Ok(WorkerClose::CannotTell { pid }),
        registry::Liveness::Alive => {}
    }
    let done = terminal::close(
        &settings.terminal,
        pid,
        // The name the tab actually carries: `spawn` put the record's title through
        // `sanitise_title` with the directory name behind it, and a template that matches
        // a tab by name has to be handed the answer that got there.
        &terminal::sanitise_title(
            worker.title.as_deref().unwrap_or_default(),
            worktree
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default(),
        ),
        dry_run,
    )?;
    if dry_run {
        return Ok(WorkerClose::WouldClose(done));
    }
    if !done.ran {
        return Ok(WorkerClose::NotRun(done));
    }
    // What the close command reported is not the question — `terminal::close` says what
    // little `ran` can mean. Neither is what the record says afterwards: a close command
    // that removed the record instead of the tab would leave a worktree that *looks* free.
    // Only the worker's own absence settles it.
    match settled(
        || registry::worker_liveness(&worker),
        std::thread::sleep,
        GONE_BUDGET,
        GONE_POLL,
    ) {
        registry::Liveness::Gone => {
            // The process went with its tab, so a record left behind would have `present`
            // lying to whoever asks next — including the next call to this. Conditional,
            // because the worktree may have been handed to a new worker while this one was
            // being closed, and that worker's record is not this call's to remove.
            let cleared = registry::unregister_worker_if(worktree, &worker)?;
            Ok(match cleared {
                registry::Cleared::Yes => WorkerClose::Closed(done),
                cleared => WorkerClose::ClosedLeftAlone { pid, cleared },
            })
        }
        registry::Liveness::Alive => Ok(WorkerClose::StillRunning { pid }),
        registry::Liveness::CannotTell => Ok(WorkerClose::GoneUnknown { pid }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worktree_with_no_record_has_no_worker_and_is_free() {
        let _sandbox = crate::testing::Sandbox::empty();
        let worktree = tempfile::tempdir().unwrap();
        let closed = close(&Settings::default(), worktree.path(), false).unwrap();
        assert!(matches!(closed, WorkerClose::NoWorker));
        assert!(closed.is_free());
    }

    /// A record that names nobody is a file somebody wrote in a worktree somebody may be
    /// working in; reading it as free is the fail-open this exists to avoid.
    #[test]
    fn a_record_that_names_no_worker_is_unreadable_and_not_free() {
        let _sandbox = crate::testing::Sandbox::empty();
        let worktree = tempfile::tempdir().unwrap();
        let record = registry::worker_record_path(worktree.path());
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, "{}").unwrap();
        let closed = close(&Settings::default(), worktree.path(), false).unwrap();
        assert!(matches!(closed, WorkerClose::Unreadable));
        assert!(!closed.is_free());
        assert!(record.exists());
    }
}
