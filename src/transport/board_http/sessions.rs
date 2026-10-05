//! The session routes whose operations answer a value rather than JSON: reopening, restarting
//! and opening a session, starting one with no task, linking one to a task, removing its
//! worktree, and its worktree's git state. Each reads the request, calls the board, and words
//! the answer. It also holds the body readers `input_of` and `text`, which `handlers` borrows.

use serde_json::{Value, json};

use crate::board::session::{
    BranchRemoval, CleanUp, CleanUpRequest, LinkSession, Loss, OpenedIn, StartSession, clean_up,
    inbox_name, link, open, restart, resume, start,
};
use crate::board::view::{find_session, session_git};
use crate::board::{Server, settings_now};
use crate::lifecycle::worker::LinkTo;
use crate::mail::Handed;
use crate::task::NewTask;

pub(super) fn input_of(body: &[u8]) -> Result<Value, String> {
    let input: Value = match body.is_empty() {
        true => json!({}),
        false => serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?,
    };
    match input.is_object() {
        true => Ok(input),
        false => Err("expected an object".to_string()),
    }
}

/// A string field, trimmed; blank and `null` are absent. Anything that is not a string is
/// refused rather than read as absent, as `TaskPatch::from_json` does for a task update.
pub(super) fn text<'a>(input: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.trim()).filter(|s| !s.is_empty())),
        Some(other) => Err(format!("{key} has to be a string, not {other}")),
    }
}

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

/// Ask a hub to start a session with no task. The body is read before the board is asked, so a
/// body wrong in any way is refused before the agent, the name, the worker limit or the hub is
/// looked at.
pub(super) fn start_session(server: &Server, body: &[u8]) -> Result<Value, String> {
    let input = input_of(body)?;
    // Optional: the worker greets the person and waits when there is none.
    let instruction = text(&input, "instruction")?.unwrap_or("");
    // The brief writes a missing instruction as `-`, so the hub could not tell this one apart.
    if instruction.trim() == "-" {
        return Err("an instruction of only `-` means no instruction; leave it empty".to_string());
    }
    let request = StartSession {
        instruction: instruction.to_string(),
        agent: text(&input, "agent")?.map(str::to_string),
        worktree_name: text(&input, "worktreeName")?.map(str::to_string),
        hub: text(&input, "hub")?.map(str::to_string),
    };
    let asked = start(server, request)?;
    let mut reply = json!({
        "handed": Handed::from(&asked.delivered),
        "hubStarted": matches!(asked.hub_started, Ok(true)),
        "worktreeName": asked.worktree_name,
        "hub": asked.hub,
        "message": inbox_name(&asked.delivered),
    });
    if let Err(e) = &asked.hub_started {
        reply["hubStartError"] = json!(e);
    }
    Ok(reply)
}

/// Join a session to a task. The body is read before the session is looked up, so a body wrong
/// in any way is refused before an unknown session or hub.
pub(super) fn link_session(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input = input_of(body)?;
    let hub = text(&input, "hub")?.map(str::to_string);
    let to = match (text(&input, "task")?, input.get("newTask")) {
        (Some(_), Some(_)) => return Err("give either task or newTask, not both".to_string()),
        (Some(task_id), None) => {
            // An existing task already has its issue or its reason not to; only a task made by
            // this link can ask the hub to file one.
            if text(&input, "kind")? == Some("file-and-start") {
                return Err(
                    "file-and-start needs a newTask: an existing task is not filed".to_string(),
                );
            }
            LinkTo::Existing(task_id.to_string())
        }
        (None, Some(new_task)) => {
            let mut fields = new_task
                .as_object()
                .cloned()
                .ok_or("newTask has to be an object")?;
            // What the link decides itself, whatever the caller sent: removed before the
            // rest is read, so that junk in them is not refused.
            fields.remove("worktreeName");
            fields.remove("status");
            fields.remove("worktree");
            LinkTo::New(NewTask::from_json(&Value::Object(fields))?)
        }
        (None, None) => return Err("a task or a newTask is required".to_string()),
    };
    let phase = text(&input, "phase")?.map(str::to_string);
    let linked = link(server, id, LinkSession { hub, to, phase })?;
    let mut reply = json!({
        "task": &linked.task,
        "session": { "id": linked.session, "task": linked.task.id, "hub": linked.hub },
    });
    match &linked.file_issue {
        None => {}
        Some(Ok(asked)) => {
            reply["fileIssue"] = json!({
                "handed": Handed::from(&asked.delivered),
                "message": inbox_name(&asked.delivered),
            });
            reply["hubStarted"] = json!(matches!(asked.hub_started, Ok(true)));
            if let Err(e) = &asked.hub_started {
                reply["hubStartError"] = json!(e);
            }
        }
        Some(Err(e)) => reply["fileIssueError"] = json!(e),
    }
    Ok(reply)
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
