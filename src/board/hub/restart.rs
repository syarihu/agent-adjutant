use super::reset::Reopened;
use super::start::start_context;
use super::stop::stop_context;
use crate::board::{Restarting, Server, hub_resume_refusal, settings_now};
use crate::lifecycle::hub::{HubStart, TabOutcome, hub_resume_check, start_hub, stop_hub};
use crate::mail::RepoHub;

/// Stop `hub` and start it again on the conversation it had.
pub fn restart(server: &Server, hub: &RepoHub) -> Result<Reopened, String> {
    let settings = settings_now(server);
    // Everything the start would refuse is refused before the hub is stopped, the
    // saved conversation included: a restart that cannot reopen it has only taken the
    // hub down. The saved session file is not touched; `--resume` reads it as it is.
    if let Some(refusal) = hub_resume_refusal(&settings) {
        return Err(refusal);
    }
    let start_ctx = start_context(server, hub, settings.clone())?;
    hub_resume_check(&start_ctx)?;
    let restarting = Restarting::claim(&hub.slug, &hub.name)?;
    let was_running = stop_hub(&stop_context(server, hub, settings))?;
    match start_hub(&start_ctx, HubStart::Resume) {
        Ok(outcome) => {
            if matches!(outcome, TabOutcome::Opened(_)) {
                // The window is open but the new hub has not registered yet: another
                // restart now would stop it or open a second window beside it.
                restarting.hold();
            }
            Ok(Reopened {
                was_running,
                outcome,
            })
        }
        Err(e) if was_running => Err(format!(
            "stopped {}, but could not start it again: {e}",
            hub.name
        )),
        Err(e) => Err(e),
    }
}
