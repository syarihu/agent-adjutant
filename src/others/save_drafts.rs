//! Saving the person's draft comments of a PR, whole.

use std::path::Path;

use serde_json::Value;

use crate::task::PrRef;

use super::drafts::{DraftsInput, StoredDrafts, validate};
use super::store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveDraftsError {
    /// The input breaks the contract: one problem per entry, all of them.
    Invalid(Vec<String>),
    /// adj has no record of this PR.
    NoRecord,
    /// The record or the file could not be read or written, and why.
    Failed(String),
}

/// Replace the drafts of `pr` with `input` (the object the page keeps, without `id` and
/// `savedAt`), stamped `saved_at`, and answer what is stored.
///
/// A done record may still be saved: the person may want to copy what they wrote. Everything
/// that can refuse does so before the first write, and before the lock is taken, so a refused
/// save leaves not even a lock file.
pub fn save_drafts(
    root: &Path,
    pr: &PrRef,
    input: &Value,
    saved_at: &str,
) -> Result<Value, SaveDraftsError> {
    // Not left to the parse: serde reads a JSON array as the fields of a struct in order, so `[]`
    // would be taken for drafts with every default.
    if !input.is_object() {
        return Err(SaveDraftsError::Invalid(vec![
            "the drafts must be an object".to_string(),
        ]));
    }
    let input: DraftsInput = serde_json::from_value(input.clone())
        .map_err(|e| SaveDraftsError::Invalid(vec![format!("cannot read the drafts: {e}")]))?;
    let problems = validate(&input);
    if !problems.is_empty() {
        return Err(SaveDraftsError::Invalid(problems));
    }
    if store::load(root, pr)
        .map_err(SaveDraftsError::Failed)?
        .is_none()
    {
        return Err(SaveDraftsError::NoRecord);
    }
    let stem = store::stem(pr);
    // The record's removal takes the drafts with it under this lock, so a save that lost the race
    // finds the record gone here and does not write a file nothing will ever remove.
    let _lock = store::lock(root, &stem).map_err(SaveDraftsError::Failed)?;
    if store::load(root, pr)
        .map_err(SaveDraftsError::Failed)?
        .is_none()
    {
        return Err(SaveDraftsError::NoRecord);
    }
    let mut stored = StoredDrafts::of(&store::id_of(pr), saved_at, &input);
    // A page that predates a key does not send it: what an earlier save holds under a key this
    // input has no place for stays (rule 10). A file that cannot be read is an error, not an
    // empty one, so nothing is dropped unseen.
    if let Some(text) = store::load_drafts(root, &stem).map_err(SaveDraftsError::Failed)? {
        let old: StoredDrafts = serde_json::from_str(&text).map_err(|e| {
            SaveDraftsError::Failed(format!(
                "cannot read the drafts of {}: {e}",
                store::id_of(pr)
            ))
        })?;
        for (key, value) in old.rest {
            stored.rest.entry(key).or_insert(value);
        }
    }
    store::write_drafts(root, &stem, &stored).map_err(SaveDraftsError::Failed)?;
    serde_json::to_value(stored).map_err(|e| SaveDraftsError::Failed(e.to_string()))
}
