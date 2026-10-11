//! What the board does for the code-review hub: ask for a read of a PR, and start the hub.
//!
//! Only the resident server serves these. Each answers a value, and the caller says what it
//! came to.

use std::path::Path;

use crate::lifecycle::hub::{HubStart, TabOutcome, start_hub};
use crate::lifecycle::review::{self, AskError, Asked};
use crate::others::pr_of;
use crate::registry;

/// Why a read was not asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestRefused {
    /// The text names no PR: it is neither a pull request URL nor `owner/repo#N`.
    BadId,
    Refused(AskError),
}

/// Ask for a read-through of the PR `id` names (a URL or `owner/repo#N`), and start the readers
/// that are next in line.
pub fn request(root: &Path, id: &str) -> Result<Asked, RequestRefused> {
    let pr = pr_of(id).ok_or(RequestRefused::BadId)?;
    review::request(root, &pr).map_err(RequestRefused::Refused)
}

/// Start the code-review hub in a new window, as `adj hub --review --tab` would.
pub fn start(root: &Path, start: HubStart) -> Result<TabOutcome, String> {
    start_hub(&registry::review_context(root)?, start)
}
