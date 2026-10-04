//! Changing a task record, and handing it over when the change queues it.

use super::*;

use serde_json::Value;

use crate::mail::DeliveryOutcome;
use crate::registry::Context;

/// One of a record's text fields as an update gives it. `null` and `""` clear it; anything
/// that is not a string is refused rather than read as "clear" — `{"pr": 42}` from a mistaken
/// caller would otherwise wipe the URL it meant to set.
pub(super) fn text_field(key: &str, value: &Value) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(v) if v.is_empty() => Ok(None),
        Value::String(v) => Ok(Some(v.clone())),
        other => Err(format!("{key} has to be a string or null, not {other}")),
    }
}

/// Change a record, and hand it over if this is the change that queued it.
pub fn update(
    ctx: &Context,
    id: &str,
    input: &Value,
) -> Result<(Task, Option<DeliveryOutcome>), String> {
    update_checked(ctx, id, input, |_| Ok(()))
}

/// `update`, refusing when `check` says so about the record as it is *under the lock*: a
/// check made before the lock is taken can pass for two callers at once.
pub fn update_checked(
    ctx: &Context,
    id: &str,
    input: &Value,
    check: impl FnOnce(&Task) -> Result<(), String>,
) -> Result<(Task, Option<DeliveryOutcome>), String> {
    let lock = store::lock(ctx, id)?;
    let mut task = get(&ctx.state, &ctx.repo.slug, id)?;
    check(&task)?;
    let was = task.status;

    if let Some(status) = string(input, "status") {
        task.status = Status::parse(&status).ok_or(format!("no such status: {status}"))?;
    }
    if let Some(order) = input.get("order").and_then(Value::as_u64) {
        task.order = order as u32;
    }
    // Set by the hub when a person approved a task that asked to be confirmed first, so that
    // being turned away for a slot afterwards does not put the same question to them again.
    if let Some(auto_start) = input.get("autoStart").and_then(Value::as_bool) {
        task.auto_start = auto_start;
    }
    if let Some(executor) = string(input, "executor") {
        task.executor = Executor::parse(&executor)
            .ok_or(format!("no such executor: {executor} (worker or jules)"))?;
    }
    let pr_before = task.pr.clone();
    for (key, field) in [
        ("worktree", &mut task.worktree),
        ("issue", &mut task.issue),
        ("pr", &mut task.pr),
        // The branching point the hub decided, for a task whose record did not bring one:
        // `adj jules start` reads it after the hub may have restarted.
        ("base", &mut task.base),
        ("julesSession", &mut task.jules_session),
        ("julesBy", &mut task.jules_by),
        ("note", &mut task.note),
        ("instruction", &mut task.instruction),
    ] {
        if let Some(value) = input.get(key) {
            // An explicit `null` clears; an absent key leaves it alone. Without the
            // distinction there is no way to take back a worktree the hub wrote down. An
            // empty string clears too, since a command line has no way to say `null` — and a
            // "waiting for a slot" note has to go once the worker starts.
            *field = text_field(key, value)?;
        }
    }
    // What the board has brought to the hub belongs to the PR it read. Another PR starts over:
    // kept, the count would leave a new PR at the limit before its first review.
    if task.pr != pr_before {
        task.announced.clear();
        task.relay_rounds = 0;
        // Likewise what the last refresh read: it described the old PR.
        task.pr_status = None;
    }
    // Only a worktree given in this update: one already stored was resolved when it was
    // given, against the directory of the command that gave it, and re-resolving it here
    // would read it against wherever this update happens to be run from.
    if input.get("worktree").is_some() {
        task.worktree = task.worktree.as_deref().map(resolved_worktree);
    }
    task.updated_at = store::stamp();
    store::save(ctx, &task)?;
    drop(lock);

    // Handing over is a *transition*, not a status: re-sending on every save would put one
    // task in the inbox once for every time somebody dragged its card.
    // Not when the hub is the one queueing it: the inbox it would land in is its own. That is
    // a resumed worker turned away for a slot, whose record was `dispatched` or `pr`.
    let hand = input
        .get("handOver")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let handed = if hand && was != Status::Queued && task.status == Status::Queued {
        Some(hand_over(ctx, &task)?)
    } else {
        None
    };
    Ok((task, handed))
}
