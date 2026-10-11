//! One record, for a reader that must tell "no record" from "a record that cannot be read".

use std::path::Path;

use crate::task::PrRef;

use super::model::Record;
use super::store;

/// The record of `pr`: `None` when there is none, and an error when there is one that cannot be
/// read. `list` skips such a file so that one bad record does not blank the page; a caller that
/// acts on one record must not mistake it for a missing one.
pub fn record(root: &Path, pr: &PrRef) -> Result<Option<Record>, String> {
    store::load(root, pr)
}
