//! The aide saying it is there: its process and its window, written down.

use super::store::aide_lock_path;
use super::*;
use crate::infra::fs::lock;
use crate::infra::terminal::SessionTerminal;

/// Record the process and window of the aide that runs as `session`. Refused unless the
/// record still names that session, so a launch that was superseded (by a reset) exits without
/// starting an agent. Keys this version does not know are kept.
pub fn note_aide(
    root: &Path,
    session: &str,
    pid: u32,
    ps_started: Option<String>,
    terminal: Option<SessionTerminal>,
) -> Result<(), String> {
    let _lock = lock(&aide_lock_path(root))?;
    let mut record = match aide_record(root) {
        Recorded::Found(record) => record,
        Recorded::Absent => return Err("no aide was started".to_string()),
        Recorded::Unreadable => {
            return Err("the aide record cannot be read".to_string());
        }
    };
    if record.session_id != session {
        return Err("this aide is no longer the one that was started".to_string());
    }
    record.pid = Some(pid);
    record.ps_started = ps_started;
    record.terminal = terminal;
    write_json(&super::store::aide_record_path(root), &record)
}

#[cfg(test)]
mod tests {
    use super::store::aide_record_path;
    use super::*;

    fn write(root: &Path, text: &str) {
        std::fs::write(aide_record_path(root), text).unwrap();
    }

    #[test]
    fn the_pid_and_window_are_written_and_unknown_keys_survive() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            r#"{"sessionId":"s1","launchedAt":5,"future":"kept"}"#,
        );
        let terminal = SessionTerminal {
            backend: "tmux".to_string(),
            socket: None,
            session: Some("main".to_string()),
            window: Some("@3".to_string()),
            pane: Some("%4".to_string()),
        };
        note_aide(
            dir.path(),
            "s1",
            77,
            Some("started".to_string()),
            Some(terminal.clone()),
        )
        .unwrap();
        let Recorded::Found(record) = aide_record(dir.path()) else {
            panic!("record lost");
        };
        assert_eq!(record.pid, Some(77));
        assert_eq!(record.ps_started.as_deref(), Some("started"));
        assert_eq!(record.terminal, Some(terminal));
        assert_eq!(record.launched_at, 5);
        assert_eq!(record.other["future"], "kept");
    }

    #[test]
    fn another_session_is_refused_and_the_record_is_left_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let text = r#"{"sessionId":"s2","launchedAt":5}"#;
        write(dir.path(), text);
        let err = note_aide(dir.path(), "s1", 77, None, None).unwrap_err();
        assert!(err.contains("no longer"), "{err}");
        assert_eq!(
            std::fs::read_to_string(aide_record_path(dir.path())).unwrap(),
            text
        );
        std::fs::remove_file(aide_record_path(dir.path())).unwrap();
        assert!(note_aide(dir.path(), "s1", 77, None, None).is_err());
    }
}
