//! Where a PR's reader works, and the names of the PR that put it there. One place, so the record,
//! the reader's window, the material and the clean-up all mean the same directory.

use std::path::{Path, PathBuf};

use crate::kernel::identity::review_hub_at;
use crate::task::PrRef;

use super::store;

/// The code-review hub's home, `<state>/review`: the directory it starts in and the readers' directories
/// are under. The name is the hub's own (`review_hub_at`), not spelled again here.
pub fn review_home(root: &Path) -> PathBuf {
    PathBuf::from(review_hub_at(root).main)
}

/// The file stem of a PR, as its record and its reader's directory are named.
pub fn reader_stem(pr: &PrRef) -> String {
    store::stem(pr)
}

/// `owner/repo#N`, the key the record carries.
pub fn pr_id(pr: &PrRef) -> String {
    store::id_of(pr)
}

/// The directory a PR's reader works in: `<state>/review/<stem>/`, never a checkout.
pub fn reader_dir(root: &Path, pr: &PrRef) -> PathBuf {
    review_home(root).join(reader_stem(pr))
}
