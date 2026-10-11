use std::path::Path;

use crate::infra::clock::now_secs;
use crate::lifecycle::review::{CloseError, close_reader_with, dispatch, stop_reader};
use crate::others::{self, HungReader, PrOpenState};
use crate::registry::{self, Liveness};
use crate::task::pr_ref;

/// Why a read is failed when the PR left review for good; the words `claim_reads` uses.
const NO_LONGER_OPEN: &str = "the PR is no longer open";

/// Close the readers of the PRs that are merged or closed, after a sync found them so: a reader
/// of a PR nobody reads any more only takes a place. A PR that left review for another reason
/// (approved, the request withdrawn) keeps its reader, which is why this looks at the PR's own
/// state and not at the record being done.
///
/// Returns the ones that could not be closed, by id, and why. A reader that cannot be told alive
/// or dead, or cannot be stopped, is left as it is and tried again at the next sync; one failing
/// does not stop the others.
pub fn close_finished_readers(root: &Path) -> Vec<(String, String)> {
    // Nothing to close, nothing to build: a config that cannot be read is no failure of a sync that had no
    // finished PR with a reader.
    if !has_finished_reader(root) {
        return Vec::new();
    }
    let ctx = match registry::review_context(root) {
        Ok(ctx) => ctx,
        // Not "no failures": the readers were not looked at (no id: it is not one PR's).
        Err(why) => return vec![(String::new(), why)],
    };
    // A running read that is closed gives its place up, and the next in line may start. Not a
    // pass on the queue otherwise: a sync does not start reads.
    let freeing = frees_a_place(root);
    let failures = close_finished_readers_with(
        root,
        now_secs(),
        &registry::hub_process_liveness,
        &|reader| stop_reader(&ctx, reader),
    );
    if freeing {
        // Its own error is the job's to retry; the closes stand.
        let _ = dispatch(root);
    }
    failures
}

/// Whether a PR that is merged or closed has a reader open.
pub(super) fn has_finished_reader(root: &Path) -> bool {
    others::list(root).iter().any(|r| {
        r.pr_state != PrOpenState::Open && r.ai.as_ref().is_some_and(|ai| ai.reader_open())
    })
}

/// Whether closing the readers of the finished PRs gives up a place: one of them is a running
/// read, which fails, where a read that is handed in already holds none.
pub(super) fn frees_a_place(root: &Path) -> bool {
    others::list(root).iter().any(|r| {
        r.pr_state != PrOpenState::Open
            && r.ai
                .as_ref()
                .is_some_and(|ai| ai.status == "running" && ai.reader_open())
    })
}

/// `close_finished_readers`, with the time, the way a process is asked after and the way it is
/// stopped handed in, and without the dispatch.
pub(crate) fn close_finished_readers_with(
    root: &Path,
    now: i64,
    alive: &dyn Fn(u32, Option<&str>) -> Liveness,
    stop: &dyn Fn(&HungReader) -> Result<(), String>,
) -> Vec<(String, String)> {
    let mut failures = Vec::new();
    for record in others::list(root) {
        if record.pr_state == PrOpenState::Open {
            continue;
        }
        let Some(pr) = pr_ref(&record.url, None) else {
            continue;
        };
        match close_reader_with(root, &pr, now, NO_LONGER_OPEN, alive, stop) {
            // Nothing open to close, or the record changed under it: nothing to say.
            Ok(_) | Err(CloseError::NoReader | CloseError::NoRecord) => {}
            Err(CloseError::Failed(why)) => failures.push((record.id, why)),
        }
    }
    failures
}
