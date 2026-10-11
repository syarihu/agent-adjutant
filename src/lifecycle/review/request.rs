use std::path::Path;

use crate::infra::clock::now_secs;
use crate::lifecycle::hub::hub_startable;
use crate::others::{self, RequestError, Requested};
use crate::registry;
use crate::task::PrRef;

use super::dispatch;

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
    let asked = request_with(root, pr, now_secs(), hub_up, startable)?;
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
