//! The tmux window a PR's reader runs in, for the board's terminal on it.
//!
//! What the page names is a PR; the socket and the window come from the record the reader wrote
//! about itself (`run_reader` records where it runs), never from the request.

use std::path::Path;

use crate::board::is_window_id;
use crate::registry::{self, Liveness};
use crate::task::PrRef;

/// Why there is no window to open on a PR's reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoReaderWindow {
    /// adj has no record of this PR.
    NoRecord,
    /// No reader is working on it, or it did not run in tmux, or it recorded no window (it was
    /// not in tmux, or the record is from before windows were kept).
    NoReader,
    /// The reader's process is gone: what the record names is no longer there to attach to.
    Gone,
    /// The record could not be read, and why. Not the same as having none.
    Unreadable(String),
}

/// The socket and window of the reader of `pr`.
pub fn reader_window(root: &Path, pr: &PrRef) -> Result<(Option<String>, String), NoReaderWindow> {
    reader_window_with(root, pr, &registry::hub_process_liveness)
}

/// `reader_window`, with the way a process is asked after handed in.
pub(crate) fn reader_window_with(
    root: &Path,
    pr: &PrRef,
    liveness: &dyn Fn(u32, Option<&str>) -> Liveness,
) -> Result<(Option<String>, String), NoReaderWindow> {
    let record = crate::others::record(root, pr)
        .map_err(NoReaderWindow::Unreadable)?
        .ok_or(NoReaderWindow::NoRecord)?;
    // A `queued` or `failed` read has no live reader, whatever an older reader left in `terminal`.
    let ai = record
        .ai
        .filter(|ai| matches!(ai.status.as_str(), "running" | "ready"))
        // A closed reader's window is gone, whatever `terminal` still names.
        .filter(|ai| ai.reader_closed_at.is_none())
        .ok_or(NoReaderWindow::NoReader)?;
    let terminal = ai.terminal.ok_or(NoReaderWindow::NoReader)?;
    let window = terminal
        .window
        .filter(|w| terminal.backend == "tmux" && is_window_id(w))
        .ok_or(NoReaderWindow::NoReader)?;
    if let Some(pid) = ai.pid {
        // A process that cannot be asked after is let through: attaching to a window that is gone
        // fails cleanly, and refusing would hide a reader that is there.
        if liveness(pid, ai.ps_started.as_deref()) == Liveness::Gone {
            return Err(NoReaderWindow::Gone);
        }
    }
    Ok((terminal.socket, window))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::pr_ref;
    use serde_json::{Value, json};

    fn pr() -> PrRef {
        pr_ref("https://github.com/acme/widget/pull/7", None).unwrap()
    }

    fn put(root: &Path, ai: Option<Value>) {
        let mut record = json!({
            "id": "acme/widget#7", "repo": "Acme/Widget", "number": 7,
            "url": "https://github.com/acme/widget/pull/7",
            "registered": true, "title": "t", "base": "main", "head": "f", "headSha": "h1",
            "prState": "open", "additions": 1, "deletions": 1, "changedFiles": 1,
            "requested": true, "requestedAt": "20261001T000000Z",
            "firstSeenAt": "20261001T000000Z", "readAt": "20261001T000000Z"
        });
        if let Some(ai) = ai {
            record["ai"] = ai;
        }
        let dir = root.join("others");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("acme~widget~7.json"), record.to_string()).unwrap();
    }

    fn reader(status: &str, terminal: Value) -> Value {
        json!({"status": status, "startedAt": "20261001T000001Z", "pid": 4242,
               "psStarted": "Thu Oct  1 00:00:02 2026", "terminal": terminal})
    }

    fn tmux(window: &str) -> Value {
        json!({"backend": "tmux", "socket": "/tmp/tmux-1/default", "window": window})
    }

    fn alive(_: u32, _: Option<&str>) -> Liveness {
        Liveness::Alive
    }

    fn window(root: &Path) -> Result<(Option<String>, String), NoReaderWindow> {
        reader_window_with(root, &pr(), &alive)
    }

    #[test]
    fn a_pr_with_no_record_has_no_window() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(window(root.path()), Err(NoReaderWindow::NoRecord));
    }

    #[test]
    fn a_record_that_cannot_be_read_is_not_a_missing_one() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("others");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("acme~widget~7.json"), "{not json").unwrap();
        assert!(matches!(
            window(root.path()),
            Err(NoReaderWindow::Unreadable(_))
        ));
    }

    #[test]
    fn only_a_reader_that_is_working_or_has_handed_in_has_a_window() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), None);
        assert_eq!(window(root.path()), Err(NoReaderWindow::NoReader));
        for status in ["queued", "failed"] {
            put(root.path(), Some(reader(status, tmux("@3"))));
            assert_eq!(
                window(root.path()),
                Err(NoReaderWindow::NoReader),
                "{status}"
            );
        }
        for status in ["running", "ready"] {
            put(root.path(), Some(reader(status, tmux("@3"))));
            assert_eq!(
                window(root.path()),
                Ok((Some("/tmp/tmux-1/default".to_string()), "@3".to_string())),
                "{status}"
            );
        }
    }

    #[test]
    fn a_closed_reader_has_no_window() {
        let root = tempfile::tempdir().unwrap();
        for status in ["running", "ready"] {
            let mut ai = reader(status, tmux("@3"));
            ai["readerClosedAt"] = json!("20261001T000009Z");
            put(root.path(), Some(ai));
            assert_eq!(
                window(root.path()),
                Err(NoReaderWindow::NoReader),
                "{status}"
            );
        }
    }

    #[test]
    fn a_terminal_that_is_not_tmux_or_has_no_usable_window_is_no_reader() {
        let root = tempfile::tempdir().unwrap();
        let cases = [
            json!({"backend": "iterm2", "window": "@3"}),
            json!({"backend": "tmux"}),
            json!({"backend": "tmux", "window": "3"}),
            json!({"backend": "tmux", "window": "@"}),
            json!({"backend": "tmux", "window": "@3; rm -rf /"}),
        ];
        for terminal in cases {
            put(root.path(), Some(reader("running", terminal.clone())));
            assert_eq!(
                window(root.path()),
                Err(NoReaderWindow::NoReader),
                "{terminal}"
            );
        }
        // No terminal at all, as a reader written before windows were kept.
        let mut ai = reader("running", Value::Null);
        ai.as_object_mut().unwrap().remove("terminal");
        put(root.path(), Some(ai));
        assert_eq!(window(root.path()), Err(NoReaderWindow::NoReader));
    }

    #[test]
    fn a_reader_whose_process_is_gone_is_refused_and_one_that_cannot_be_told_is_not() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), Some(reader("running", tmux("@3"))));
        let asked = std::cell::RefCell::new(Vec::new());
        let gone = |pid: u32, started: Option<&str>| {
            asked.borrow_mut().push((pid, started.map(str::to_string)));
            Liveness::Gone
        };
        assert_eq!(
            reader_window_with(root.path(), &pr(), &gone),
            Err(NoReaderWindow::Gone)
        );
        // Asked about the process the record names, with the start time it recorded.
        assert_eq!(
            *asked.borrow(),
            vec![(4242, Some("Thu Oct  1 00:00:02 2026".to_string()))]
        );
        let unsure = |_: u32, _: Option<&str>| Liveness::CannotTell;
        assert_eq!(
            reader_window_with(root.path(), &pr(), &unsure),
            Ok((Some("/tmp/tmux-1/default".to_string()), "@3".to_string()))
        );
    }

    #[test]
    fn a_reader_with_no_recorded_process_is_not_asked_after() {
        let root = tempfile::tempdir().unwrap();
        let mut ai = reader("ready", tmux("@3"));
        ai.as_object_mut().unwrap().remove("pid");
        put(root.path(), Some(ai));
        let never = |_: u32, _: Option<&str>| -> Liveness { panic!("no process to ask after") };
        assert_eq!(
            reader_window_with(root.path(), &pr(), &never),
            Ok((Some("/tmp/tmux-1/default".to_string()), "@3".to_string()))
        );
    }
}
