use std::path::Path;

use serde_json::{Value, json};

use super::start::{hub_context, inbox_name, start_if_stopped};
use crate::board::view::find_session;
use crate::board::{Server, input_of, settings_now, text};
use crate::lifecycle::worker::{LinkRequest, LinkTo, Linked};
use crate::mail::Message;
use crate::task;

/// Join the session `id` to a task, and to the hub that task belongs to.
///
/// The task is either an existing one (`task`) or a new one (`newTask`, the fields
/// `POST /api/tasks` takes) and is looked up or made in the task directory of the hub named
/// by `hub` — that directory is what says which hub a task belongs to, and the worker's
/// record follows it. The link itself is `lifecycle::worker::link`'s; what is here is the
/// request, the issue the hub is asked to file and the answer.
pub fn link(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input = input_of(body)?;
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    if session.kind != "worker" {
        return Err("only a worker session can be linked to a task".to_string());
    }
    let (hub, ctx) = hub_context(server, text(&input, "hub")?, settings)?;
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
            LinkTo::New(task::NewTask::from_json(&Value::Object(fields))?)
        }
        (None, None) => return Err("a task or a newTask is required".to_string()),
    };
    let phase = text(&input, "phase")?.map(str::to_string);
    let worktree = session.worktree.as_str();
    let Linked {
        task: linked,
        made_here,
    } = crate::lifecycle::worker::link(&ctx, Path::new(worktree), LinkRequest { to, phase })?;
    // Only a task this link made: an existing one may carry `file-and-start` from its own
    // request, which was already handed to a hub.
    let files_issue = made_here && linked.kind == task::Kind::FileAndStart;
    let mut reply = json!({
        "task": linked,
        "session": { "id": session.id, "task": linked.id, "hub": hub.id },
    });
    if files_issue {
        // The link stands whatever happens here: the worker already has its task, and a hub that
        // could not be told is something to say, not to undo.
        let message = Message {
            from: "dashboard".to_string(),
            worktree: None,
            kind: "file-issue".to_string(),
            subject: format!("[file {}] {}", linked.id, linked.title),
            body: format!(
                "{}\n## Worker running in {worktree}\n",
                task::render_request(&linked)
            ),
        };
        match crate::mail::deliver_to_hub(&ctx, &message) {
            Ok(delivered) => {
                reply["fileIssue"] = json!({
                    "handed": crate::mail::Handed::from(&delivered),
                    "message": inbox_name(&delivered),
                });
                let (started, error) = start_if_stopped(server, &ctx, &delivered);
                reply["hubStarted"] = json!(started);
                if let Some(e) = error {
                    reply["hubStartError"] = json!(e);
                }
            }
            Err(e) => {
                reply["fileIssueError"] = json!(e);
                // The worker was told the hub would file it: say it will not, in the words it
                // already handles, so it does not wait for a URL.
                let _ = crate::mail::deliver_to_worker(
                    &ctx,
                    Path::new(worktree),
                    "dashboard",
                    &format!("[issue {}] not filed: the hub could not be told", linked.id),
                    "Nothing will file an issue for this task automatically. Carry on without one.",
                    None,
                );
            }
        }
    }
    Ok(reply)
}
