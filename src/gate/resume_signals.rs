//! What can show a worker moved on from a waiting gate.

use std::path::Path;

use super::store::{dir, list_modified_since, path_of};
use super::*;

/// The gates that can show a worker in `open` (one hub's open gates, as `list` read them)
/// moved on to something else: `open` itself, and what the hub's records and answered gates
/// hold that was written since the earliest waiting gate was opened. Only reads, so a board
/// can ask it about a hub that is not its own.
///
/// A gate already answered on the board still shows the worker got as far as opening it. The
/// archive only grows, so it is not parsed whole: a file older than the earliest waiting gate
/// cannot be a signal. A little slack for coarse file times; the mtime only prunes, and
/// `resumed_at` and the caller's filter decide.
pub fn resume_signals(root: &Path, slug: &str, open: &[Gate]) -> Vec<Gate> {
    let open_dir = dir(root, slug, Shelf::Open);
    let since = open
        .iter()
        .filter(|g| g.wait && !g.answered_by_hub())
        .filter_map(|g| {
            std::fs::metadata(path_of(&open_dir, &g.id))
                .and_then(|m| m.modified())
                .ok()
        })
        .min()
        .map(|t| t - std::time::Duration::from_secs(2))
        .unwrap_or(std::time::UNIX_EPOCH);
    open.iter()
        .cloned()
        .chain(list_modified_since(&dir(root, slug, Shelf::Record), since))
        .chain(list_modified_since(
            &dir(root, slug, Shelf::Answered),
            since,
        ))
        .filter(|g| !g.answered_by_hub())
        .collect()
}
