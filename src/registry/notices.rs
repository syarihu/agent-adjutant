//! A queue of notifications for the client that is open to ring them.
//!
//! When the desktop app or a board page is open, a notification is not run through the
//! machine's own notifier: it is queued here, and the client takes it with its next poll and
//! rings it itself. A notice is one file, and taking it is a rename, so of every server
//! process and every poll exactly one gets each.

use super::notify_clients::PAGE_FRESH_SECS;
use super::store::notices_dir;
use super::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// How long a notice may wait for its client before the server takes it back and rings it with
/// the machine's notifier. A client counted as open polled within `PAGE_FRESH_SECS` and a hidden
/// one polls about once a minute, so this is longer than that: a shorter limit would take back
/// the notice of a window that is merely throttled.
pub const NOTICE_EXPIRY_SECS: i64 = PAGE_FRESH_SECS + 10;

/// A file left behind by a taker that stopped between the rename and the delete, or one that
/// cannot be read at all, is removed once it is this old.
const LEFTOVER_SECS: u64 = 10 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub id: String,
    pub title: String,
    pub message: String,
    #[serde(default)]
    pub nwo: String,
    pub created_at: i64,
    /// What the page rings the same event under, for a notice that repeats what a page may ring
    /// from its own state (a wait or a gate): the page shows one of the two. Absent on a notice
    /// from before this key, and on one that nothing else announces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Queue a notice. The id sorts by time, so the oldest is taken first.
pub fn enqueue_notice(
    root: &Path,
    nwo: &str,
    title: &str,
    message: &str,
    now: i64,
) -> Result<String, String> {
    enqueue_notice_tagged(root, nwo, title, message, now, None)
}

/// `enqueue_notice` for what the page announces from its own state too, named by `tag`.
pub fn enqueue_notice_tagged(
    root: &Path,
    nwo: &str,
    title: &str,
    message: &str,
    now: i64,
    tag: Option<&str>,
) -> Result<String, String> {
    let id = format!(
        "{now:012}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let notice = Notice {
        id: id.clone(),
        title: title.to_string(),
        message: message.to_string(),
        nwo: nwo.to_string(),
        created_at: now,
        tag: tag.map(str::to_string),
    };
    write_json(&notices_dir(root).join(format!("{id}.json")), &notice)?;
    Ok(id)
}

/// The queued notices' files, oldest first.
fn queued(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(notices_dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

/// Take the notice in `path` for this caller alone: renamed away first, so of two callers only
/// one finds it, then read and deleted. `None` for one somebody else took, and for one that is
/// not a notice, which is gone with it.
fn take(path: &Path) -> Option<Notice> {
    let taken = path.with_extension("taken");
    std::fs::rename(path, &taken).ok()?;
    let notice = read_json(&taken).and_then(|v| serde_json::from_value::<Notice>(v).ok());
    let _ = remove_if_present(&taken);
    notice
}

/// Take up to `max` queued notices, oldest first.
pub fn claim_notices(root: &Path, max: usize) -> Vec<Notice> {
    queued(root)
        .iter()
        .filter_map(|path| take(path))
        .take(max)
        .collect()
}

/// Take the notices no client has taken within `NOTICE_EXPIRY_SECS`, so that they can be rung
/// another way. Also clears the leftovers of a taker that stopped half way.
pub fn take_expired_notices(root: &Path, now: i64) -> Vec<Notice> {
    let mut expired = Vec::new();
    for path in queued(root) {
        match read_json(&path).and_then(|v| serde_json::from_value::<Notice>(v).ok()) {
            Some(notice) if now - notice.created_at >= NOTICE_EXPIRY_SECS => {
                if let Some(taken) = take(&path) {
                    expired.push(taken);
                }
            }
            Some(_) => {}
            // Not readable (yet, or ever): left for its writer to finish, removed when old.
            None => remove_when_old(&path),
        }
    }
    for entry in std::fs::read_dir(notices_dir(root))
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "taken") {
            remove_when_old(&path);
        }
    }
    expired
}

fn remove_when_old(path: &Path) {
    let old = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|at| at.elapsed().ok())
        .is_some_and(|age| age.as_secs() >= LEFTOVER_SECS);
    if old {
        let _ = remove_if_present(path);
    }
}

#[cfg(test)]
mod tests;
