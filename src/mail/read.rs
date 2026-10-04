use super::*;

pub fn read(slug: &str, name: &str) -> Result<String, String> {
    let path = safe_join(&inbox_dir(slug), name)?;
    std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}
