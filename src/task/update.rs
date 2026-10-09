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
    // What the hub reads out of a request is written before the work starts; on a started task
    // these would only disagree with the worktree and brief it already has.
    let rewrites = patch.kind.is_some()
        || patch.done_when.is_some()
        || patch.stop_at.is_some()
        || patch.issue_url.is_some()
        || patch.worktree_name.is_some()
        || patch.title.is_some();
    if rewrites && !matches!(task.status, Status::Backlog | Status::Queued) {
        return Err(format!(
            "kind, done-when, stop-at, issue-url, worktree-name and title can only be changed while a task is in the backlog or queued, not {}",
            task.status.as_str()
        ));
    }
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
    if let Some(kind) = patch.kind {
        task.kind = kind;
    }
    if let Some(done_when) = patch.done_when {
        task.done_when = done_when;
    }
    if let Some(stop_at) = patch.stop_at {
        task.stop_at = stop_at;
    }
    // A title the hub read from the issue is the title for good: a pending one (made from
    // the URL) has nothing left to wait for.
    if let Some(title) = patch.title.as_deref().and_then(one_line_title) {
        task.title = title;
        task.title_pending = false;
    }
    let pr_before = task.pr.clone();
    let reading = std::mem::take(&mut task.needs_reading);
    for (value, field) in [
        (&patch.issue_url, &mut task.issue_url),
        (&patch.worktree_name, &mut task.worktree_name),
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
    // Only the hub's `--read` clears the marker, and what it read has to be startable. Starting
    // without asking is refused outright until then — from the board's 着手 as much as the
    // command line: the hub has yet to read the request and confirm it on a gate, and the hub's
    // approval always comes after `--read`.
    if reading && patch.auto_start == Some(true) && !patch.read {
        return Err(
            "the hub has not read this request yet; it will ask on the board before starting"
                .to_string(),
        );
    }
    task.needs_reading = reading && !patch.read;
    if reading && patch.read {
        check_startable(&task)?;
        // The base was written by the hub from free text, in an update that did not carry
        // `--read` (any value is accepted there, as it always was), so it is held to the branch
        // rule here, on what is stored.
        if let Some(base) = task.base.as_deref().filter(|b| !b.is_empty()) {
            create::check_base(base)?;
        }
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
