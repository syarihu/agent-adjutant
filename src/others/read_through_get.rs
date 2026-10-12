//! The stored read-through of a PR, as the board page reads it.

use std::path::Path;

use serde_json::Value;

use crate::task::PrRef;

use super::read_through::Stored;
use super::store;

/// The read-through kept for `pr`: `None` when there is none, and an error when there is one
/// that cannot be read, which is not the same answer. Keys this binary does not know are in it.
pub fn read_through(root: &Path, pr: &PrRef) -> Result<Option<Value>, String> {
    let Some(text) = store::load_read(root, &store::stem(pr))? else {
        return Ok(None);
    };
    let stored: Stored = serde_json::from_str(&text)
        .map_err(|e| format!("cannot read the read-through of {}: {e}", store::id_of(pr)))?;
    serde_json::to_value(stored)
        .map(Some)
        .map_err(|e| e.to_string())
}
