//! Reading one gate, open or kept as a record.

use std::path::Path;

use super::store::{exists, load};
use super::*;

/// A gate by id, open or kept as a record. Open first: that is what an id usually names, and
/// a record's id cannot be an open gate's (see `claim_id`).
pub fn get(root: &Path, slug: &str, id: &str) -> Result<Gate, String> {
    // Only a missing file falls through: one that is there and broken says so, rather than
    // reading as an id that does not exist.
    if exists(root, slug, Shelf::Open, id) {
        return load(root, slug, Shelf::Open, id);
    }
    if exists(root, slug, Shelf::Record, id) {
        return load(root, slug, Shelf::Record, id);
    }
    Err(format!("no open gate or record: {id}"))
}
