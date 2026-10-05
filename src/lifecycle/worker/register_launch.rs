use super::Launch;
use crate::infra::terminal;
use crate::kernel::identity;
use crate::registry;

/// A session-record write that failed and was got past: the worker runs either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionNote {
    /// The session could not be saved, so `--resume` may not reopen this worker.
    NotSaved(String),
    /// The session could not be re-saved under the new hub, so the worker may still be
    /// counted for its old one.
    StillUnderOldHub(String),
}

/// What registering the worker left to say.
#[derive(Debug)]
pub struct Registered {
    pub session: Option<SessionNote>,
}

/// Write down the worker `launch` plans: change into its worktree, register it and record
/// its session. The next step is `exec_launch`, which has to be this same process.
pub fn register_launch(launch: &Launch) -> Result<Registered, String> {
    let Launch {
        ctx,
        worktree,
        title,
        task,
        fresh_session,
        resumed,
        ..
    } = launch;
    let task = task.as_deref();

    std::env::set_current_dir(worktree)
        .map_err(|e| format!("cannot change directory to {}: {e}", worktree.display()))?;
    // The address goes into the record here, at the last moment before this process stops
    // being a launcher. Everything the worker's agent later sends is addressed from it.
    let location = terminal::own_location(&ctx.settings.terminal);
    registry::register_worker(
        worktree,
        title,
        ctx.repo.hub.as_deref(),
        task,
        Some(&location),
    )?;
    // Said and got past, as for the hub: a worker that cannot be resumed still works. And as
    // for the hub, a fresh start with nothing to record clears what an earlier worker saved.
    let mut session = None;
    match resumed {
        None => {
            let saved = match fresh_session {
                Some(id) => registry::save_worker_session(
                    worktree,
                    title,
                    ctx.repo.hub.as_deref(),
                    task,
                    id,
                )
                .map(|_| ()),
                None => registry::forget_worker_session(worktree),
            };
            if let Err(e) = saved {
                session = Some(SessionNote::NotSaved(e));
            }
        }
        Some(saved) => {
            // Reopened under another hub than the session remembers: say so there too, or the
            // worker would count for the old hub once it has ended and its record is gone.
            let slug_of = |hub: Option<&str>| identity::slug_for(&ctx.repo.nwo, hub);
            if slug_of(ctx.repo.hub.as_deref()) != slug_of(saved.hub.as_deref())
                && let Err(e) = registry::save_worker_session(
                    worktree,
                    title,
                    ctx.repo.hub.as_deref(),
                    task,
                    &saved.session_id,
                )
            {
                session = Some(SessionNote::StillUnderOldHub(e));
            }
        }
    }
    Ok(Registered { session })
}
