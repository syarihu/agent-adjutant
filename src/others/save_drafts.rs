//! Saving the person's draft comments of a PR, whole.

use std::path::Path;

use serde_json::Value;

use crate::task::PrRef;

use super::drafts::{DraftItem, DraftsInput, StoredDrafts, validate};
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

/// A draft the page sent without `sentAt` keeps the one stored for it while it is the draft that
/// was sent, in its body and in the place it is on: a save already on its way, or another tab, must not take a draft back out of the
/// pending review (and have it sent twice). Edited or moved since, it is a new
/// comment and goes unmarked.
fn keep_sent_marks(input: &mut DraftsInput, was: &DraftsInput) {
    for d in input.drafts.iter_mut().filter(|d| d.sent_at.is_none()) {
        if let Some(sent) = was.drafts.iter().find(|w| {
            w.sent_at.is_some()
                && DraftItem {
                    sent_at: None,
                    ..(*w).clone()
                } == *d
        }) {
            d.sent_at.clone_from(&sent.sent_at);
        }
    }
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
    // A page that predates a key does not send it: what an earlier save holds under a key this
    // input has no place for stays (rule 10). A file that cannot be read is an error, not an
    // empty one, so nothing is dropped unseen.
    let old = match store::load_drafts(root, &stem).map_err(SaveDraftsError::Failed)? {
        Some(text) => Some(serde_json::from_str::<StoredDrafts>(&text).map_err(|e| {
            SaveDraftsError::Failed(format!(
                "cannot read the drafts of {}: {e}",
                store::id_of(pr)
            ))
        })?),
        None => None,
    };
    let mut input = input;
    if let Some(Ok(was)) = old.as_ref().map(StoredDrafts::input) {
        keep_sent_marks(&mut input, &was);
    }
    let mut stored = StoredDrafts::of(&store::id_of(pr), saved_at, &input);
    if let Some(old) = old {
        for (key, value) in old.rest {
            // A key of the input that this save left out (`fromMode` of a read) is not kept: it was the input's to say.
            if !super::drafts::INPUT_KEYS.contains(&key.as_str()) {
                stored.rest.entry(key).or_insert(value);
            }
        }
    }
    store::write_drafts(root, &stem, &stored).map_err(SaveDraftsError::Failed)?;
    serde_json::to_value(stored).map_err(|e| SaveDraftsError::Failed(e.to_string()))
}
