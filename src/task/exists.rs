//! Whether a task record is there.

use super::*;

/// Whether task `id` of hub `slug` has a record under the state root `root`.
///
/// `Ok(false)` only when there is definitely none: an id that is not plain names no file, and a
/// missing file is no record. A record that is there and cannot be read still exists, and a
/// directory that cannot be looked at is an error, so a caller that lets go of something with the
/// task does so only on `Ok(false)`.
pub fn exists(root: &Path, slug: &str, id: &str) -> Result<bool, String> {
    if !is_plain_id(id) {
        return Ok(false);
    }
    let path = store::path_of(&store::dir(root, slug), id);
    crate::infra::fs::record_exists(&path)
        .map_err(|e| format!("cannot look for {}: {e}", path.display()))
}
