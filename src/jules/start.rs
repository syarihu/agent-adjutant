//! Starting a session for a task, and recording it on the task.

use super::*;

use crate::task;

/// Start a Jules session for task `id` from `prompt`, and write its id onto the record.
///
/// `base` is the branch to start from, as given; without it the task's own base is used.
pub fn start(
    ctx: &crate::registry::Context,
    id: &str,
    prompt: &str,
    base: Option<&str>,
) -> Result<Session, String> {
    // Read before anything is started: a session that exists with nobody recorded as its
    // starter would have its review comments passed on in the wrong name, and a lookup that
    // hangs after the session is created would leave it running unrecorded.
    let by = github_login(&ctx.repo.main);
    // Held from the checks to the record, across the call that creates the session. Two starts
    // for one task at once would otherwise both find it without a session and both create one,
    // and the record would keep whichever wrote last.
    let edited = task::edit(ctx, id, |task| {
        // Handed over by the hub once the task is in progress, and not before. The board
        // follows a session only while its task is in progress or in review, so one started
        // for a task still in the backlog or the queue would run with nothing watching it.
        if task.status != task::Status::Dispatched {
            return Err(format!(
                "{} is {}, not in progress: a task goes to Jules once the hub has dispatched it and its plan is approved",
                task.id,
                task.status.as_str()
            ));
        }
        // Who implements was decided when the task was written down, and a worker-implemented
        // task handed to Jules as well would be implemented twice.
        if task.executor != task::Executor::Jules {
            return Err(format!(
                "{} is to be implemented by its worker; `adj task update --id {} --executor jules` first if Jules should do it",
                task.id, task.id
            ));
        }
        // A second session for one task is two pull requests for one change, and the first one
        // would be forgotten: the record keeps one id.
        if let Some(session) = &task.jules_session {
            return Err(format!(
                "{} is already with Jules (session {session}); clear it with `adj task update --id {} --jules-session ''` to start another",
                task.id, task.id
            ));
        }
        // Given explicitly, the branch is taken as it is. The task's own base is what was typed
        // on the board, where `origin/feature/x` is as likely as `feature/x`; Jules wants the
        // name GitHub has, so that one is checked against this checkout's remote-tracking refs.
        let base = match (base, task.base.as_deref()) {
            (Some(given), _) => given.to_string(),
            (None, Some(stored)) => branch_on_github(&ctx.repo.main, &ctx.repo.nwo, stored),
            (None, None) => {
                return Err("which branch should Jules start from? write it onto the task with `adj task update --id <task> --base <branch>`, or pass --base (without origin/)".to_string());
            }
        };
        let session = api::create(
            &ctx.settings.jules_key,
            &ctx.repo.nwo,
            &base,
            &task.title,
            prompt,
        )?;
        task.jules_session = Some(session.id.clone());
        // Who started it, as far as `gh` can say. Jules acts on comments by that person only,
        // so a comment passed on in anybody else's name would be posted and ignored.
        task.jules_by = by;
        Ok(task::Edit::Write(session))
    })?;
    // Written as soon as the session exists, under the lock. If this fails the session is
    // still running, so the error says which one it is rather than leaving it to be found on
    // jules.google.com.
    edited.saved.map_err(|e| {
        format!(
            "Jules started session {} but the task record could not be updated: {e}",
            edited.value.id
        )
    })?;
    Ok(edited.value)
}

/// The branch as GitHub names it, for a base written the way `git worktree add` takes it.
///
/// `origin/` is taken off only when this checkout says it is the remote's prefix: a
/// remote-tracking ref of that name exists, and no branch on the remote is itself called
/// `origin/…`. Anything the checkout cannot vouch for is left as it was written, for the API to
/// accept or refuse — stripping blindly would turn a real branch named `origin/x` into `x`.
///
/// Only when the checkout is a clone of the repository the session is for. `--repo` names the
/// repository without moving the command into its checkout, and another repository's refs say
/// nothing about this one's branches.
fn branch_on_github(main: &str, nwo: &str, base: &str) -> String {
    let Some(rest) = base.strip_prefix("origin/") else {
        return base.to_string();
    };
    if crate::kernel::identity::name_with_owner(main).0 != nwo {
        return base.to_string();
    }
    let known = |name: &str| {
        crate::infra::git::git(
            &[
                "-C",
                main,
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/remotes/{name}"),
            ],
            None,
        )
        .is_ok_and(|out| out.status.success())
    };
    if known(&format!("origin/{base}")) || !known(base) {
        base.to_string()
    } else {
        rest.to_string()
    }
}
