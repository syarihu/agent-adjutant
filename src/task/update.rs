//! Changing a task record, and handing it over when the change queues it.

use super::*;

use crate::mail::DeliveryOutcome;
use crate::registry::Context;

/// Change a record, and hand it over if `hand` is set and this is the change that queued it.
pub fn update(
    ctx: &Context,
    id: &str,
    patch: &TaskPatch,
    hand: bool,
) -> Result<(Task, Option<DeliveryOutcome>), String> {
    update_checked(ctx, id, patch, hand, |_| Ok(()))
}

/// `update`, refusing when `check` says so about the record as it is *under the lock*: a
/// check made before the lock is taken can pass for two callers at once. The patch is read
/// before this is called, so a bad value is refused before the lock is taken; under it only
/// `check` runs and the fields are applied.
pub fn update_checked(
    ctx: &Context,
    id: &str,
    patch: &TaskPatch,
    hand: bool,
    check: impl FnOnce(&Task) -> Result<(), String>,
) -> Result<(Task, Option<DeliveryOutcome>), String> {
    // Before the lock, so a refusal takes nothing.
    patch.check()?;
    let lock = store::lock(ctx, id)?;
    let mut task = get(&ctx.state, &ctx.repo.slug, id)?;
    check(&task)?;
    let was = task.status;

    if let Some(status) = patch.status {
        task.status = status;
    }
    if let Some(order) = patch.order {
        task.order = order;
    }
    if let Some(auto_start) = patch.auto_start {
        task.auto_start = auto_start;
    }
    if let Some(executor) = patch.executor {
        task.executor = executor;
    }
    let pr_before = task.pr.clone();
    for (value, field) in [
        (&patch.worktree, &mut task.worktree),
        (&patch.issue, &mut task.issue),
        (&patch.pr, &mut task.pr),
        (&patch.base, &mut task.base),
        (&patch.parent, &mut task.parent),
        (&patch.jules_session, &mut task.jules_session),
        (&patch.jules_by, &mut task.jules_by),
        (&patch.note, &mut task.note),
        (&patch.instruction, &mut task.instruction),
    ] {
        if let Some(value) = value {
            field.clone_from(value);
        }
    }
    // What the board has brought to the hub belongs to the PR it read. Another PR starts over:
    // kept, the count would leave a new PR at the limit before its first review.
    if task.pr != pr_before {
        task.announced.clear();
        task.relay_rounds = 0;
        // Likewise what the last refresh read: it described the old PR.
        task.pr_status = None;
        task.pr_turn_at = None;
    }
    // A park is a person's call about a task still in play: a finished one is refused, and one
    // that becomes finished here loses it (as a merged PR's does in `refresh`).
    match &patch.parked {
        Some(Some(_)) if matches!(task.status, Status::Done | Status::Cancelled) => {
            return Err("a finished task cannot be parked".to_string());
        }
        Some(Some(request)) => {
            // As `check_park` read them: a direct caller's " pdm" is stored as "pdm".
            let reason = request.reason.trim().to_string();
            let text = request
                .text
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string);
            let same = task
                .parked
                .as_ref()
                .is_some_and(|p| p.reason == reason && p.text == text && !p.since.is_empty());
            if !same {
                task.parked = Some(Park {
                    reason,
                    text,
                    since: store::stamp(),
                    extra: Default::default(),
                });
            }
        }
        Some(None) => task.parked = None,
        None => {}
    }
    if matches!(task.status, Status::Done | Status::Cancelled) {
        task.parked = None;
    }
    // Only a worktree given in this update: one already stored was resolved when it was
    // given, against the directory of the command that gave it, and re-resolving it here
    // would read it against wherever this update happens to be run from.
    if patch.worktree.is_some() {
        task.worktree = task.worktree.as_deref().map(resolved_worktree);
    }
    task.updated_at = store::stamp();
    store::save(ctx, &task)?;
    drop(lock);

    // Handing over is a *transition*, not a status: re-sending on every save would put one
    // task in the inbox once for every time somebody dragged its card.
    // Not when the hub is the one queueing it: the inbox it would land in is its own. That is
    // a resumed worker turned away for a slot, whose record was `dispatched` or `pr`.
    let handed = if hand && was != Status::Queued && task.status == Status::Queued {
        Some(hand_over(ctx, &task)?)
    } else {
        None
    };
    Ok((task, handed))
}
