use std::path::Path;

use super::resume::own_hub_context;
use crate::board::view::find_session;
use crate::board::{Restarting, Server, resume_refusal, settings_now};
use crate::lifecycle::worker::{Started, resume_worker, saved_worker_session};
use crate::mail;
use crate::registry;

/// What restarting a worker came to.
pub struct Restarted {
    pub was_running: bool,
    pub description: String,
    pub hub: Option<String>,
    pub hub_running: bool,
}

/// Stop the worker session `id` and start it again on the same conversation, in one step:
/// what closing it and then resuming it would do, with every refusal made before anything is
/// closed. A restart that finds out afterwards that the conversation cannot be reopened has
/// stopped a worker for nothing.
///
/// A worker that does not go is not forced: nothing is started, and its record stays. Starting
/// beside it would be two workers in one worktree.
pub fn restart(server: &Server, id: &str) -> Result<Restarted, String> {
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    if session.kind != "worker" {
        return Err("only a worker session can be restarted".to_string());
    }
    let worktree = Path::new(&session.worktree);
    if registry::is_starting(worktree, crate::infra::clock::now_secs()) {
        return Err("the session is starting".to_string());
    }
    if let Some(refusal) = resume_refusal(&settings) {
        return Err(refusal);
    }
    saved_worker_session(worktree)?;
    // Without a way to close the window the restart would end up starting a second worker
    // beside the one it could not stop.
    if settings.terminal.close.is_off() {
        return Err(
            "terminal.close is off, so the session cannot be closed to restart it".to_string(),
        );
    }
    // Built before anything is closed: it does not depend on the close, and a failure here
    // after it would have closed the session for nothing.
    let ctx = own_hub_context(server, settings)?;
    let _restarting = Restarting::claim(&session.worktree, "the session")?;
    let was_running = registry::worker_status(worktree).present;
    let repo = server.ctx.repo.nwo.clone();
    if !crate::lifecycle::worker::close(&ctx.settings, worktree, false)?.is_free() {
        return Err(
            "the session could not be closed (pid still running or its record unreadable); \
             nothing was restarted"
                .to_string(),
        );
    }
    let done = match resume_worker(&ctx, Some(&repo), None, &session.worktree, "", None, false) {
        Ok(Started::Opened(done)) => done,
        Ok(Started::Full(refusal)) | Err(refusal) => {
            // Nothing was closed when nothing was running, and saying so would be false.
            return Err(match was_running {
                true => format!("closed the session, but could not start it again: {refusal}"),
                false => format!("could not start the session again: {refusal}"),
            });
        }
    };
    let hub_running = mail::all_repo_hubs(&server.ctx.state, &server.ctx.repo)
        .iter()
        .any(|h| Some(&h.id) == session.hub.as_ref() && h.state.present);
    Ok(Restarted {
        was_running,
        description: done.description,
        hub: session.hub,
        hub_running,
    })
}
