//! Deleting a task record.

use super::*;

use crate::registry::Context;

/// Delete task `id`'s record: the reverse of `create`.
///
/// `<id>.lock` is left where it is. A lock file that is unlinked can be locked twice — a holder
/// keeps the old inode while the next caller creates a new one (#291) — and no listing reads a
/// `.lock`.
pub fn remove(ctx: &Context, id: &str) -> Result<(), String> {
    if !is_plain_id(id) {
        return Err(format!("no such task: {id}"));
    }
    // Under the lock, so a writer that loaded the record before this cannot save it back.
    let _lock = store::lock(ctx, id)?;
    let path = store::path_of(&store::dir_of(ctx), id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!("no such task: {id}")),
        Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
    }
}
