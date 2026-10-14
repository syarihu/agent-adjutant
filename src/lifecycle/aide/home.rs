use std::path::{Path, PathBuf};

/// The aide's working directory, `<state>/aide`, next to the code-review hub's `review`. It is
/// machine-wide and belongs to no checkout. It stays the same across resets on purpose: Claude
/// Code asks once per directory whether to trust it, and a new directory would ask every time.
pub fn aide_home(root: &Path) -> PathBuf {
    root.join("aide")
}
