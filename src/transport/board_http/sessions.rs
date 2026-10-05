//! The session routes whose operations answer a value rather than JSON: reopening, restarting
//! and opening a session, removing its worktree, and its worktree's git state. Each reads the
//! request, calls the board, and words the answer.

use serde_json::{Value, json};

use crate::board::session::{
    BranchRemoval, CleanUp, CleanUpRequest, Loss, OpenedIn, clean_up, open, restart, resume,
};
use crate::board::view::{find_session, session_git};
use crate::board::{Server, input_of, settings_now, text};

/// The body is refused before the session is looked up, as the operation did when it read it.
pub(super) fn resume_session(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    input_of(body)?;
    let done = resume(server, id)?;
    Ok(json!({
        "resumed": true,
        "description": done.description,
        "hub": done.hub,
        "hubRunning": done.hub_running,
    }))
}

pub(super) fn restart_session(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    input_of(body)?;
    let done = restart(server, id)?;
    Ok(json!({
        "restarted": true,
        "wasRunning": done.was_running,
        "description": done.description,
        "hub": done.hub,
        "hubRunning": done.hub_running,
    }))
}

pub(super) fn open_session(server: &Server, id: &str) -> Result<Value, String> {
    let opened = open(server, id)?;
    let terminal = match opened.terminal {
        OpenedIn::Attach => "terminal.attach",
        OpenedIn::ITerm2 => "iTerm2",
    };
    Ok(json!({
        "opened": true,
        "description": format!("opened {id} in {terminal}"),
        "session": opened.session,
        "window": opened.window,
    }))
}

pub(super) fn git_of_session(server: &Server, id: &str) -> Result<Value, String> {
    match session_git(server, id)? {
        Some(state) => {
            serde_json::to_value(state).map_err(|e| format!("cannot describe the worktree: {e}"))
        }
        // Only a gone worktree comes here, so the usual path still looks the session up once.
        None => {
            let session = find_session(server, &settings_now(server), id)?;
            Err(format!("{} does not exist", session.worktree))
        }
    }
}

/// Remove a session's worktree. A removal that would lose work is answered 200 with
/// `removed: false` and the reasons, like `closed: false` is: the page keeps only `error` from
/// a non-2xx answer, and the reasons are what the person needs to decide whether to force.
///
/// The body is read before the session is looked up, as the operation did.
pub(super) fn clean_up_session(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input = input_of(body)?;
    let force = match input.get("force") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(force)) => *force,
        Some(other) => return Err(format!("force has to be true or false, not {other}")),
    };
    let confirm = text(&input, "confirm")?.map(str::to_string);
    match clean_up(server, id, CleanUpRequest { force, confirm })? {
        CleanUp::Kept {
            reasons,
            git,
            closed,
        } => {
            let mut reply = json!({
                "removed": false,
                "reasons": reasons.iter().map(loss_of).collect::<Vec<_>>(),
                "git": git,
            });
            if let Some(closed) = closed {
                reply["closed"] = json!(closed);
            }
            Ok(reply)
        }
        CleanUp::Removed(done) => {
            let branch = match &done.branch {
                Some(BranchRemoval {
                    name,
                    deleted: Ok(()),
                }) => json!({ "name": name, "deleted": true }),
                Some(BranchRemoval {
                    name,
                    deleted: Err(e),
                }) => json!({ "name": name, "deleted": false, "error": e }),
                None => json!({ "name": Value::Null, "deleted": false }),
            };
            let hooks: Vec<Value> = done
                .hooks
                .iter()
                .map(|hook| match &hook.result {
                    Ok(()) => json!({ "command": hook.command, "ok": true }),
                    Err(e) => json!({ "command": hook.command, "ok": false, "error": e }),
                })
                .collect();
            let mut reply = json!({
                "removed": true,
                "forced": done.forced,
                "closed": done.closed,
                "branch": branch,
                "tasks": done.tasks,
                "hooks": hooks,
            });
            if !done.task_errors.is_empty() {
                reply["taskErrors"] = done
                    .task_errors
                    .iter()
                    .map(|(id, error)| json!({ "id": id, "error": error }))
                    .collect();
            }
            Ok(reply)
        }
    }
}

fn loss_of(loss: &Loss) -> Value {
    match loss {
        Loss::Git(e) => json!({ "kind": "git", "detail": e }),
        Loss::Uncommitted {
            files,
            insertions,
            deletions,
        } => json!({
            "kind": "uncommitted",
            "detail": format!("{files} changed file(s), +{insertions} -{deletions} lines"),
        }),
        Loss::Untracked(n) => json!({
            "kind": "untracked",
            "detail": format!("{n} untracked path(s)"),
        }),
        Loss::Unpushed(n) => json!({
            "kind": "unpushed",
            "detail": format!("{n} commit(s) no remote has"),
        }),
    }
}
