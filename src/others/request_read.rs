//! Asking for a read-through of a PR: it joins the queue.

use std::path::Path;

use crate::infra::clock::utc_stamp;
use crate::task::PrRef;

use super::model::{AiRead, Record, State};
use super::store;

/// How long a reader may stay `running`, in seconds, before asking again may take its place from
/// it (after the old reader has been looked at: see `take_over_read`). A reader that cannot
/// finish and cannot say so would otherwise hold a place for ever.
pub const HUNG_READER_SECS: i64 = 30 * 60;

/// What asking came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Requested {
    /// It is in the queue now.
    Queued(Record),
    /// It was queued or being read already; nothing was written.
    Already(Record),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// adj has no record of this PR: the list is filled by a sync.
    NoRecord,
    /// The PR is done: nobody reads what is merged, closed or withdrawn.
    Done,
    /// The record could not be read or written, and why.
    Failed(String),
}

/// Put the read of `record` in the queue as a new one: asked now, with no reader. What an earlier
/// read-through made (`sha`, `counts`, the file) is kept.
pub(super) fn queue_afresh(record: &mut Record, now: i64) {
    let stamp = utc_stamp(now);
    let ai = record.ai.get_or_insert_with(|| AiRead {
        status: String::new(),
        sha: None,
        started_at: None,
        finished_at: None,
        launched_at: None,
        read_from: None,
        session_id: None,
        pid: None,
        ps_started: None,
        terminal: None,
        error: None,
        counts: None,
        stale: false,
        extra: Default::default(),
    });
    ai.status = "queued".to_string();
    ai.started_at = Some(stamp);
    ai.launched_at = None;
    ai.pid = None;
    ai.ps_started = None;
    ai.session_id = None;
    ai.terminal = None;
    ai.error = None;
}

/// Queue a read of `pr`. The old read-through, if there is one, stays where it is, and is shown
/// as made at the old head until the new one replaces it: its `sha`, `counts` and file are kept.
/// `startedAt` is when the person asked, which is also the place in the queue.
pub fn request_read(root: &Path, pr: &PrRef, now: i64) -> Result<Requested, RequestError> {
    // No lock file for a record that is not there.
    match store::load(root, pr) {
        Ok(Some(_)) => {}
        Ok(None) => return Err(RequestError::NoRecord),
        Err(e) => return Err(RequestError::Failed(e)),
    }
    let _lock = store::lock(root, &store::stem(pr)).map_err(RequestError::Failed)?;
    let mut record = match store::load(root, pr) {
        Ok(Some(record)) => record,
        Ok(None) => return Err(RequestError::NoRecord),
        Err(e) => return Err(RequestError::Failed(e)),
    };
    if record.state == State::Done {
        return Err(RequestError::Done);
    }
    // A read that is queued or running is not asked for again, however long it has been running:
    // whether a reader that has run too long may be replaced is for `take_over_read`, which is
    // asked only after the old reader has been looked at.
    if record
        .ai
        .as_ref()
        .is_some_and(|ai| matches!(ai.status.as_str(), "queued" | "running"))
    {
        return Ok(Requested::Already(record));
    }
    queue_afresh(&mut record, now);
    store::save(root, &mut record).map_err(RequestError::Failed)?;
    Ok(Requested::Queued(record))
}
