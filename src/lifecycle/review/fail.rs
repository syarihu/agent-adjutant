use std::path::Path;

use crate::infra::clock::now_secs;
use crate::lifecycle::review::dispatch;
use crate::others;
use crate::task::PrRef;

/// Why a reader's "I cannot finish" was not taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailError {
    /// The read is not the one this reader runs: it was asked for again, or it is not queued or
    /// running any more. Nothing was written.
    NotCurrent,
    Failed(String),
}

/// A reader saying it cannot finish: its read becomes `failed` with `why`, which frees its place
/// and lets the person ask again, and the next reader in line is started.
///
/// `session` is the reader's (`ADJUTANT_REVIEW_SESSION`) and must be the record's current one.
/// Only a person doing it by hand says none, and then whichever session holds the read fails.
///
/// `Ok` holds why the queue could not be worked afterwards, bare, when it could not; the read is
/// failed all the same.
pub fn fail(
    root: &Path,
    pr: &PrRef,
    session: Option<&str>,
    why: &str,
) -> Result<Option<String>, FailError> {
    let failed =
        others::fail_read(root, pr, session, why, now_secs()).map_err(FailError::Failed)?;
    if !failed {
        return Err(FailError::NotCurrent);
    }
    Ok(dispatch(root).error)
}
