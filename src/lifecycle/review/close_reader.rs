use std::path::Path;

use crate::infra::clock::now_secs;
use crate::lifecycle::review::{dispatch, stop_reader};
use crate::others::{self, ClosedRead, HungReader, Record};
use crate::registry::{self, Liveness};
use crate::task::PrRef;

/// What closing a reader came to.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedReader {
    /// The record as it is now, with `ai.readerClosedAt` set.
    pub record: Record,
    /// The read was running, so it is `failed` now and can be asked for again.
    pub was_running: bool,
    /// Why the queue could not be worked after a running read gave up its place, bare; a
    /// transport words it. The close itself stands: the job tries again.
    pub queue_error: Option<String>,
}

/// Why a reader was not closed. The transports word it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseError {
    /// adj has no record of this PR.
    NoRecord,
    /// No reader is open on it: none was started, it is closed already, it was asked for again
    /// or is not running or handed in any more, or it has not recorded a process yet.
    NoReader,
    /// The reader could not be stopped, or could not be told alive or dead, or the record could
    /// not be read or written, and why. The record is as it was.
    Failed(String),
}

/// Why the read of a PR is failed when its reader is closed by hand.
const CLOSED_BY_HAND: &str = "the reader was closed";

/// Close the reader of `pr`: its pane is closed and the process confirmed gone (`stop_reader`),
/// and then the read is marked closed. The read-through, the drafts and the record stay. A read
/// that was running fails, which frees its place, and the next reader in line is started.
pub fn close_reader(root: &Path, pr: &PrRef) -> Result<ClosedReader, CloseError> {
    let ctx = registry::review_context(root).map_err(CloseError::Failed)?;
    let mut closed = close_reader_with(
        root,
        pr,
        now_secs(),
        CLOSED_BY_HAND,
        &registry::hub_process_liveness,
        &|reader| stop_reader(&ctx, reader),
    )?;
    if closed.was_running {
        // Outside the record's lock, which `close_read` has let go of.
        closed.queue_error = dispatch(root).error;
    }
    Ok(closed)
}

/// `close_reader`, with `why`, the time, the way a process is asked after and the way it is
/// stopped handed in, and without the dispatch. The reader is stopped first and the record
/// written after: a reader that cannot be stopped is left open and marked as nothing.
pub(crate) fn close_reader_with(
    root: &Path,
    pr: &PrRef,
    now: i64,
    why: &str,
    alive: &dyn Fn(u32, Option<&str>) -> Liveness,
    stop: &dyn Fn(&HungReader) -> Result<(), String>,
) -> Result<ClosedReader, CloseError> {
    if others::record(root, pr)
        .map_err(CloseError::Failed)?
        .is_none()
    {
        return Err(CloseError::NoRecord);
    }
    let reader = others::open_reader(root, pr)
        .map_err(CloseError::Failed)?
        .ok_or(CloseError::NoReader)?;
    // No process recorded yet: the reader is still being started, and there is nothing to stop.
    let pid = reader.pid.ok_or(CloseError::NoReader)?;
    match alive(pid, reader.ps_started.as_deref()) {
        Liveness::Gone => {}
        Liveness::Alive => {
            if let Err(why) = stop(&reader) {
                // Another closer (a sync, a second tab) may have closed the pane first: the process
                // is asked after once more, and a reader that is gone is what was wanted.
                if alive(pid, reader.ps_started.as_deref()) != Liveness::Gone {
                    return Err(CloseError::Failed(why));
                }
            }
        }
        Liveness::CannotTell => {
            return Err(CloseError::Failed(format!(
                "cannot tell whether the reader (pid {pid}) is still running"
            )));
        }
    }
    // The session is asked again under the record's lock: the read may have been asked for again
    // while the reader was being stopped.
    match others::close_read(root, pr, &reader.session_id, why, now).map_err(CloseError::Failed)? {
        ClosedRead::NotCurrent => Err(CloseError::NoReader),
        ClosedRead::Closed {
            record,
            was_running,
        } => Ok(ClosedReader {
            record: *record,
            was_running,
            queue_error: None,
        }),
    }
}
