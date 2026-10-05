use crate::kernel::identity;
use crate::mail::Message;
use crate::mail::{NotWoken, Reached};
use serde_json::{Value, json};

use super::{cwd_param, cwd_property, hub_property, repo_property, resolve_repo};

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_send",
        "description": "Deliver a message to a repository's hub. Never fails for want of a listener: if the hub is not running the message waits in its inbox and is picked up when it next starts, and the reply says which of the two happened. Use for bug reports found mid-task, answers to a hub's question, acknowledgements, and telling the hub a task is finished so it can close this tab and clear the worktree. The message records the worktree you are sending from, derived from `cwd`, so pass `cwd` whenever you are not in it.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "body": { "type": "string", "description": "The message. Markdown; keep it under 30 lines — the hub reshapes it into an issue." },
                "subject": { "type": "string", "description": "One line stating the conclusion. This is all a human sees in a listing." },
                "from": { "type": "string", "description": "Who is sending: your session or worktree name." },
                "kind": { "type": "string", "description": "report (default) | question | answer | ack | done | needs-user | request | next | gate | jules-pr | jules-review" },
                "wake": { "type": "boolean", "description": "Whether to wake the hub. Defaults to automatic: wakes for actionable messages (reports, answers, done, requests, next, gate, jules-pr, jules-review); delivers without waking for questions, needs-user, ack, or when sending to yourself." },
                "repo": repo_property(),
                "hub": hub_property(),
                "cwd": cwd_property(),
            },
            "required": ["body"],
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let info = resolve_repo(args)?;
    let body = args["body"].as_str().unwrap_or("").trim();
    if body.is_empty() {
        return Err("body is empty".to_string());
    }
    let message = Message {
        from: args["from"].as_str().unwrap_or("unknown").to_string(),
        // From the caller's own `cwd` when it gave one: this server is started once
        // and then asked about whichever checkout the session is sitting in, so the
        // process's own directory is not the sender's.
        worktree: identity::current_worktree(cwd_param(args).as_deref()),
        kind: args["kind"].as_str().unwrap_or("report").to_string(),
        subject: args["subject"].as_str().unwrap_or("").to_string(),
        body: body.to_string(),
    };
    let wake = args.get("wake").and_then(|v| v.as_bool());
    let ctx = crate::registry::context_of(info)?;
    let delivered = crate::mail::deliver_to_hub_with_wake(&ctx, &message, true, wake)?;
    let (note, why) = match &delivered.reached {
        Reached::Woken => (
            "Woke the hub; it will pick this up. Do not wait for a reply, go back to your own task.",
            None,
        ),
        Reached::Running {
            wake: NotWoken::NotNeeded,
        } => (
            "The hub is running; waking was skipped because this message needs no action. It will pick this up the next time it checks its inbox.",
            None,
        ),
        Reached::Running {
            wake: NotWoken::Held { why },
        } => (
            "The hub is running; it will pick this up the next time it checks its inbox. Do not wait for a reply, go back to your own task.",
            why.as_ref(),
        ),
        Reached::NotRunning => (
            "The hub is not running. Left in its inbox; it will be picked up the next time it starts. If this is urgent, ask the user to run `adj hub`.",
            None,
        ),
    };
    let note = match why {
        Some(why) => format!(
            "{} {note}",
            crate::transport::wording::wake_note_sentence(why)
        ),
        None => note.to_string(),
    };
    let mut out = json!({
        "hubName": ctx.repo.hub_name,
        "present": delivered.is_present(),
        "woken": delivered.was_woken(),
        "path": delivered.path.to_string_lossy(),
        "note": note,
    });
    if let Some(why) = why {
        out["wakeNote"] = json!(why);
    }
    Ok(out)
}
