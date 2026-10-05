use super::start::hub_startable;
use crate::infra::terminal;
use crate::lifecycle::{GONE_BUDGET, GONE_POLL, settled};
use crate::registry::{self, Context};

/// Stop the hub `ctx` addresses by closing the tmux pane it runs in. `Ok(true)` when a hub
/// was running and is gone, `Ok(false)` when there was none (a record it left is cleared).
///
/// Everything is about **one** hub, read out of the record once: the pane that is closed,
/// the process that has to be gone afterwards, and the record that may then be cleared. Asked
/// separately, each could be answered about a different hub — one that started in the
/// meantime — and the record cleared would be its.
pub fn stop_hub(ctx: &Context) -> Result<bool, String> {
    let slug = &ctx.repo.slug;
    let root = &ctx.state;
    let read = registry::read_hub_record(root, slug);
    let record = match &read {
        registry::Recorded::Found(r) => Some(r),
        _ => None,
    };
    // The start time as recorded, not `recorded_anchor`: a blank one is refused below by
    // name rather than read as no anchor.
    let named = record.and_then(|r| Some((r.pid? as u32, r.ps_started.clone())));
    let Some((pid, started)) = named else {
        // No record, or none that names a process: no hub, and nothing that is safe to clear.
        return match registry::hub_liveness(root, slug) {
            registry::Liveness::CannotTell => Err(registry::hub_cannot_tell(root, slug)),
            _ => Ok(false),
        };
    };
    // Before anything is looked up or closed: the start time is what tells this hub from
    // whatever inherited its pid, and closing a pane on the strength of the pid alone could
    // close somebody else's.
    if started.as_deref().is_none_or(|s| s.trim().is_empty()) {
        return Err("the hub record carries no start time, so the process cannot be told apart from a reused pid; stop it where it runs".to_string());
    }
    match registry::hub_process_liveness(pid, started.as_deref()) {
        registry::Liveness::Gone => {
            registry::unregister_hub_if(root, slug, pid, started.as_deref())?;
            return Ok(false);
        }
        registry::Liveness::CannotTell => return Err(registry::hub_cannot_tell(root, slug)),
        registry::Liveness::Alive => {}
    }
    let terminal_recorded = record.and_then(|r| r.terminal.clone());
    // Only a hub known to sit in tmux is looked for there: asking tmux about a hub that runs
    // anywhere else would start by talking to whichever server is the default.
    let in_tmux = match &terminal_recorded {
        Some(t) => t.backend == "tmux",
        None => hub_startable(&ctx.settings.terminal),
    };
    let socket = terminal_recorded
        .as_ref()
        .and_then(|t| t.socket.clone())
        .or_else(|| ctx.settings.terminal.tmux_socket().map(str::to_string));
    if !in_tmux {
        return Err(format!(
            "the hub is not in a tmux pane on socket {}; stop it where it runs",
            socket.as_deref().unwrap_or("default")
        ));
    }
    let panes = terminal::list_tmux_panes(socket.as_deref())?;
    let pane = terminal::find_matching_pane(&panes, Some(pid), terminal::tty_of(pid).as_deref())
        .ok_or_else(|| {
            format!(
                "the hub is not in a tmux pane on socket {}; stop it where it runs",
                socket.as_deref().unwrap_or("default")
            )
        })?;
    crate::infra::shell::run_shell(&terminal::tmux_kill_pane_script(
        socket.as_deref(),
        &pane.pane_id,
    ))?;
    match settled(
        || registry::hub_process_liveness(pid, started.as_deref()),
        std::thread::sleep,
        GONE_BUDGET,
        GONE_POLL,
    ) {
        registry::Liveness::Gone => {
            registry::unregister_hub_if(root, slug, pid, started.as_deref())?;
            Ok(true)
        }
        _ => Err(format!(
            "closed the pane, but the hub (pid {pid}) is still running"
        )),
    }
}
