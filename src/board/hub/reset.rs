use super::start::start_context;
use super::stop::stop_context;
use crate::board::{Server, settings_now};
use crate::lifecycle::hub::{HubStart, TabOutcome, hub_startable, start_hub, stop_hub};
use crate::mail::RepoHub;

/// What stopping a hub and starting it again came to. `outcome` is `AlreadyRunning` when a hub
/// is up that this call did not start (nothing was running, or another start won the race
/// after the stop): then it is not the conversation that was asked for.
pub struct Reopened {
    pub was_running: bool,
    pub outcome: TabOutcome,
}

/// Stop `hub` and start it again on a new conversation.
pub fn reset(server: &Server, hub: &RepoHub) -> Result<Reopened, String> {
    let settings = settings_now(server);
    // Everything start would refuse is refused before the hub is stopped: a reset that
    // could not start again would only have taken the hub down.
    if !hub_startable(&settings.terminal) {
        return Err("starting a hub from the board needs terminal.preset \"tmux\"".to_string());
    }
    let start_ctx = start_context(server, hub, settings.clone())?;
    let was_running = stop_hub(&stop_context(server, hub, settings))?;
    match start_hub(&start_ctx, HubStart::New) {
        Ok(outcome) => Ok(Reopened {
            was_running,
            outcome,
        }),
        Err(e) if was_running => Err(format!(
            "stopped {}, but could not start it again: {e}",
            hub.name
        )),
        Err(e) => Err(e),
    }
}
