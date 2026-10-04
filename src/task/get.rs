//! Reading one task record.

use super::*;

/// Task `id` of hub `slug` under the state root `root`.
///
/// "no such task" for an id that is not plain or has no record, "cannot read" for a record
/// that does not parse.
pub fn get(root: &Path, slug: &str, id: &str) -> Result<Task, String> {
    store::load(root, slug, id)
}
