use crate::mail::{NotWoken, Reached};
use serde_json::{Value, json};

use super::resolve_repo;

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
