use std::path::Path;

use crate::kernel::config::{Settings, resolve_config};
use crate::kernel::identity::AIDE_CONFIG_KEY;
use crate::lifecycle::hub::hub_startable;
use crate::registry::{self, AideRecord, Recorded};

use super::{AideOpened, AideRefused, open_aide, stop_aide};

/// What a reset came to.
#[derive(Debug, PartialEq)]
pub struct AideReset {
    /// A process was stopped.
    pub was_running: bool,
    /// The aide that is there now.
    pub opened: AideOpened,
}

/// Why the aide was not reset.
#[derive(Debug, PartialEq)]
pub enum ResetRefused {
    /// The terminal is not tmux: nothing was stopped, because it could not be started again.
    NotStartable,
    /// The old one could not be stopped, or its record not read, before anything was stopped:
    /// nothing was changed.
    StopFailed(String),
    /// Something failed after a process may have been stopped: a new one could not be started, the
    /// old record not cleared, or a later stop failed. `stopped` says whether one was stopped.
    StartFailed { stopped: bool, why: String },
}

/// Replace the aide with a fresh one: stop the one that runs, then open another. A turn in
/// progress is cut short on purpose: the person pressed reset, so that is what they asked for,
/// and a reset is also the way out of one that is stuck. A fresh aide loses nothing it cannot
/// fetch again, which is why it is started anew and not resumed.
pub fn reset_aide(root: &Path) -> Result<AideReset, ResetRefused> {
    let settings = resolve_config(AIDE_CONFIG_KEY)
        .map_err(ResetRefused::StopFailed)?
        .settings;
    reset_aide_with(
        root,
        &settings,
        &|record| stop_aide(&settings, record),
        &|| open_aide(root),
    )
}

/// `reset_aide`, with the stop and the open handed in.
pub(crate) fn reset_aide_with(
    root: &Path,
    settings: &Settings,
    stop: &dyn Fn(&AideRecord) -> Result<bool, String>,
    open: &dyn Fn() -> Result<AideOpened, AideRefused>,
) -> Result<AideReset, ResetRefused> {
    // First, so a reset that could not start again does not take the old one down.
    if !hub_startable(&settings.terminal) {
        return Err(ResetRefused::NotStartable);
    }
    let mut was_running = false;
    match registry::aide_record(root) {
        Recorded::Unreadable => {
            return Err(ResetRefused::StopFailed(
                "the aide record cannot be read; remove it to start a new one".to_string(),
            ));
        }
        Recorded::Absent => {}
        Recorded::Found(first) => {
            // Whatever the old one was, it is not the one to keep: a launch still on its way
            // (no process recorded yet) is superseded too, and finds this out when it reports.
            // But the record is only cleared if it is still what was stopped: a launch that
            // recorded its process meanwhile is running, and is stopped in its turn.
            let mut record = first;
            let mut tries = 0;
            loop {
                // A stop that fails after an earlier round stopped a process is not "nothing
                // changed".
                let stopped = stop(&record).map_err(|why| {
                    if was_running {
                        ResetRefused::StartFailed { stopped: true, why }
                    } else {
                        ResetRefused::StopFailed(why)
                    }
                })?;
                was_running |= stopped;
                let cleared = registry::clear_aide_unchanged(root, &record.session_id, record.pid)
                    .map_err(|e| ResetRefused::StartFailed {
                        stopped: was_running,
                        why: format!("the old record could not be cleared: {e}"),
                    })?;
                if cleared {
                    break;
                }
                tries += 1;
                match registry::aide_record(root) {
                    Recorded::Found(now) if tries < 3 => record = now,
                    Recorded::Absent => break,
                    _ => {
                        let why = "the aide kept changing while it was being stopped".to_string();
                        return Err(if was_running {
                            ResetRefused::StartFailed { stopped: true, why }
                        } else {
                            ResetRefused::StopFailed(why)
                        });
                    }
                }
            }
        }
    }
    // If a concurrent open wins between the stop and here, that one is the fresh aide.
    match open() {
        Ok(opened) => Ok(AideReset {
            was_running,
            opened,
        }),
        // Checked at the top already: the terminal changed since, and the old one may be gone.
        Err(AideRefused::NotStartable) => Err(ResetRefused::StartFailed {
            stopped: was_running,
            why: "the terminal is no longer tmux".to_string(),
        }),
        Err(AideRefused::Failed(why)) => Err(ResetRefused::StartFailed {
            stopped: was_running,
            why,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn settings(preset: &str) -> (crate::testing::Sandbox, Settings) {
        let sandbox = crate::testing::Sandbox::new(&format!(
            r#"{{"defaults":{{"terminal":{{"preset":"{preset}"}}}}}}"#
        ));
        let settings = resolve_config("@aide").unwrap().settings;
        (sandbox, settings)
    }

    fn put(root: &Path, session: &str) {
        std::fs::write(
            root.join("aide.json"),
            format!(r#"{{"sessionId":"{session}","launchedAt":1}}"#),
        )
        .unwrap();
    }

    fn record(session: &str) -> AideRecord {
        AideRecord {
            session_id: session.to_string(),
            launched_at: 1,
            pid: None,
            ps_started: None,
            terminal: None,
            other: serde_json::Map::new(),
        }
    }

    fn opened(session: &str) -> AideOpened {
        AideOpened::Starting(record(session))
    }

    #[test]
    fn a_terminal_that_cannot_start_one_stops_nothing() {
        let (_sandbox, settings) = settings("iterm2");
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "old");
        let out = reset_aide_with(dir.path(), &settings, &|_| panic!("stopped"), &|| {
            panic!("opened")
        });
        assert_eq!(out, Err(ResetRefused::NotStartable));
    }

    #[test]
    fn a_stop_that_fails_leaves_the_record_and_starts_nothing() {
        let (_sandbox, settings) = settings("tmux");
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "old");
        let out = reset_aide_with(
            dir.path(),
            &settings,
            &|_| Err("stuck".to_string()),
            &|| panic!("opened"),
        );
        assert_eq!(out, Err(ResetRefused::StopFailed("stuck".to_string())));
        assert!(dir.path().join("aide.json").exists());
    }

    #[test]
    fn a_stop_then_an_open_gives_a_new_session() {
        let (_sandbox, settings) = settings("tmux");
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "old");
        let stopped = Cell::new(false);
        let out = reset_aide_with(
            dir.path(),
            &settings,
            &|r| {
                assert_eq!(r.session_id, "old");
                stopped.set(true);
                Ok(true)
            },
            &|| {
                assert!(stopped.get());
                Ok(opened("new"))
            },
        )
        .unwrap();
        assert!(out.was_running);
        assert_eq!(out.opened, opened("new"));
        assert!(!dir.path().join("aide.json").exists());
    }

    #[test]
    fn a_launch_that_records_its_process_during_the_stop_is_stopped_too() {
        let (_sandbox, settings) = settings("tmux");
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "old");
        let stops = std::cell::RefCell::new(Vec::new());
        let out = reset_aide_with(
            dir.path(),
            &settings,
            &|r| {
                stops.borrow_mut().push(r.pid);
                if r.pid.is_none() {
                    // The launch reports its process while the first stop returns.
                    std::fs::write(
                        dir.path().join("aide.json"),
                        r#"{"sessionId":"old","launchedAt":1,"pid":42}"#,
                    )
                    .unwrap();
                    return Ok(false);
                }
                Ok(true)
            },
            &|| Ok(opened("new")),
        )
        .unwrap();
        assert_eq!(*stops.borrow(), vec![None, Some(42)]);
        assert!(out.was_running);
        assert!(!dir.path().join("aide.json").exists());
    }

    #[test]
    fn a_start_that_fails_after_the_stop_says_so() {
        let (_sandbox, settings) = settings("tmux");
        let dir = tempfile::tempdir().unwrap();
        let out = reset_aide_with(
            dir.path(),
            &settings,
            &|_| panic!("nothing to stop"),
            &|| Err(AideRefused::Failed("no window".to_string())),
        );
        assert_eq!(
            out,
            Err(ResetRefused::StartFailed {
                stopped: false,
                why: "no window".to_string()
            })
        );
    }
}
