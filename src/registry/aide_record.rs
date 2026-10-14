//! Reading the aide's record.

use super::store::{aide_record_path, read_record};
use super::*;

/// The aide's record: absent when none was ever written (or it was cleared), and
/// unreadable, which is not the same news, when the file is there and is not one.
pub fn aide_record(root: &Path) -> Recorded<AideRecord> {
    read_record(&aide_record_path(root), |value| {
        serde_json::from_value(value).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_unreadable_and_found_are_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(aide_record(dir.path()), Recorded::Absent);
        std::fs::write(aide_record_path(dir.path()), "not json").unwrap();
        assert_eq!(aide_record(dir.path()), Recorded::Unreadable);
        std::fs::write(
            aide_record_path(dir.path()),
            r#"{"sessionId":"s1","launchedAt":5,"future":1}"#,
        )
        .unwrap();
        let Recorded::Found(record) = aide_record(dir.path()) else {
            panic!("expected a record");
        };
        assert_eq!(record.session_id, "s1");
        assert_eq!(record.launched_at, 5);
        assert_eq!(record.other["future"], 1);
    }
}
