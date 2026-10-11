//! Where the records of PRs asked of you are kept, and how a sync is kept from running twice.
//!
//! ```text
//! <state>/others/<owner>~<repo>~<number>.json   one record (`~` is never in a GitHub name)
//! <state>/others/<owner>~<repo>~<number>.lock   its write lock
//! <state>/others/sync.json                      how the last sync went
//! <state>/others/sync.lock                      held for the whole of a sync
//! <state>/others/reads/<owner>~<repo>~<number>.json   the PR's latest read-through
//! <state>/others/drafts/<owner>~<repo>~<number>.json  the person's draft review comments
//! <state>/others/diffs/<owner>~<repo>~<number>.json   the diff the stored read-through was made from
//! <state>/others/review.lock                    held while readers are counted and claimed
//! ```

use std::path::{Path, PathBuf};

use crate::task::PrRef;

use super::model::{LastSync, Record, derive, is_stale};

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
pub(super) fn stem_of_id(id: &str) -> String {
    id.replace(['/', '#'], "~")
}

fn path_of(root: &Path, stem: &str) -> PathBuf {
    dir(root).join(format!("{stem}.json"))
}

/// What is derived is never read back from disk: the state follows the facts, so a writer that
/// sets only a fact (the read-through's result) needs no second write to move the state.
fn derived(mut record: Record) -> Record {
    derive_in_place(&mut record);
    record
}

/// Set what is derived: the state, why it is done, and whether the read-through is of an
/// older head.
fn derive_in_place(record: &mut Record) {
    let (state, reason) = derive(record);
    record.state = state;
    record.done_reason = reason;
    let stale = is_stale(record);
    if let Some(ai) = record.ai.as_mut() {
        ai.stale = stale;
    }
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
    derive_in_place(record);
    crate::infra::fs::write_json(&path_of(root, &stem_of_id(&record.id)), record)
}

/// Remove a record's file. The lock file beside it stays: unlinking a lock file while another
/// process holds it open hands the next two callers two different locks.
///
/// What was made for the record goes with it, the read-through, its diff, the drafts and the
/// reader's scratch directory: a PR that is reopened later must not be served the read of an earlier time.
pub(super) fn remove(root: &Path, stem: &str) -> Result<(), String> {
    let scratch = super::reader_dir::review_home(root).join(stem);
    // The record last: one that could not be fully removed stays listed, and a later sync tries again.
    crate::infra::fs::remove_if_present(&read_path(root, stem))?;
    remove_diff(root, stem)?;
    crate::infra::fs::remove_if_present(&drafts_path(root, stem))?;
    match std::fs::remove_dir_all(&scratch) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "cannot remove the reader's directory of {stem}: {e}"
            ));
        }
    }
    crate::infra::fs::remove_if_present(&path_of(root, stem))
}

/// Hold the write lock of one record until the returned handle is dropped: a load, a change and
/// a save are one step, and the sync and whoever writes the read-through must not undo each other.
pub(super) fn lock(root: &Path, stem: &str) -> Result<std::fs::File, String> {
    crate::infra::fs::lock(&dir(root).join(format!("{stem}.lock")))
}

/// Where the read-throughs are: one file per PR, in a directory of their own so that `list`
/// does not try to read one as a record.
fn reads_dir(root: &Path) -> PathBuf {
    dir(root).join("reads")
}

fn read_path(root: &Path, stem: &str) -> PathBuf {
    reads_dir(root).join(format!("{stem}.json"))
}

/// The text of a PR's stored read-through: `None` when there is none, an error when there is
/// one that cannot be read.
pub(super) fn load_read(root: &Path, stem: &str) -> Result<Option<String>, String> {
    let path = read_path(root, stem);
    match crate::infra::fs::record_exists(&path) {
        Ok(false) => return Ok(None),
        Ok(true) => {}
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Where the diffs of the read-throughs are: a directory of their own, for the same reason as
/// `reads`.
fn diffs_dir(root: &Path) -> PathBuf {
    dir(root).join("diffs")
}

fn diff_path(root: &Path, stem: &str) -> PathBuf {
    diffs_dir(root).join(format!("{stem}.json"))
}

/// The text of a PR's stored diff: `None` when there is none, an error when there is one that
/// cannot be read.
pub(super) fn load_diff(root: &Path, stem: &str) -> Result<Option<String>, String> {
    let path = diff_path(root, stem);
    match crate::infra::fs::record_exists(&path) {
        Ok(false) => return Ok(None),
        Ok(true) => {}
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Replace a PR's stored diff whole.
pub(super) fn write_diff(
    root: &Path,
    stem: &str,
    diff: &impl serde::Serialize,
) -> Result<(), String> {
    crate::infra::fs::write_json(&diff_path(root, stem), diff)
}

/// Remove a PR's stored diff; none stored is not an error.
pub(super) fn remove_diff(root: &Path, stem: &str) -> Result<(), String> {
    crate::infra::fs::remove_if_present(&diff_path(root, stem))
}

/// Put back what `load_diff` gave before a change: its text as it was, or no file when there
/// was none.
pub(super) fn restore_diff(root: &Path, stem: &str, previous: Option<&str>) -> Result<(), String> {
    let Some(text) = previous else {
        return remove_diff(root, stem);
    };
    write_text(&diff_path(root, stem), text)
}

/// Where the person's draft comments are: a directory of their own, for the same reason as
/// `reads`.
fn drafts_dir(root: &Path) -> PathBuf {
    dir(root).join("drafts")
}

fn drafts_path(root: &Path, stem: &str) -> PathBuf {
    drafts_dir(root).join(format!("{stem}.json"))
}

/// The text of a PR's stored drafts: `None` when there are none, an error when there is a file
/// that cannot be read.
pub(super) fn load_drafts(root: &Path, stem: &str) -> Result<Option<String>, String> {
    let path = drafts_path(root, stem);
    match crate::infra::fs::record_exists(&path) {
        Ok(false) => return Ok(None),
        Ok(true) => {}
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Replace a PR's stored drafts whole.
pub(super) fn write_drafts(
    root: &Path,
    stem: &str,
    drafts: &impl serde::Serialize,
) -> Result<(), String> {
    crate::infra::fs::write_json(&drafts_path(root, stem), drafts)
}

/// Replace a PR's stored read-through whole.
pub(super) fn write_read(
    root: &Path,
    stem: &str,
    read: &impl serde::Serialize,
) -> Result<(), String> {
    crate::infra::fs::write_json(&read_path(root, stem), read)
}

/// Put back what `load_read` gave before a change: its text as it was, or no file when there
/// was none.
pub(super) fn restore_read(root: &Path, stem: &str, previous: Option<&str>) -> Result<(), String> {
    let path = read_path(root, stem);
    let Some(text) = previous else {
        return crate::infra::fs::remove_if_present(&path);
    };
    write_text(&path, text)
}

/// Replace the file at `path` with `text` in one step, as `write_json` does for a value.
pub(super) fn write_text(path: &Path, text: &str) -> Result<(), String> {
    let staged = crate::infra::fs::stage(crate::infra::fs::parent_dir(path)?, text)?;
    std::fs::rename(&staged, path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("cannot write {}: {e}", path.display())
    })
}

/// Hold the lock that readers are counted and claimed under, until the handle is dropped: the
/// count spans records that each have a lock of their own, so it has one more. Taken before a
/// record's lock, never after.
pub(super) fn review_lock(root: &Path) -> Result<std::fs::File, String> {
    crate::infra::fs::lock(&dir(root).join("review.lock"))
}

/// The state roots a sync of this process is running on. A sync of this process is the one a
/// second click can meet, and it is answered at once from here.
static RUNNING: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// The lock of a running sync: its root's place among `RUNNING`, and the file lock once taken.
/// The place is given back by `Drop`, so every way out of `try_lock_sync` gives it back.
pub(super) struct SyncLock {
    root: PathBuf,
    _file: Option<std::fs::File>,
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        // The file first, so a root missing from `RUNNING` is never still held by this sync.
        self._file.take();
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        running.retain(|r| r != &self.root);
    }
}

/// The lock a sync holds from start to end; `None` when another one holds it.
///
/// A sync of this process is found in `RUNNING` and refused at once. A file lock held with none
/// of ours running is asked again for a while before it is believed: a process that forks and
/// takes time to exec (a pty's child between `fork` and `exec`) holds a copy of every open file
/// until it execs, and `flock` belongs to the open file, not to the descriptor, so the lock of a
/// sync that has just ended can look held to the next one.
pub(super) fn try_lock_sync(root: &Path) -> Result<Option<SyncLock>, String> {
    try_lock_sync_asking(root, LOCK_ASKS, LOCK_WAIT)
}

pub(super) fn try_lock_sync_asking(
    root: &Path,
    asks: u32,
    wait: std::time::Duration,
) -> Result<Option<SyncLock>, String> {
    let mut lock = {
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if running.iter().any(|r| r == root) {
            return Ok(None);
        }
        running.push(root.to_path_buf());
        // From here the guard owns the place, whatever way this function is left.
        SyncLock {
            root: root.to_path_buf(),
            _file: None,
        }
    };
    let path = dir(root).join("sync.lock");
    for attempt in 0..asks {
        if let Some(file) = crate::infra::fs::try_lock(&path)? {
            lock._file = Some(file);
            return Ok(Some(lock));
        }
        if attempt + 1 < asks {
            std::thread::sleep(wait);
        }
    }
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
