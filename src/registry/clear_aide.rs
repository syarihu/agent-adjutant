//! Taking back a claim whose launch failed.

use super::store::{aide_lock_path, aide_record_path};
use super::*;
use crate::infra::fs::lock;

/// Remove the aide's record, but only when it still names `session`: a launch that failed
/// takes back its own claim and never a newer one. A record that cannot be read is left alone.
pub fn clear_aide_if(root: &Path, session: &str) -> Result<(), String> {
    let _lock = lock(&aide_lock_path(root))?;
    match aide_record(root) {
        Recorded::Found(record) if record.session_id == session => {
            remove_if_present(&aide_record_path(root))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_record_of_that_session_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = aide_record_path(dir.path());
        std::fs::write(&path, r#"{"sessionId":"s2","launchedAt":5}"#).unwrap();
        clear_aide_if(dir.path(), "s1").unwrap();
        assert!(path.exists());
        clear_aide_if(dir.path(), "s2").unwrap();
        assert!(!path.exists());
        // Nothing there is not an error, and a damaged file is not ours to remove.
        clear_aide_if(dir.path(), "s2").unwrap();
        std::fs::write(&path, "{").unwrap();
        clear_aide_if(dir.path(), "s2").unwrap();
        assert!(path.exists());
    }
}
