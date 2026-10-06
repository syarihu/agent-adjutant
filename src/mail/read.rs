use super::*;

/// The body of one waiting message. Reading it is what marks it as looked at: an empty marker
/// is left beside it, and a marker that cannot be written leaves the message unseen rather
/// than failing the read.
pub fn read(root: &Path, slug: &str, name: &str) -> Result<String, String> {
    let dir = inbox_dir(root, slug);
    let path = safe_join(&dir, name)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    // Renamed over whatever marker an earlier message of this name left, so that its time is
    // now and there is no moment with none; the staged file is a dotfile `list` does not offer.
    if let Ok(staged) = stage(&dir, "") {
        let marker = dir.join(format!("{SEEN}{name}"));
        if std::fs::rename(&staged, &marker).is_err() {
            let _ = std::fs::remove_file(&staged);
        }
    }
    Ok(text)
}
