use crate::board::{Server, settings_now};
use crate::kernel::config::Settings;
use crate::lifecycle::hub::{HubStart, TabOutcome, start_hub};
use crate::mail::RepoHub;
use crate::registry::Context;

/// Where a hub started by its key is listed: its `hubs[].id` and the slug of its board.
pub struct HubAt {
    pub id: String,
    pub slug: String,
}

/// The context that starts `hub`: addressed by its key, refused when a parent hub's key is not
/// known (it could only be started as some other hub).
pub(super) fn start_context(
    server: &Server,
    hub: &RepoHub,
    settings: Settings,
) -> Result<Context, String> {
    if hub.parent && hub.key.is_none() {
        return Err(
            "the key of this hub is not known; start it with adj hub --hub <key>".to_string(),
        );
    }
    Ok(Context {
        repo: server.ctx.repo.clone().addressed(hub.key.as_deref())?,
        resolved: server.ctx.resolved.clone(),
        state: server.ctx.state.clone(),
        settings,
    })
}

/// Start `hub` from the board, as `adj hub` does, `start` saying whether on a new conversation,
/// the saved one, or whichever `hubAutoResumeHours` picks.
pub fn start(server: &Server, hub: &RepoHub, start: HubStart) -> Result<TabOutcome, String> {
    let ctx = start_context(server, hub, settings_now(server))?;
    start_hub(&ctx, start)
}

// ── start a parent-task hub ──────────────────────────────────────────

/// Start the hub for the parent-task `key`, as `adj hub --hub KEY` does, and say where it is
/// listed. The other way to start a parent-task hub is by the id in `hubs[]`, which only
/// exists once something points at it.
pub fn start_for_key(
    server: &Server,
    key: &str,
    start: HubStart,
) -> Result<(HubAt, TabOutcome), String> {
    let settings = settings_now(server);
    let ctx = Context {
        repo: server.ctx.repo.clone().addressed(Some(key))?,
        resolved: server.ctx.resolved.clone(),
        state: server.ctx.state.clone(),
        settings,
    };
    let at = HubAt {
        id: format!("hub-{key}"),
        slug: ctx.repo.slug.clone(),
    };
    Ok((at, start_hub(&ctx, start)?))
}
