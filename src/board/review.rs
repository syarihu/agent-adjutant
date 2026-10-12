//! What the board does for the code-review hub: ask for a read of a PR, and start the hub.
//!
//! Only the resident server serves these. Each answers a value, and the caller says what it
//! came to.

use std::path::Path;

use crate::lifecycle::hub::{HubStart, TabOutcome, start_hub, stop_hub};
use crate::lifecycle::review::{self, AskError, Asked, CloseError, ClosedReader, TaskAskError};
use crate::others::{self, ReadMode, Record, SyncError, Synced, pr_of};
use crate::registry::{self, Context};

mod hand_to_worker;
mod reader_window;

pub use hand_to_worker::*;
pub use reader_window::*;

/// Why a read was not asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestRefused {
    /// The text names no PR: it is neither a pull request URL nor `owner/repo#N`.
    BadId,
    Refused(AskError),
}

/// Ask for a read-through of the PR `id` names (a URL or `owner/repo#N`), and start the readers
/// that are next in line, for a read-through or, in `mode` split, a split proposal.
pub fn request(root: &Path, id: &str, mode: ReadMode) -> Result<Asked, RequestRefused> {
    let pr = pr_of(id).ok_or(RequestRefused::BadId)?;
    review::request(root, &pr, mode).map_err(RequestRefused::Refused)
}

/// Ask for a read-through of the PR of the task `task_id` of the hub `ctx` is for: the same record
/// and the same queue as `request`, linked to the task.
pub fn ask_for_task(ctx: &Context, task_id: &str, mode: ReadMode) -> Result<Asked, TaskAskError> {
    review::request_for_task(ctx, task_id, mode)
}

/// Why a reader was not closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseRefused {
    /// The text names no PR: it is neither a pull request URL nor `owner/repo#N`.
    BadId,
    Refused(CloseError),
}

/// Close the reader of the PR `id` names: its window goes, and its read-through, drafts and
/// record stay.
pub fn close(root: &Path, id: &str) -> Result<ClosedReader, CloseRefused> {
    let pr = pr_of(id).ok_or(CloseRefused::BadId)?;
    review::close_reader(root, &pr).map_err(CloseRefused::Refused)
}

/// Sync the PRs asked of you, and then close the readers of the ones the sync found merged or
/// closed. The readers are closed once the sync's lock is let go: stopping one waits for its
/// process, and a second sync must not be held up behind that.
///
/// The records in the answer are read again after the closes, so they carry `readerClosedAt`.
/// A task's PR that nobody requested of you is not in them: it is not on the list.
/// The failures are the PRs whose reader could not be closed, by id, and why; the sync itself
/// stands.
pub fn sync(root: &Path) -> Result<(Synced, Vec<(String, String)>), SyncError> {
    let mut synced = others::sync(root)?;
    let failures = review::close_finished_readers(root);
    synced.records = others::list(root)
        .into_iter()
        .filter(Record::on_review_list)
        .collect();
    Ok((synced, failures))
}

/// Start the code-review hub in a new window, as `adj hub --review --tab` would.
pub fn start(root: &Path, start: HubStart) -> Result<TabOutcome, String> {
    start_hub(&registry::review_context(root)?, start)
}

/// Stop the code-review hub by closing its tmux pane, as a repository's hub is stopped from the
/// board; `true` when one was running.
pub fn stop(root: &Path) -> Result<bool, String> {
    stop_hub(&registry::review_context(root)?)
}
