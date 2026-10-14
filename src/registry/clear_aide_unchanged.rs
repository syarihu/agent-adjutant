//! Clearing the aide's record only if it is still the one that was stopped.

use super::store::{aide_lock_path, aide_record_path};
use super::*;
use crate::infra::fs::lock;

/// Remove the aide's record when it still names `session` with the process `pid` that was
/// stopped (`None`: none was recorded then). `Ok(true)` when it is gone, or was not that session's
/// any more; `Ok(false)` when the same session has recorded another process since, which is
/// still running and has to be stopped before the record can go.
pub fn clear_aide_unchanged(root: &Path, session: &str, pid: Option<u32>) -> Result<bool, String> {
    let _lock = lock(&aide_lock_path(root))?;
    match aide_record(root) {
        Recorded::Found(record) if record.session_id == session => {
            if record.pid != pid {
                return Ok(false);
            }
            remove_if_present(&aide_record_path(root))?;
            Ok(true)
        }
        Recorded::Unreadable => Err("the aide record cannot be read".to_string()),
        _ => Ok(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_clears_only_what_was_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let path = aide_record_path(dir.path());
        std::fs::write(&path, r#"{"sessionId":"s","launchedAt":1}"#).unwrap();
        assert_eq!(clear_aide_unchanged(dir.path(), "s", Some(7)), Ok(false));
        assert!(path.exists());
        assert_eq!(clear_aide_unchanged(dir.path(), "other", None), Ok(true));
        assert!(path.exists());
        assert_eq!(clear_aide_unchanged(dir.path(), "s", None), Ok(true));
        assert!(!path.exists());
        assert_eq!(clear_aide_unchanged(dir.path(), "s", None), Ok(true));
    }
}
