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

/// The name of a reader's directory for the file stem `stem`: the stem with `+` where it has `~`.
/// Claude Code takes a name ending in `~<digit>` for a Windows 8.3 short name and asks about every
/// file under it, which a reader that may not ask cannot work with. GitHub allows no `+` in an
/// owner or repository name, so the name still maps back to one PR.
pub fn reader_dir_name(stem: &str) -> String {
    stem.replace('~', "+")
}

/// The directory a PR's reader works in: `<state>/review/<name>/` (see `reader_dir_name`), never
/// a checkout.
pub fn reader_dir(root: &Path, pr: &PrRef) -> PathBuf {
    review_home(root).join(reader_dir_name(&reader_stem(pr)))
}
