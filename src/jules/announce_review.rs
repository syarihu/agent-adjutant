//! Bringing new review comments on a Jules task's pull request to the hub.

use super::*;

use crate::task;

/// New review comments on a Jules task's PR, brought to the hub, which prepares them for a
/// person to approve passing on.
///
/// Only while Jules is idle: a session working is answering the last round, and comments that
/// arrive meanwhile are read with the next. Each comment is brought up once, and a PR at most
/// `RELAY_ROUNDS` times; past that the card is left to the side sheet's manual relay.
pub fn announce_review(
    ctx: &crate::registry::Context,
    task_id: &str,
    session: &Session,
) -> Result<(), String> {
    if working(&session.state) {
        return Ok(());
    }
    let eligible = |t: &task::Task| {
        t.status == task::Status::Pr
            && t.pr.is_some()
            && t.jules_session.as_deref() == Some(session.id.as_str())
            && t.relay_rounds < RELAY_ROUNDS
    };
    // A first look without the lock: listing the comments is two round trips to GitHub, and
    // most polls end here.
    if !eligible(&task::get(&ctx.state, &ctx.repo.slug, task_id)?) {
        return Ok(());
    }
    let listed = findings(ctx, task_id)?;
    let edited = task::edit(ctx, task_id, |task| {
        if !eligible(task) {
            return Ok(task::Edit::Keep(None));
        }
        let new: Vec<&Finding> = listed
            .iter()
            .filter(|f| {
                !f.relayed && !task.relayed.contains(&f.id) && !task.announced.contains(&f.id)
            })
            .collect();
        if new.is_empty() {
            return Ok(task::Edit::Keep(None));
        }
        let round = task.relay_rounds + 1;
        let ids: Vec<&str> = new.iter().map(|f| f.id.as_str()).collect();
        let message = crate::mail::Message {
            from: "jules".to_string(),
            // None, for the reason `task::hand_over` gives.
            worktree: None,
            kind: "jules-review".to_string(),
            subject: task.title.clone(),
            body: format!(
                "## task        {}\n## pr          {}\n## session     {}\n## round       {round}/{RELAY_ROUNDS}\n## comments    {}\n",
                task.id,
                task.pr.as_deref().unwrap_or_default(),
                session.id,
                ids.join(" ")
            ),
        };
        // Posted first and written after, under the lock; woken and notified after it, for the
        // reasons `follow` gives.
        let posted = crate::mail::post_to_hub(ctx, &message)?;
        task.announced.extend(ids.iter().map(|id| id.to_string()));
        task.relay_rounds = round;
        Ok(task::Edit::Write(Some(posted)))
    })?;
    if let Some(posted) = edited.value {
        posted.follow_up(ctx, true);
    }
    edited.saved
}
