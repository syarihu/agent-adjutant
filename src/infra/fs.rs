use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// How many names are tried before a claim gives up. Same-second sends are normal (a
/// worker filing two findings at once), so the counter is not an edge case to skip; a
/// thousand of them in one second is not a collision but a runaway.
pub(crate) const CLAIM_ATTEMPTS: usize = 1000;

pub(crate) fn remove_if_present(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
    }
}

/// Write `text` into `dir` under a name `list` will not return, so the file can be linked
/// into place complete. Staging inside the destination directory rather than in a temporary
/// one is what keeps the link possible: `hard_link` cannot cross a filesystem.
pub(crate) fn stage(dir: &Path, text: &str) -> Result<PathBuf, String> {
    stage_as(dir, text, false)
}

/// `stage`, readable by its owner only when `private` (unix; elsewhere the default).
fn stage_as(dir: &Path, text: &str, private: bool) -> Result<PathBuf, String> {
    use std::io::Write;
    for attempt in 0..CLAIM_ATTEMPTS {
        // Concurrent stagers are threads as well as processes, so the pid alone is not
        // unique; `create_new` plus a counter settles both.
        let path = dir.join(format!(".staging-{}-{attempt}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        match options.open(&path) {
            Ok(mut file) => {
                // A write that fails leaves a file behind — invisible to `list`, so it grows
                // unnoticed, and after a thousand of them nothing can be staged at all.
                // `sync_all` before the caller publishes it: the point of writing here and
                // linking there is that the name never points at an incomplete file, and
                // without this that holds for a crash but not for a power cut.
                let written = file
                    .write_all(text.as_bytes())
                    .and_then(|()| file.sync_all());
                return match written {
                    Ok(()) => Ok(path),
                    Err(e) => {
                        let _ = std::fs::remove_file(&path);
                        Err(format!("cannot write {}: {e}", path.display()))
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write {}: {e}", path.display())),
        }
    }
    Err(format!(
        "cannot stage a file in {} after {CLAIM_ATTEMPTS} tries",
        dir.display()
    ))
}

/// Make an empty file at `path` only if nothing is there: `true` when this call made it,
/// `false` when the name was taken. Any other error goes back so each caller keeps its wording.
pub(crate) fn create_new(path: &Path) -> std::io::Result<bool> {
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

/// Open the lock file at `path`, making its directory first. Never truncated and never removed:
/// unlinking a lock file while another process holds it open hands the next two callers two
/// different locks. The lock is advisory and released by the system when its holder exits.
pub(crate) fn open_lock(path: &Path) -> Result<std::fs::File, String> {
    parent_dir(path)?;
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))
}

/// Hold the lock at `path` until the returned handle is dropped, waiting for it if need be.
pub(crate) fn lock(path: &Path) -> Result<std::fs::File, String> {
    let file = open_lock(path)?;
    file.lock()
        .map_err(|e| format!("cannot lock {}: {e}", path.display()))?;
    Ok(file)
}

/// `lock` without waiting: `None` when another handle holds it. What to do then is the caller's.
pub(crate) fn try_lock(path: &Path) -> Result<Option<std::fs::File>, String> {
    let file = open_lock(path)?;
    Ok(try_hold(&file, path)?.then_some(file))
}

/// How many times a lock is taken again because its path had been unlinked and made anew.
const LOCK_RETRIES: usize = 5;

/// Whether `path` still names the file `held` is open on. A missing path does not. Where inodes
/// cannot be compared, it is taken to.
fn names(held: &std::fs::File, path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (held.metadata(), std::fs::metadata(path)) {
            (Ok(held), Ok(now)) => held.dev() == now.dev() && held.ino() == now.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (held, path);
        true
    }
}

/// `lock` for a lock file somebody may remove: one that has been unlinked while this waited, or
/// between its `open` and its `lock`, is a lock nobody else is using, so the handle is dropped and
/// the path opened again. Two callers then never hold two different locks for one path.
///
/// The check of the path is unix-only (it compares inodes); elsewhere this is `lock`, and a
/// caller that unlinks lock files must not do it there.
pub(crate) fn lock_checked(path: &Path) -> Result<std::fs::File, String> {
    lock_checked_with(path, || {})
}

/// `lock_checked`, calling `between` after each lock is taken and before the path is checked:
/// where a test replaces the file.
fn lock_checked_with(path: &Path, mut between: impl FnMut()) -> Result<std::fs::File, String> {
    for _ in 0..LOCK_RETRIES {
        let file = lock(path)?;
        between();
        if names(&file, path) {
            return Ok(file);
        }
    }
    Err(format!(
        "cannot lock {}: it keeps being replaced",
        path.display()
    ))
}

/// `try_lock` with the check of `lock_checked`: a lock that was replaced is `None`, as if held.
pub(crate) fn try_lock_checked(path: &Path) -> Result<Option<std::fs::File>, String> {
    try_lock_checked_with(path, || {})
}

fn try_lock_checked_with(
    path: &Path,
    between: impl FnOnce(),
) -> Result<Option<std::fs::File>, String> {
    let Some(file) = try_lock(path)? else {
        return Ok(None);
    };
    between();
    Ok(names(&file, path).then_some(file))
}

/// One attempt to lock a file `open_lock` opened: `false` when another handle holds it. For a
/// caller that polls, so that it opens the file once rather than on every attempt.
pub(crate) fn try_hold(file: &std::fs::File, path: &Path) -> Result<bool, String> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("cannot lock {}: {e}", path.display())),
    }
}

// ── plumbing ─────────────────────────────────────────────────────────

fn render_json<T: Serialize + ?Sized>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|s| s + "\n")
        .map_err(|e| e.to_string())
}

pub(crate) fn parent_dir(path: &Path) -> Result<&Path, String> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    Ok(parent)
}

/// Replace whatever is at `path` with `value`, in one step.
///
/// A record is read by a process other than the one writing it, and a reader that catches
/// a half-written file reads no record at all — which for a presence check means a live
/// session reported as absent. Writing beside the record, syncing it and renaming over it
/// means the name never points at a partial file, even after a crash.
pub(crate) fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    write_json_as(path, value, false)
}

/// `write_json` for a record that holds text a person would not paste anywhere: the file is
/// readable by its owner only, from the moment it exists.
pub(crate) fn write_json_private<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
) -> Result<(), String> {
    write_json_as(path, value, true)
}

fn write_json_as<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    private: bool,
) -> Result<(), String> {
    let parent = parent_dir(path)?;
    let staged = stage_as(parent, &render_json(value)?, private)?;
    std::fs::rename(&staged, path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("cannot write {}: {e}", path.display())
    })
}

/// `write_json` for a file somebody else owns and may have tightened: the new one takes the old
/// one's permission bits (unix), so a `0600` file does not come back `0644`.
pub(crate) fn replace_json(path: &Path, value: &Value) -> Result<(), String> {
    let parent = parent_dir(path)?;
    let staged = stage(parent, &render_json(value)?)?;
    let finish = || -> Result<(), String> {
        #[cfg(unix)]
        if let Ok(old) = std::fs::metadata(path) {
            std::fs::set_permissions(&staged, old.permissions())
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
        std::fs::rename(&staged, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
    };
    finish().inspect_err(|_| {
        let _ = std::fs::remove_file(&staged);
    })
}

pub(crate) enum CreateError {
    /// The name already exists. Not a failure — an answer.
    Taken,
    Failed(String),
}

/// Write `value` at `path` only if nothing is there, and say which of the two happened.
pub(crate) fn create_new_json(path: &Path, value: &Value) -> Result<(), CreateError> {
    let parent = parent_dir(path).map_err(CreateError::Failed)?;
    let text = render_json(value).map_err(CreateError::Failed)?;
    let staged = stage(parent, &text).map_err(CreateError::Failed)?;
    let result = match std::fs::hard_link(&staged, path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(CreateError::Taken),
        Err(e) => Err(CreateError::Failed(format!(
            "cannot write {}: {e}",
            path.display()
        ))),
    };
    let _ = std::fs::remove_file(&staged);
    result
}

/// Whether anything at all is at `path`, a symlink included — with "cannot tell" kept apart.
///
/// `symlink_metadata` rather than `try_exists`: `try_exists` follows a symlink, so one whose
/// target has gone answers "nothing here", and "nothing here" is the answer each caller goes
/// on to act on. Asked about the link itself, the answer is that something is there — and the
/// read that follows fails, which is the "there and cannot be read" each caller already has.
pub(crate) fn record_exists(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

pub(crate) fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_new_answers_false_on_a_taken_name_and_leaves_it_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claim");
        assert!(create_new(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        std::fs::write(&path, "kept").unwrap();
        assert!(!create_new(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept");
    }

    #[test]
    fn create_new_hands_other_errors_back() {
        let dir = tempfile::tempdir().unwrap();
        let err = create_new(&dir.path().join("missing").join("claim")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn try_lock_answers_none_while_another_handle_holds_it_and_takes_it_after() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deeper").join("a.lock");
        let held = lock(&path).unwrap();
        assert!(try_lock(&path).unwrap().is_none());
        drop(held);
        // A child another test forks shares the lock until it execs, so allow it a moment.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while try_lock(&path).unwrap().is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "still held after it was dropped"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_lock_whose_path_was_replaced_after_it_was_taken_is_taken_again_on_the_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.lock");
        let mut replaced = 0;
        let got = lock_checked_with(&path, || {
            // Once: as a sweep that unlinked the path just after this took the old file.
            if replaced == 0 {
                std::fs::remove_file(&path).unwrap();
                drop(open_lock(&path).unwrap());
            }
            replaced += 1;
        })
        .unwrap();
        assert_eq!(replaced, 2);
        assert!(names(&got, &path));
        assert!(try_lock(&path).unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_try_lock_on_a_path_that_was_replaced_is_not_a_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.lock");
        let replaced = try_lock_checked_with(&path, || {
            std::fs::remove_file(&path).unwrap();
            drop(open_lock(&path).unwrap());
        })
        .unwrap();
        assert!(replaced.is_none());
        assert!(try_lock_checked(&path).unwrap().is_some());
        // And a path that is gone is not the file that was locked either.
        let gone = try_lock_checked_with(&path, || std::fs::remove_file(&path).unwrap()).unwrap();
        assert!(gone.is_none());
    }

    #[test]
    fn open_lock_leaves_what_is_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.lock");
        std::fs::write(&path, "kept").unwrap();
        drop(open_lock(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept");
    }

    #[test]
    fn write_json_writes_a_typed_record_as_pretty_json() {
        #[derive(Serialize)]
        struct Record {
            id: String,
            count: u32,
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("record.json");
        let record = Record {
            id: "a".into(),
            count: 2,
        };
        write_json(&path, &record).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            serde_json::to_string_pretty(&record).unwrap() + "\n"
        );
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn replace_json_keeps_the_mode_of_the_file_it_replaces() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        replace_json(&path, &serde_json::json!({ "a": 1 })).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
