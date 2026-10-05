use crate::mail::{NotWoken, Reached};
use serde_json::{Value, json};

use super::{cwd_property, hub_property, repo_property, resolve_repo};

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_tell",
        "description": "Leave a message for the worker in a worktree, and wake it if it is sitting there. This is the hub-to-worker direction: the address is the worktree, not a session, so it reaches whatever agent is working there whatever it is doing. Start the subject with `[question <id>]` when you need an answer back — that marker is what tells the worker it may reply.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "worktree": { "type": "string", "description": "Absolute path of the worktree." },
                "subject": { "type": "string", "description": "One line stating the point. `[question <id>]` asks for an answer; `[ack]` acknowledges; anything else is a notice." },
                "body": { "type": "string", "description": "The message. Markdown." },
                "from": { "type": "string", "description": "Who is speaking (default: this repository's hub name)." },
                "wake": { "type": "boolean", "description": "Whether to wake the worker. Defaults to automatic based on the subject: wakes for `[question <id>]` and gate answers; delivers without waking for `[ack]` and plain notices." },
                "repo": repo_property(),
                "hub": hub_property(),
                "cwd": cwd_property(),
            },
            "required": ["worktree", "subject", "body"],
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let info = resolve_repo(args)?;
    let worktree = crate::infra::paths::expand_home(args["worktree"].as_str().unwrap_or(""));
    if !worktree.is_dir() {
        return Err(format!("no such worktree: {}", worktree.display()));
    }
    let subject = args["subject"].as_str().unwrap_or("").trim();
    let body = args["body"].as_str().unwrap_or("").trim();
    if subject.is_empty() || body.is_empty() {
        return Err("subject and body are both required".to_string());
    }
    let ctx = crate::registry::context_of(info)?;
    let from = args["from"]
        .as_str()
        .unwrap_or(&ctx.repo.hub_name)
        .to_string();
    let wake = args.get("wake").and_then(|v| v.as_bool());
    let told = crate::mail::deliver_to_worker(&ctx, &worktree, &from, subject, body, wake)?;
    let (note, why) = match &told.reached {
        Reached::Woken => (
            "Woke the worker. Do not wait for a reply, go back to waiting.",
            None,
        ),
        Reached::Running {
            wake: NotWoken::NotNeeded,
        } => (
            "The worker is running; waking was skipped because this message needs no action. It will read this the next time it checks its outbox.",
            None,
        ),
        Reached::Running {
            wake: NotWoken::Held { why },
        } => (
            "The worker is running; it will read this the next time it checks its outbox.",
            why.as_ref(),
        ),
        Reached::NotRunning => (
            "The worker is not running; it will read this the next time it starts.",
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
        "worktree": worktree.to_string_lossy(),
        "outbox": told.path.to_string_lossy(),
        "present": told.is_present(),
        "woken": told.was_woken(),
        "note": note,
    });
    if let Some(why) = why {
        out["wakeNote"] = json!(why);
    }
    Ok(out)
}
