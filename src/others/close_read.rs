//! Marking a read as closed once its reader was stopped.
//!
//! Stopping the process is not this module's: `others` starts and stops no process. It marks the
//! read closed only when told the very session that was stopped is still the current one.

use std::path::Path;

use crate::task::PrRef;

use super::fail_read::mark_failed;
use super::model::Record;
use super::store;

/// What closing the reader came to.
#[derive(Debug, Clone, PartialEq)]
pub enum ClosedRead {
    /// The read is not the one that was stopped: it was asked for again, or it is not running or
    /// handed in any more. Nothing was written.
    NotCurrent,
    Closed {
        /// Boxed: a record is large, and the other variant holds nothing.
        record: Box<Record>,
        /// The read was `running`, so it is `failed` now and can be asked for again.
        was_running: bool,
    },
}

/// Mark the reader of `session_id` as closed, under the record's lock. A read that is `running`
/// fails with `why`, as `fail_read` does, which frees its place and lets the person ask again;
/// one that is `ready` stays so. The read-through, the drafts and the record all stay.
/// When the read is no longer running or handed in under that session nothing is changed.
pub fn close_read(
    root: &Path,
    pr: &PrRef,
    session_id: &str,
    why: &str,
    now: i64,
) -> Result<ClosedRead, String> {
    let _lock = store::lock(root, &store::stem(pr))?;
    let mut record = store::load(root, pr)?.ok_or("no record of this pull request")?;
    let Some(ai) = record.ai.as_mut() else {
        return Ok(ClosedRead::NotCurrent);
    };
    let current = matches!(ai.status.as_str(), "running" | "ready")
        && ai.session_id.as_deref() == Some(session_id);
    if !current {
        return Ok(ClosedRead::NotCurrent);
    }
    let was_running = ai.status == "running";
    if was_running {
        mark_failed(ai, why, now);
    }
    ai.reader_closed_at = Some(crate::infra::clock::utc_stamp(now));
    store::save(root, &mut record)?;
    Ok(ClosedRead::Closed {
        record: Box::new(record),
        was_running,
    })
}
