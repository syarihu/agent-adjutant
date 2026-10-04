use super::*;

/// Everything in the outbox has been dealt with. Removed rather than emptied so that
/// "is there anything for me" is answered by the file existing at all.
pub fn clear_outbox(worktree: &Path) -> Result<(), String> {
    remove_if_present(&outbox_path(worktree))
}
