//! A read whose reader has run for too long: what it is, and handing the read to a new request.
//!
//! Deciding that the old reader may go is not this module's: it needs to ask whether the process
//! is alive and to stop it, and `others` starts and stops no process. It reports who the reader
//! is, and takes over only when told the very reader that was checked is still the one.

use std::path::Path;

use crate::infra::clock::utc_stamp;
use crate::infra::terminal::SessionTerminal;
use crate::task::PrRef;

use super::model::{ReadMode, State};
use super::request_read::{HUNG_READER_SECS, RequestError, Requested, queue_afresh};
use super::store;

/// The reader of a read that has been `running` for more than `HUNG_READER_SECS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HungReader {
    pub session_id: String,
    pub pid: Option<u32>,
    pub ps_started: Option<String>,
    pub terminal: Option<SessionTerminal>,
}

/// The reader of `pr`'s read when it has been running too long: `None` when the read is not
/// running, has no launch time to be old from, or is younger than the limit. A running read with
/// no session is not a reader that can be named, and is also `None`.
pub fn hung_reader(root: &Path, pr: &PrRef, now: i64) -> Result<Option<HungReader>, String> {
    let Some(record) = store::load(root, pr)? else {
        return Ok(None);
    };
    // Stamps sort as text, so the cut-off is a stamp.
    let hung_before = utc_stamp(now - HUNG_READER_SECS);
    Ok(record
        .ai
        .filter(|ai| ai.status == "running")
        .filter(|ai| {
            ai.launched_at
                .as_deref()
                .is_some_and(|at| at < hung_before.as_str())
        })
        .and_then(|ai| {
            Some(HungReader {
                session_id: ai.session_id?,
                pid: ai.pid,
                ps_started: ai.ps_started,
                terminal: ai.terminal,
            })
        }))
}

/// Queue the read of `pr` afresh in place of the running one of `session_id`, the reader that was
/// checked. Asked again under the record's lock: when the read is no longer running under that
/// session (it handed in, failed, or was taken over already) nothing is changed and the answer is
/// `Already`. The new request's `mode` wins over the old reader's.
pub fn take_over_read(
    root: &Path,
    pr: &PrRef,
    now: i64,
    session_id: &str,
    mode: ReadMode,
) -> Result<Requested, RequestError> {
    let _lock = store::lock(root, &store::stem(pr)).map_err(RequestError::Failed)?;
    let mut record = match store::load(root, pr) {
        Ok(Some(record)) => record,
        Ok(None) => return Err(RequestError::NoRecord),
        Err(e) => return Err(RequestError::Failed(e)),
    };
    if record.state == State::Done {
        return Err(RequestError::Done);
    }
    let same = record
        .ai
        .as_ref()
        .is_some_and(|ai| ai.status == "running" && ai.session_id.as_deref() == Some(session_id));
    if !same {
        return Ok(Requested::Already(record));
    }
    queue_afresh(&mut record, now, mode);
    store::save(root, &mut record).map_err(RequestError::Failed)?;
    Ok(Requested::Queued(record))
}
