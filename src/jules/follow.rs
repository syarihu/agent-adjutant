//! The first time a session is seen with a pull request: record it and tell the hub.

use super::*;

use crate::task;

/// The first time a session is seen with a pull request: write it onto the task, move the
/// card to review, and tell the hub, which has the PR's description to rewrite.
///
/// Keyed on the record having no PR yet, so it happens once however many times the session
/// finishes — it finishes again after every round of comments it answers.
///
/// All of it under the task's lock, decided on the record as it is now rather than as it was
/// when the question to Jules went out: the answer can take seconds, and by then the task may
/// have been finished, pulled back, or given another session, whose card this PR is not.
///
/// The hub is told first and the record written after. The other order loses the message for
/// good when delivery fails: the record already has its PR, so no later poll gets this far
/// again. This order at worst tells the hub twice, when the write fails after a delivery.
pub fn follow(
    ctx: &crate::registry::Context,
    task_id: &str,
    session: &Session,
) -> Result<(), String> {
    let Some(pr) = &session.pr else {
        return Ok(());
    };
    let lock = task::lock(ctx, task_id)?;
    let mut task = task::get(&ctx.state, &ctx.repo.slug, task_id)?;
    let waiting = task.pr.is_none()
        && task.jules_session.as_deref() == Some(session.id.as_str())
        && matches!(task.status, task::Status::Dispatched | task::Status::Pr);
    if !waiting {
        return Ok(());
    }
    let message = crate::mail::Message {
        from: "jules".to_string(),
        // None, for the reason `task::hand_over` gives: this comes from no worktree.
        worktree: None,
        kind: "jules-pr".to_string(),
        subject: task.title.clone(),
        body: format!(
            "## task        {}\n## pr          {pr}\n## session     {}\n",
            task.id, session.id
        ),
    };
    // Posted under the lock, so no second poll can post it again; the hub is woken and the
    // person told after the lock is let go, since those run commands that may not return.
    let posted = crate::mail::post_to_hub(ctx, &message)?;
    task.pr = Some(pr.clone());
    if task.status == task::Status::Dispatched {
        task.status = task::Status::Pr;
    }
    task.updated_at = crate::infra::clock::utc_stamp(crate::infra::clock::now_secs());
    let saved = task::save(ctx, &task);
    drop(lock);
    posted.follow_up(ctx, true);
    saved.map(|_| ())
}
