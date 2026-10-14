//! The tmux window the aide runs in, for the board's terminal on it.
//!
//! The socket and the window come from the record the aide wrote about itself (`run_aide`
//! records where it runs), never from the request.

use std::path::Path;

use crate::board::is_window_id;
use crate::registry::{self, Liveness, Recorded};

/// Why there is no window to open on the aide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoAideWindow {
    /// No aide was ever started, or it was cleared.
    NoAide,
    /// It is not in tmux, or has not recorded a window yet.
    NoWindow,
    /// Its process is gone.
    Gone,
    /// The page asked for a session that is not the current one: the aide was reset.
    Replaced,
    /// The record could not be read, and why.
    Unreadable(String),
}

/// The socket and window of the aide. `session`, when given, is the session the page knows: a
/// different one means the page is looking at an aide that has since been replaced.
pub fn window(
    root: &Path,
    session: Option<&str>,
) -> Result<(Option<String>, String), NoAideWindow> {
    window_with(root, session, &registry::hub_process_liveness)
}

/// `window`, with the way a process is asked after handed in.
pub(crate) fn window_with(
    root: &Path,
    session: Option<&str>,
    liveness: &dyn Fn(u32, Option<&str>) -> Liveness,
) -> Result<(Option<String>, String), NoAideWindow> {
    let record = match registry::aide_record(root) {
        Recorded::Absent => return Err(NoAideWindow::NoAide),
        Recorded::Unreadable => {
            return Err(NoAideWindow::Unreadable(
                "the aide record cannot be read".to_string(),
            ));
        }
        Recorded::Found(record) => record,
    };
    if session.is_some_and(|session| session != record.session_id) {
        return Err(NoAideWindow::Replaced);
    }
    let terminal = record.terminal.ok_or(NoAideWindow::NoWindow)?;
    let window = terminal
        .window
        .filter(|w| terminal.backend == "tmux" && is_window_id(w))
        .ok_or(NoAideWindow::NoWindow)?;
    if let Some(pid) = record.pid {
        // A process that cannot be asked after is let through: attaching to a window that is gone
        // fails cleanly, and refusing would hide an aide that is there.
        if liveness(pid, record.ps_started.as_deref()) == Liveness::Gone {
            return Err(NoAideWindow::Gone);
        }
    }
    Ok((terminal.socket, window))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, terminal: &str) {
        std::fs::write(
            root.join("aide.json"),
            format!(r#"{{"sessionId":"s1","launchedAt":1,"pid":9,"psStarted":"t",{terminal}}}"#),
        )
        .unwrap();
    }

    fn alive(_: u32, _: Option<&str>) -> Liveness {
        Liveness::Alive
    }

    #[test]
    fn a_window_is_the_recorded_one_for_the_current_session() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            window_with(dir.path(), None, &alive),
            Err(NoAideWindow::NoAide)
        );
        put(
            dir.path(),
            r#""terminal":{"backend":"tmux","socket":"/tmp/s","window":"@3"}"#,
        );
        assert_eq!(
            window_with(dir.path(), Some("s1"), &alive),
            Ok((Some("/tmp/s".to_string()), "@3".to_string()))
        );
        assert_eq!(
            window_with(dir.path(), Some("other"), &alive),
            Err(NoAideWindow::Replaced)
        );
    }

    #[test]
    fn a_dead_process_a_non_tmux_backend_and_a_bad_window_id_have_no_window() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), r#""terminal":{"backend":"tmux","window":"@3"}"#);
        assert_eq!(
            window_with(dir.path(), None, &|_, _| Liveness::Gone),
            Err(NoAideWindow::Gone)
        );
        assert!(window_with(dir.path(), None, &|_, _| Liveness::CannotTell).is_ok());
        for terminal in [
            r#""terminal":{"backend":"iterm2","window":"@3"}"#,
            r#""terminal":{"backend":"tmux","window":"3; rm"}"#,
            r#""terminal":{"backend":"tmux"}"#,
            r#""x":1"#,
        ] {
            put(dir.path(), terminal);
            assert_eq!(
                window_with(dir.path(), None, &alive),
                Err(NoAideWindow::NoWindow),
                "{terminal}"
            );
        }
        std::fs::write(dir.path().join("aide.json"), "{").unwrap();
        assert!(matches!(
            window_with(dir.path(), None, &alive),
            Err(NoAideWindow::Unreadable(_))
        ));
    }
}
