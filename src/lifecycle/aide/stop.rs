use std::time::Duration;

use crate::infra::terminal;
use crate::kernel::config::Settings;
use crate::lifecycle::hub::hub_startable;
use crate::lifecycle::{GONE_BUDGET, GONE_POLL, settled};
use crate::registry::{self, AideRecord, Liveness};

/// What closing a pane answers.
type Closed = Result<(), String>;

/// Stop the aide by closing the tmux pane it runs in, the way a reader is stopped
/// (`stop_reader`). `Ok(true)` when a process was stopped and is gone, `Ok(false)` when there was
/// none to stop (no process recorded, or it has ended). `Err` when it cannot be done: the process
/// cannot be told alive or dead, it recorded no start time (so it cannot be told from a reused
/// pid), it is not in a tmux pane, or it is still running after the pane closed. Nothing is
/// changed then.
///
/// This is the third copy of the reader's and the hub's stop. Folding the three into one function
/// is left to a follow-up of its own rather than done inside this change.
pub fn stop_aide(settings: &Settings, record: &AideRecord) -> Result<bool, String> {
    stop_aide_with(
        settings,
        record,
        &registry::hub_process_liveness,
        &|socket, pane| {
            crate::infra::shell::run_shell(&terminal::tmux_kill_pane_script(socket, pane))
                .map(|_| ())
        },
        &std::thread::sleep,
    )
}

/// `stop_aide`, with the process check, the closing of a pane and the waiting handed in.
pub(crate) fn stop_aide_with(
    settings: &Settings,
    record: &AideRecord,
    liveness: &dyn Fn(u32, Option<&str>) -> Liveness,
    close_pane: &dyn Fn(Option<&str>, &str) -> Closed,
    wait: &dyn Fn(Duration),
) -> Result<bool, String> {
    let Some(pid) = record.pid else {
        return Ok(false);
    };
    let started = record.ps_started.as_deref();
    match liveness(pid, started) {
        Liveness::Gone => return Ok(false),
        Liveness::CannotTell => {
            return Err(format!(
                "cannot tell whether the aide (pid {pid}) is still running"
            ));
        }
        Liveness::Alive => {}
    }
    // Before anything is looked up or closed: the start time is what tells this process from
    // whatever inherited its pid, and closing a pane on the pid alone could close somebody else's.
    if started.is_none_or(|s| s.trim().is_empty()) {
        return Err("the aide recorded no start time, so its process cannot be told apart from a reused pid; stop it where it runs".to_string());
    }
    let recorded = record.terminal.as_ref();
    let in_tmux = match recorded {
        Some(t) => t.backend == "tmux",
        None => hub_startable(&settings.terminal),
    };
    let socket = recorded
        .and_then(|t| t.socket.clone())
        .or_else(|| settings.terminal.tmux_socket().map(str::to_string));
    let elsewhere = || {
        format!(
            "the aide is not in a tmux pane on socket {}; stop it where it runs",
            socket.as_deref().unwrap_or("default")
        )
    };
    if !in_tmux {
        return Err(elsewhere());
    }
    let pane = match recorded.and_then(|t| t.pane.clone()) {
        Some(pane) => pane,
        None => {
            let panes = terminal::list_tmux_panes(socket.as_deref())?;
            terminal::find_matching_pane(&panes, Some(pid), terminal::tty_of(pid).as_deref())
                .ok_or_else(elsewhere)?
                .pane_id
                .clone()
        }
    };
    close_pane(socket.as_deref(), &pane)?;
    match settled(|| liveness(pid, started), wait, GONE_BUDGET, GONE_POLL) {
        Liveness::Gone => Ok(true),
        _ => Err(format!(
            "closed the pane, but the aide (pid {pid}) is still running"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::terminal::SessionTerminal;
    use serde_json::Map;
    use std::cell::Cell;

    fn record(pid: Option<u32>, started: Option<&str>, pane: Option<&str>) -> AideRecord {
        AideRecord {
            session_id: "s".to_string(),
            launched_at: 1,
            pid,
            ps_started: started.map(str::to_string),
            terminal: Some(SessionTerminal {
                backend: "tmux".to_string(),
                socket: None,
                session: None,
                window: Some("@1".to_string()),
                pane: pane.map(str::to_string),
            }),
            other: Map::new(),
        }
    }

    fn never(_: Option<&str>, _: &str) -> Result<(), String> {
        panic!("no pane may be closed")
    }

    fn stop(r: &AideRecord, l: &dyn Fn(u32, Option<&str>) -> Liveness) -> Result<bool, String> {
        stop_aide_with(&Settings::default(), r, l, &never, &|_| {})
    }

    #[test]
    fn no_process_or_a_gone_one_is_nothing_to_stop() {
        assert_eq!(stop(&record(None, None, None), &|_, _| panic!()), Ok(false));
        let gone = record(Some(5), Some("t"), Some("%1"));
        assert_eq!(stop(&gone, &|_, _| Liveness::Gone), Ok(false));
    }

    #[test]
    fn a_process_that_cannot_be_told_or_has_no_start_time_is_refused() {
        let r = record(Some(5), Some("t"), Some("%1"));
        assert!(stop(&r, &|_, _| Liveness::CannotTell).is_err());
        let r = record(Some(5), None, Some("%1"));
        assert!(
            stop(&r, &|_, _| Liveness::Alive)
                .unwrap_err()
                .contains("no start time")
        );
    }

    #[test]
    fn closing_the_pane_ends_it_or_the_stop_says_it_did_not() {
        let r = record(Some(5), Some("t"), Some("%1"));
        let closed = Cell::new(false);
        let close = |_: Option<&str>, pane: &str| {
            assert_eq!(pane, "%1");
            closed.set(true);
            Ok(())
        };
        let after_kill = |_: u32, _: Option<&str>| {
            if closed.get() {
                Liveness::Gone
            } else {
                Liveness::Alive
            }
        };
        let stopped = stop_aide_with(&Settings::default(), &r, &after_kill, &close, &|_| {});
        assert_eq!(stopped, Ok(true));
        let still = stop_aide_with(
            &Settings::default(),
            &r,
            &|_, _| Liveness::Alive,
            &|_, _| Ok(()),
            &|_| {},
        );
        assert!(still.unwrap_err().contains("still running"));
    }
}
