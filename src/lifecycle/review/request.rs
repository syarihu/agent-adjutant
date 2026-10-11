use std::path::Path;

use crate::infra::clock::now_secs;
use crate::lifecycle::hub::hub_startable;
use crate::others::{self, HungReader, RequestError, Requested};
use crate::registry;
use crate::registry::Liveness;
use crate::task::PrRef;

use super::{dispatch, stop_reader};

/// Why asking for a read was refused. The transports word it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskError {
    /// The code-review hub is not running, so nothing would start a reader.
    HubStopped,
    /// The terminal is not one a reader can be opened in (`terminal.preset` is not tmux).
    NotStartable,
    /// adj has no record of this PR: the list is filled by a sync.
    NoRecord,
    /// The PR is done.
    Done,
    /// Something could not be read or written, and why.
    Failed(String),
}

/// What asking came to, and what the pass that followed could not do.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    pub requested: Requested,
    /// Why the queue could not be worked after the request was taken, bare; a transport words it.
    /// The request itself stands: the job tries again.
    pub queue_error: Option<String>,
}

/// Ask for a read-through of `pr`, and start the readers that are next in line, this one
/// included when a place is free. Refused while the code-review hub is not up: a request would
/// only wait, and the hub is also where a reader reports to.
pub fn request(root: &Path, pr: &PrRef) -> Result<Asked, AskError> {
    let ctx = registry::review_context(root).map_err(AskError::Failed)?;
    let hub_up = registry::hub_status(&ctx.state, &ctx.repo.slug, &ctx.repo.hub_name).present;
    let startable = hub_startable(&ctx.settings.terminal);
    let now = now_secs();
    let asked = request_taking_over(
        root,
        pr,
        now,
        hub_up,
        startable,
        &registry::hub_process_liveness,
        &|reader| stop_reader(&ctx, reader),
    )?;
    // Outside the record's lock, which `request_read` has let go of.
    let queue_error = dispatch(root).error;
    // The record as it is after the dispatch, which may have started this very read.
    let id = match &asked {
        Requested::Queued(record) | Requested::Already(record) => record.id.clone(),
    };
    let now = others::list(root).into_iter().find(|r| r.id == id);
    let requested = match (asked, now) {
        (Requested::Queued(_), Some(record)) => Requested::Queued(record),
        (Requested::Already(_), Some(record)) => Requested::Already(record),
        (asked, None) => asked,
    };
    Ok(Asked {
        requested,
        queue_error,
    })
}

/// `request`, with what it asked the machine (is the hub up, can a reader be opened) handed in,
/// and without the dispatch.
pub(crate) fn request_with(
    root: &Path,
    pr: &PrRef,
    now: i64,
    hub_up: bool,
    startable: bool,
) -> Result<Requested, AskError> {
    if !hub_up {
        return Err(AskError::HubStopped);
    }
    if !startable {
        return Err(AskError::NotStartable);
    }
    others::request_read(root, pr, now).map_err(|e| match e {
        RequestError::NoRecord => AskError::NoRecord,
        RequestError::Done => AskError::Done,
        RequestError::Failed(why) => AskError::Failed(why),
    })
}

/// `request_with`, and when the read is running with a reader that has run too long, the
/// takeover: the old reader goes first, and the read is taken over only once it is known to be
/// gone. A reader that is alive (and recorded when it started) is stopped; one that cannot be told
/// alive or dead, or cannot be stopped, is left running and the answer is `Already`: starting a
/// second beside it would put more than the allowed number of readers at work.
pub(crate) fn request_taking_over(
    root: &Path,
    pr: &PrRef,
    now: i64,
    hub_up: bool,
    startable: bool,
    alive: &dyn Fn(u32, Option<&str>) -> Liveness,
    stop: &dyn Fn(&HungReader) -> Result<(), String>,
) -> Result<Requested, AskError> {
    let asked = request_with(root, pr, now, hub_up, startable)?;
    let Requested::Already(_) = &asked else {
        return Ok(asked);
    };
    let hung = others::hung_reader(root, pr, now).map_err(AskError::Failed)?;
    let Some(reader) = hung.filter(|reader| may_take_over(reader, alive, stop)) else {
        return Ok(asked);
    };
    // The session is asked again under the record's lock: it may have ended since it was looked at.
    others::take_over_read(root, pr, now, &reader.session_id).map_err(|e| match e {
        RequestError::NoRecord => AskError::NoRecord,
        RequestError::Done => AskError::Done,
        RequestError::Failed(why) => AskError::Failed(why),
    })
}

/// Whether the old reader is out of the way: no process was ever recorded, it is gone, or it was
/// alive and has been stopped. Anything else leaves it where it is.
fn may_take_over(
    reader: &HungReader,
    alive: &dyn Fn(u32, Option<&str>) -> Liveness,
    stop: &dyn Fn(&HungReader) -> Result<(), String>,
) -> bool {
    let Some(pid) = reader.pid else {
        return true;
    };
    match alive(pid, reader.ps_started.as_deref()) {
        Liveness::Gone => true,
        Liveness::CannotTell => false,
        // Stopping needs the start time, which `stop` refuses without.
        Liveness::Alive => stop(reader).is_ok(),
    }
}
