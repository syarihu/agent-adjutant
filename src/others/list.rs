//! The records, in the order the page lists them, and when the last sync ran.

use std::path::Path;

use super::model::{LastSync, Record};
use super::store;

/// Every record, by repository (case-insensitive), then the time the request arrived (a record
/// with none last), then number.
pub fn list(root: &Path) -> Vec<Record> {
    let mut records = store::list(root);
    sort(&mut records);
    records
}

fn sort(records: &mut [Record]) {
    records.sort_by(|a, b| {
        let key = |r: &Record| (r.requested_at.is_none(), r.requested_at.clone());
        a.repo
            .to_ascii_lowercase()
            .cmp(&b.repo.to_ascii_lowercase())
            .then_with(|| key(a).cmp(&key(b)))
            .then_with(|| a.number.cmp(&b.number))
    });
}

/// How the last sync went; `None` before the first one.
pub fn last_sync(root: &Path) -> Option<LastSync> {
    store::read_last_sync(root)
}
