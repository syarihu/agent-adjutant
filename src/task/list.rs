//! Reading a hub's task records.

use super::*;

/// Every task hub `slug` has under the state root `root`, queue order first. A file that
/// does not parse is skipped.
pub fn list(root: &Path, slug: &str) -> Vec<Task> {
    store::list(root, slug)
}
