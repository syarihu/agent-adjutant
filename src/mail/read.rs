use super::*;

pub fn read(root: &Path, slug: &str, name: &str) -> Result<String, String> {
    let path = safe_join(&inbox_dir(root, slug), name)?;
    std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}
