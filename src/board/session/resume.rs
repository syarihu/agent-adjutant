use std::path::Path;

use serde_json::{Value, json};

use crate::board::view::find_session;
use crate::board::{Server, input_of, resume_refusal, settings_now};
use crate::lifecycle::worker::{Started, resume_worker};
use crate::mail;
use crate::registry::{self, Context};

/// The context of the repository's own hub, as `adj work --resume` builds one, but from what the
/// server holds: the server's directory is nowhere near the repository, so nothing here may
/// resolve anything from it.
pub(super) fn own_hub_context(
    server: &Server,
    settings: crate::kernel::config::Settings,
) -> Result<Context, String> {
    Ok(Context {
        repo: server.ctx.repo.clone().addressed(None)?,
        resolved: server.ctx.resolved.clone(),
        state: server.ctx.state.clone(),
        settings,
    })
}

/// Reopen the worker session `id` in a new tab, as `adj work --resume` does.
///
/// The worker goes back under the hub that dispatched it without being told which: the saved
/// session remembers it (rewritten on every link), and `adj worker --resume` reads it there.
/// Forwarding this server's own hub would re-file the worker under whichever hub the board
/// happens to be for.
pub fn resume(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    input_of(body)?;
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    if session.kind != "worker" {
        return Err("only a worker session can be resumed".to_string());
    }
    if session.present {
        return Err("the session is running".to_string());
    }
    let worktree = Path::new(&session.worktree);
    if registry::is_starting(worktree, crate::infra::clock::now_secs()) {
        return Err("the session is starting".to_string());
    }
    if let Some(refusal) = resume_refusal(&settings) {
        return Err(refusal);
    }
    let ctx = own_hub_context(server, settings)?;
    let repo = ctx.repo.nwo.clone();
    let done = match resume_worker(&ctx, Some(&repo), None, &session.worktree, "", None, false)? {
        Started::Opened(done) => done,
        Started::Full(refusal) => return Err(refusal),
    };
    let hub_running = mail::all_repo_hubs(&server.ctx.state, &server.ctx.repo)
        .iter()
        .any(|h| Some(&h.id) == session.hub.as_ref() && h.state.present);
    Ok(json!({
        "resumed": true,
        "description": done.description,
        "hub": session.hub,
        "hubRunning": hub_running,
    }))
}
