use std::path::Path;

use crate::infra::clock::now_secs;
use crate::lifecycle::review::dispatch;
use crate::others::{self, SubmitError, Submitted};
use crate::task::PrRef;

/// What a hand-in came to, and what the pass that followed could not do.
#[derive(Debug, Clone, PartialEq)]
pub struct HandedIn {
    pub submitted: Submitted,
    /// Why the next reader in line could not be started, bare. The hand-in stands.
    pub queue_error: Option<String>,
}

/// Hand in a read-through (see `others::submit_read`), and then start the next reader in line:
/// this one has let go of its place. The dispatch is after the record's lock is let go of.
pub fn submit(
    root: &Path,
    pr: &PrRef,
    text: &str,
    session: Option<&str>,
) -> Result<HandedIn, SubmitError> {
    let submitted = others::submit_read(root, pr, text, session, now_secs())?;
    let queue_error = dispatch(root).error;
    Ok(HandedIn {
        submitted,
        queue_error,
    })
}
