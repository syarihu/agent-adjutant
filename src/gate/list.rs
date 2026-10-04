//! Reading a hub's gates by shelf.

use std::path::Path;

use super::store::{dir, list_where};
use super::*;

/// Every gate on `shelf`, oldest first — which for the open shelf is the order they should
/// be worked through.
pub fn list(root: &Path, slug: &str, shelf: Shelf) -> Vec<Gate> {
    list_where(&dir(root, slug, shelf), |_| true)
}

/// The gates of one kind on `shelf`, told apart by their file names before any is read. For
/// the archive, which only grows: the board asks it for plans on every poll, and parsing every
/// diff ever answered to find them would cost more each day.
pub fn list_of_kind(root: &Path, slug: &str, shelf: Shelf, kind: Kind) -> Vec<Gate> {
    // `{stamp}-{kind}`, `{stamp}-{kind}-{seq}` or `{stamp}-{kind}-record`. No kind's name
    // begins another's, so the prefix is enough; the kind is checked again once parsed.
    list_where(&dir(root, slug, shelf), |id| {
        id.split_once('-')
            .is_some_and(|(_, rest)| rest.starts_with(kind.as_str()))
    })
    .into_iter()
    .filter(|g| g.kind == kind)
    .collect()
}
