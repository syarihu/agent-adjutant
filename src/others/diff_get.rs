//! The diff a read-through was made from, kept at hand-in and served to the board page.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::task::PrRef;

use super::read_through::Stored;
use super::store;

/// The diffs over this size are not kept, only their size: a page has to take the answer whole.
pub const MAX_DIFF_BYTES: usize = 8 * 1024 * 1024;

/// A stored diff. `diff` is `""` for a PR with no changes and `null` for one over
/// `MAX_DIFF_BYTES`; a PR whose diff is not known has no file at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredDiff {
    pub sha: String,
    pub saved_at: String,
    pub bytes: u64,
    pub diff: Option<String>,
}

/// The diff kept for `pr`, as `{id, sha, bytes, diff}`: `None` when there is none, or when it is
/// of another head than the stored read-through (fail closed: annotations are anchored to the
/// diff they were read from). An error when a file cannot be read, which is not the same answer.
pub fn stored_diff(root: &Path, pr: &PrRef) -> Result<Option<Value>, String> {
    let stem = store::stem(pr);
    let Some(text) = store::load_diff(root, &stem)? else {
        return Ok(None);
    };
    let diff: StoredDiff = serde_json::from_str(&text)
        .map_err(|e| format!("cannot read the diff of {}: {e}", store::id_of(pr)))?;
    let Some(read) = store::load_read(root, &stem)? else {
        return Ok(None);
    };
    let read: Stored = serde_json::from_str(&read)
        .map_err(|e| format!("cannot read the read-through of {}: {e}", store::id_of(pr)))?;
    if read.rest.get("sha").and_then(Value::as_str) != Some(diff.sha.as_str()) {
        return Ok(None);
    }
    Ok(Some(json!({
        "id": store::id_of(pr),
        "sha": diff.sha,
        "bytes": diff.bytes,
        "diff": diff.diff,
    })))
}
