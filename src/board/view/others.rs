//! The PRs that asked for your review, as the page reads them.

use std::path::Path;

use serde::Serialize;

use crate::infra::clock::now_secs;
use crate::others::{LastSync, Record, last_sync, list};

/// The document `GET /api/others` sends.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OthersState {
    pub now: i64,
    /// `None` before the first sync.
    pub last_sync: Option<LastSync>,
    pub records: Vec<Record>,
}

/// Read-only: a sync is the only thing that writes these records.
pub fn others(root: &Path) -> OthersState {
    OthersState {
        now: now_secs(),
        last_sync: last_sync(root),
        records: list(root),
    }
}
