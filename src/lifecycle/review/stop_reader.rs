use crate::infra::terminal;
use crate::lifecycle::hub::hub_startable;
use crate::lifecycle::{GONE_BUDGET, GONE_POLL, settled};
use crate::others::HungReader;
use crate::registry::{self, Context, Liveness};

/// Stop a reader that has run for too long by closing the tmux pane it runs in, the way a hub is
/// stopped (`stop_hub`): the pane is found by the window the reader recorded, else by its process,
/// closed, and the process must then be gone. `Err` when it cannot be done (the reader is not in
/// a tmux pane, its start time was not recorded so it cannot be told from a reused pid, or it is
/// still running after the pane closed), and then nothing may be taken over.
pub fn stop_reader(ctx: &Context, reader: &HungReader) -> Result<(), String> {
    let Some(pid) = reader.pid else {
        return Ok(());
    };
    let started = reader.ps_started.as_deref();
    // Before anything is looked up or closed: the start time is what tells this reader from
    // whatever inherited its pid, and closing a pane on the pid alone could close somebody else's.
    if started.is_none_or(|s| s.trim().is_empty()) {
        return Err("the reader recorded no start time, so its process cannot be told apart from a reused pid; stop it where it runs".to_string());
    }
    let recorded = reader.terminal.as_ref();
    let in_tmux = match recorded {
        Some(t) => t.backend == "tmux",
        None => hub_startable(&ctx.settings.terminal),
    };
    let socket = recorded
        .and_then(|t| t.socket.clone())
        .or_else(|| ctx.settings.terminal.tmux_socket().map(str::to_string));
    let elsewhere = || {
        format!(
            "the reader is not in a tmux pane on socket {}; stop it where it runs",
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
    crate::infra::shell::run_shell(&terminal::tmux_kill_pane_script(socket.as_deref(), &pane))?;
    match settled(
        || registry::hub_process_liveness(pid, started),
        std::thread::sleep,
        GONE_BUDGET,
        GONE_POLL,
    ) {
        Liveness::Gone => Ok(()),
        _ => Err(format!(
            "closed the pane, but the reader (pid {pid}) is still running"
        )),
    }
}
