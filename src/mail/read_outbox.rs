use super::*;

pub fn read_outbox(worktree: &Path) -> String {
    std::fs::read_to_string(outbox_path(worktree)).unwrap_or_default()
}
