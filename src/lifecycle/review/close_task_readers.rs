use std::path::Path;

use crate::infra::clock::now_secs;
use crate::lifecycle::review::{CloseError, close_reader_with, stop_reader};
use crate::others::{self, HungReader, TaskLink};
use crate::registry::{self, Liveness};
use crate::task::{self, Status, pr_ref};

/// Why a running read is failed when the task it is read for is finished.
const TASK_FINISHED: &str = "the task is finished";

/// Close the readers of the tasks that are finished (done or cancelled), or whose record is gone:
/// a reader of a task nobody works on any more only takes a place. A PR that is also requested
/// from you follows the review-request rule instead, and a reader whose task is still in play is
/// left alone, whatever its worker session does. The read-through, the drafts and the record stay.
///
/// Returns the ones that could not be closed, by id, and why. A reader that cannot be told alive
/// or dead, or cannot be stopped, is left as it is and tried again at the next pass; one failing
/// does not stop the others. No pass on the queue: the resident works it right after.
pub fn close_task_readers(root: &Path) -> Vec<(String, String)> {
    // Nothing to close, nothing to build: neither the tasks nor the config are read for a pass
    // with no reader open on a task's PR.
    if !has_task_reader(root) {
        return Vec::new();
    }
    let ctx = match registry::review_context(root) {
        Ok(ctx) => ctx,
        // Not "no failures": the readers were not looked at (no id: it is not one PR's).
        Err(why) => return vec![(String::new(), why)],
    };
    close_task_readers_with(
        root,
        now_secs(),
        &registry::hub_process_liveness,
        &|reader| stop_reader(&ctx, reader),
    )
}

/// Whether a PR read for a task, and not also requested from you, has a reader open.
pub(super) fn has_task_reader(root: &Path) -> bool {
    others::list(root).iter().any(|r| {
        r.task.is_some() && !r.requested && r.ai.as_ref().is_some_and(|ai| ai.reader_open())
    })
}

/// `close_task_readers`, with the time, the way a process is asked after and the way it is
/// stopped handed in.
pub(crate) fn close_task_readers_with(
    root: &Path,
    now: i64,
    alive: &dyn Fn(u32, Option<&str>) -> Liveness,
    stop: &dyn Fn(&HungReader) -> Result<(), String>,
) -> Vec<(String, String)> {
    let mut failures = Vec::new();
    for record in others::list(root) {
        let Some(link) = &record.task else {
            continue;
        };
        if record.requested || !record.ai.as_ref().is_some_and(|ai| ai.reader_open()) {
            continue;
        }
        if !task_finished(root, link) {
            continue;
        }
        let Some(pr) = pr_ref(&record.url, None) else {
            continue;
        };
        match close_reader_with(root, &pr, now, TASK_FINISHED, alive, stop) {
            // Nothing open to close, or the record changed under it: nothing to say.
            Ok(_) | Err(CloseError::NoReader | CloseError::NoRecord) => {}
            Err(CloseError::Failed(why)) => failures.push((record.id, why)),
        }
    }
    failures
}

/// Whether the task is done or cancelled, or definitely has no record. A record that is there and
/// cannot be read, or a place that cannot be looked at, is not finished: the reader stays.
fn task_finished(root: &Path, link: &TaskLink) -> bool {
    match task::get(root, &link.board, &link.id) {
        Ok(task) => matches!(task.status, Status::Done | Status::Cancelled),
        Err(_) => matches!(task::exists(root, &link.board, &link.id), Ok(false)),
    }
}
