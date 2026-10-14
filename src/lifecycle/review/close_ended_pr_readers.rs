use std::path::Path;

use crate::infra::clock::now_secs;
use crate::lifecycle::review::close_readers::NO_LONGER_OPEN;
use crate::lifecycle::review::{CloseError, close_reader_with, stop_reader};
use crate::others::{self, HungReader};
use crate::registry::{self, Liveness};
use crate::task::PrRef;

/// Close the readers of `prs`, which the PR poll has just read as merged or closed: the same
/// close as `close_finished_readers`, for the PRs a read names and without reading every record's
/// state. Nothing is built, and the config is not read, when none of them has a reader open.
///
/// It runs inside the poll round, so a reader that is slow to stop delays that round; that is
/// accepted, since it happens once for each reader.
///
/// Returns the ones that could not be closed, by id, and why; they are tried again at the next
/// sync, which looks at every record.
pub fn close_ended_pr_readers(root: &Path, prs: &[PrRef]) -> Vec<(String, String)> {
    if !prs.iter().any(|pr| has_open_reader(root, pr)) {
        return Vec::new();
    }
    let ctx = match registry::review_context(root) {
        Ok(ctx) => ctx,
        Err(why) => return vec![(String::new(), why)],
    };
    // A running read that is closed gives its place up; the resident's queue pass starts the
    // next one, so this does not dispatch.
    close_ended_pr_readers_with(
        root,
        prs,
        now_secs(),
        &registry::hub_process_liveness,
        &|reader| stop_reader(&ctx, reader),
    )
}

/// Whether `pr` has a record with a reader open. A record that cannot be read has none to close.
fn has_open_reader(root: &Path, pr: &PrRef) -> bool {
    matches!(others::record(root, pr), Ok(Some(r)) if r.ai.as_ref().is_some_and(|ai| ai.reader_open()))
}

/// `close_ended_pr_readers`, with the time, the way a process is asked after and the way it is
/// stopped handed in.
pub(crate) fn close_ended_pr_readers_with(
    root: &Path,
    prs: &[PrRef],
    now: i64,
    alive: &dyn Fn(u32, Option<&str>) -> Liveness,
    stop: &dyn Fn(&HungReader) -> Result<(), String>,
) -> Vec<(String, String)> {
    let mut failures = Vec::new();
    for pr in prs {
        match close_reader_with(root, pr, now, NO_LONGER_OPEN, alive, stop) {
            Ok(_) | Err(CloseError::NoReader | CloseError::NoRecord) => {}
            Err(CloseError::Failed(why)) => {
                failures.push((format!("{}#{}", pr.nwo(), pr.number), why))
            }
        }
    }
    failures
}
