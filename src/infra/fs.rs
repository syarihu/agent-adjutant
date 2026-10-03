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
    use std::io::Write;
    for attempt in 0..CLAIM_ATTEMPTS {
        // Concurrent stagers are threads as well as processes, so the pid alone is not
        // unique; `create_new` plus a counter settles both.
        let path = dir.join(format!(".staging-{}-{attempt}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
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

// ── plumbing ─────────────────────────────────────────────────────────

fn render_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string()) + "\n"
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
/// session reported as absent. Writing beside the record and renaming over it means the
/// name never points at a partial file.
pub(crate) fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let parent = parent_dir(path)?;
    let staged = stage(parent, &render_json(value))?;
    std::fs::rename(&staged, path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("cannot write {}: {e}", path.display())
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
    let staged = stage(parent, &render_json(value)).map_err(CreateError::Failed)?;
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
