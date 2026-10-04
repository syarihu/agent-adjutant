use super::*;

pub fn pending(root: &Path, slug: &str) -> Pending {
    Pending {
        dir: inbox_dir(root, slug),
        messages: list(root, slug),
    }
}

/// The inbox directory, created: whoever asks for the path is about to write into it.
pub fn open_inbox(root: &Path, slug: &str) -> Result<PathBuf, String> {
    let dir = inbox_dir(root, slug);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}
