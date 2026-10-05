use crate::board::{Server, settings_now};
use crate::kernel::config::Settings;
use crate::lifecycle::hub::stop_hub;
use crate::mail::RepoHub;
use crate::registry::Context;

/// The context that stops `hub`. Addressed by the slug the hub was listed under: a hub whose
/// key cannot be told can still be stopped, and nothing here needs the key for it.
pub(super) fn stop_context(server: &Server, hub: &RepoHub, settings: Settings) -> Context {
    let mut stopping = server.ctx.repo.clone();
    stopping.slug = hub.slug.clone();
    stopping.hub_name = hub.name.clone();
    Context {
        repo: stopping,
        resolved: server.ctx.resolved.clone(),
        state: server.ctx.state.clone(),
        settings,
    }
}

/// Stop `hub` from the board: `true` when one was running and is gone, `false` when there was
/// none.
pub fn stop(server: &Server, hub: &RepoHub) -> Result<bool, String> {
    stop_hub(&stop_context(server, hub, settings_now(server)))
}
