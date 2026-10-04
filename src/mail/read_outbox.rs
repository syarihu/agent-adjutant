use super::*;

pub fn read_outbox(worktree: &Path) -> Outbox {
    let path = outbox_path(worktree);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    Outbox { path, text }
}
