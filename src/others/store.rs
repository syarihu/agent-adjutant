//! Where the records of PRs asked of you are kept, and how a sync is kept from running twice.
//!
//! ```text
//! <state>/others/<owner>~<repo>~<number>.json   one record (`~` is never in a GitHub name)
//! <state>/others/<owner>~<repo>~<number>.lock   its write lock
//! <state>/others/sync.json                      how the last sync went
//! <state>/others/sync.lock                      held for the whole of a sync
//! ```

use std::path::{Path, PathBuf};

use crate::task::PrRef;

use super::model::{LastSync, Record, derive};

pub(super) fn dir(root: &Path) -> PathBuf {
    root.join("others")
}

/// The file stem of a PR's record. Owner and repository are already in lower case in a `PrRef`.
pub(super) fn stem(r: &PrRef) -> String {
    format!("{}~{}~{}", r.owner, r.repo, r.number)
}

/// `owner/repo#N`, the key a record carries.
pub(super) fn id_of(r: &PrRef) -> String {
    format!("{}/{}#{}", r.owner, r.repo, r.number)
}

/// The stem of a record, from its key: the one name both are made from.
fn stem_of_id(id: &str) -> String {
    id.replace(['/', '#'], "~")
}

fn path_of(root: &Path, stem: &str) -> PathBuf {
    dir(root).join(format!("{stem}.json"))
}

/// What is derived is never read back from disk: the state follows the facts, so a writer that
/// sets only a fact (the read-through's result) needs no second write to move the state.
fn derived(mut record: Record) -> Record {
    let (state, reason) = derive(&record);
    record.state = state;
    record.done_reason = reason;
    record
}

/// The record of `r`: `None` when there is none, and an error when there is one that cannot be
/// read, so a sync reports that PR rather than writing over a record it could not understand.
pub(super) fn load(root: &Path, r: &PrRef) -> Result<Option<Record>, String> {
    let path = path_of(root, &stem(r));
    match crate::infra::fs::record_exists(&path) {
        Ok(false) => return Ok(None),
        Ok(true) => {}
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str::<Record>(&text)
        .map(|record| Some(derived(record)))
        .map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Every record. A file that will not parse is skipped rather than fatal, as the task list
/// does: the directory may have been edited by hand, and one bad file must not blank the page.
pub(super) fn list(root: &Path) -> Vec<Record> {
    let Ok(entries) = std::fs::read_dir(dir(root)) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|path| {
            // `sync.json` and the staged `.staging-*` files have no `~` in a record's place.
            path.extension().is_some_and(|ext| ext == "json")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.contains('~') && !s.starts_with('.'))
        })
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str::<Record>(&text).ok())
        .map(derived)
        .collect()
}

/// Write `record` whole, with its state derived from the facts it holds.
pub(super) fn save(root: &Path, record: &mut Record) -> Result<(), String> {
    let (state, reason) = derive(record);
    record.state = state;
    record.done_reason = reason;
    crate::infra::fs::write_json(&path_of(root, &stem_of_id(&record.id)), record)
}

/// Remove a record's file. The lock file beside it stays: unlinking a lock file while another
/// process holds it open hands the next two callers two different locks.
pub(super) fn remove(root: &Path, stem: &str) -> Result<(), String> {
    crate::infra::fs::remove_if_present(&path_of(root, stem))
}

/// Hold the write lock of one record until the returned handle is dropped: a load, a change and
/// a save are one step, and the sync and whoever writes the read-through must not undo each other.
pub(super) fn lock(root: &Path, stem: &str) -> Result<std::fs::File, String> {
    crate::infra::fs::lock(&dir(root).join(format!("{stem}.lock")))
}

/// The state roots a sync of this process is running on. A sync of this process is the one a
/// second click can meet, and it is answered at once from here.
static RUNNING: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// The lock of a running sync: the file lock, and the root's place among `RUNNING`.
pub(super) struct SyncLock {
    _file: std::fs::File,
    root: PathBuf,
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        forget(&self.root);
    }
}

fn forget(root: &Path) {
    let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    running.retain(|r| r != root);
}

/// The lock a sync holds from start to end; `None` when another one holds it.
///
/// A sync of this process is found in `RUNNING` and refused at once. A file lock held with none
/// of ours running is asked again for a while before it is believed: a process that forks and
/// takes time to exec (a pty's child between `fork` and `exec`) holds a copy of every open file
/// until it execs, and `flock` belongs to the open file, not to the descriptor, so the lock of a
/// sync that has just ended can look held to the next one.
pub(super) fn try_lock_sync(root: &Path) -> Result<Option<SyncLock>, String> {
    {
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if running.iter().any(|r| r == root) {
            return Ok(None);
        }
        running.push(root.to_path_buf());
    }
    let path = dir(root).join("sync.lock");
    for attempt in 0..LOCK_ASKS {
        match crate::infra::fs::try_lock(&path) {
            Ok(Some(file)) => {
                return Ok(Some(SyncLock {
                    _file: file,
                    root: root.to_path_buf(),
                }));
            }
            Ok(None) if attempt + 1 < LOCK_ASKS => std::thread::sleep(LOCK_WAIT),
            Ok(None) => break,
            Err(e) => {
                forget(root);
                return Err(e);
            }
        }
    }
    forget(root);
    Ok(None)
}

const LOCK_ASKS: u32 = 100;
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_millis(20);

pub(super) fn read_last_sync(root: &Path) -> Option<LastSync> {
    serde_json::from_str(&std::fs::read_to_string(dir(root).join("sync.json")).ok()?).ok()
}

pub(super) fn write_last_sync(root: &Path, last: &LastSync) -> Result<(), String> {
    crate::infra::fs::write_json(&dir(root).join("sync.json"), last)
}
