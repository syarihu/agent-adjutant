use super::*;

/// Where the hub leaves messages for the worker. One markdown file rather than one file per
/// message, because the worker reads it with its eyes as often as with a tool.
pub fn outbox_path(worktree: &Path) -> PathBuf {
    worktree.join(".claude").join("adjutant-outbox.md")
}

/// Append one entry to the worker's outbox.
///
/// The shape is fixed here rather than described in the hub's procedure: a format spelled
/// out in prose is a format that drifts, and the worker reads the `##` heading to know who
/// is speaking and the first line to know whether it is being asked something.
pub fn tell(worktree: &Path, from: &str, subject: &str, body: &str) -> Result<PathBuf, String> {
    let path = outbox_path(worktree);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let entry = format!(
        "## {} from {}\n\n{}\n{}\n\n",
        utc_stamp(now_secs()),
        one_line(from),
        one_line(subject),
        body.trim_end()
    );
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.write_all(entry.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

pub fn read_outbox(worktree: &Path) -> String {
    std::fs::read_to_string(outbox_path(worktree)).unwrap_or_default()
}

/// Everything in the outbox has been dealt with. Removed rather than emptied so that
/// "is there anything for me" is answered by the file existing at all.
pub fn clear_outbox(worktree: &Path) -> Result<(), String> {
    remove_if_present(&outbox_path(worktree))
}
