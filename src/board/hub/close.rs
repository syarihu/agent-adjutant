use super::stop::stop;
use crate::board::Server;
use crate::lifecycle::hub::{closable_check, close as close_hub};
use crate::mail::RepoHub;

/// What closing a hub from the board came to.
pub struct HubClosed {
    pub was_running: bool,
    /// Unread messages still in the hub's inbox: closing keeps them.
    pub unread: usize,
}

/// Stop `hub` and close it, so that it drops out of the list. A hub that cannot be closed is
/// refused before anything is stopped.
pub fn close(server: &Server, hub: &RepoHub) -> Result<HubClosed, String> {
    closable_check(&server.ctx.repo, hub)?;
    // A hub that will not stop is not closed: nothing is forgotten until it is gone.
    let was_running = stop(server, hub)?;
    // The stop cleared a record naming the process it stopped; a hub that registered in the
    // meantime stays, and `close_hub` refuses it as still running.
    let closed = close_hub(&server.ctx.state, &server.ctx.repo, hub)?;
    Ok(HubClosed {
        was_running,
        unread: closed.unread,
    })
}
