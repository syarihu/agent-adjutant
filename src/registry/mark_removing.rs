//! The marker saying a worktree is being removed.

use super::store::removing_marker_path;
use super::*;

/// How long a "being removed" marker is believed when the process that wrote it cannot be
/// shown to be running. One that can be is believed for as long as it runs.
const REMOVING_SECS: i64 = 300;

/// The marker saying a worktree is being removed, removed again when this goes out of scope,
/// whichever way that happens.
pub struct RemovingMark(PathBuf);

impl Drop for RemovingMark {
    fn drop(&mut self) {
        let _ = remove_if_present(&self.0);
    }
}

/// Say that `worktree` is being removed, so that nothing starts a worker in it meanwhile.
/// Written under the dispatch lock, with the process and the time.
pub fn mark_worktree_removing(main: &Path, worktree: &Path) -> Result<RemovingMark, String> {
    let path = removing_marker_path(main, worktree);
    let pid = std::process::id();
    write_json(
        &path,
        &json!({ "pid": pid, "psStarted": ps_started(pid), "at": now_secs() }),
    )?;
    Ok(RemovingMark(path))
}

/// Whether a removal of `worktree` is under way: the process that wrote the marker is still
/// running, or the marker is recent enough that it may be.
pub fn is_being_removed(main: &Path, worktree: &Path) -> bool {
    let Some(marker) = read_json(&removing_marker_path(main, worktree)) else {
        return false;
    };
    let alive = marker
        .get("pid")
        .and_then(Value::as_u64)
        .zip(marker.get("psStarted").and_then(Value::as_str))
        .is_some_and(|(pid, started)| process_matches(pid as u32, None, Some(started)));
    let recent = marker
        .get("at")
        .and_then(Value::as_i64)
        .is_some_and(|at| (0..REMOVING_SECS).contains(&(now_secs() - at)));
    alive || recent
}
