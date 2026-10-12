//! Answering a gate, and noting the answer on its task.

use super::store::{archive, exists, load, lock, save, stamp};
use super::*;

use crate::registry::Context;

/// Hand the ball back. The gate leaves the queue and the answer lands in the outbox.
///
/// A record can be answered too, with `changes` only: a person sending back a review the
/// worker had already moved past. The answer reaches the worker the same way, and the record
/// stays where it is with the answer appended, so what was sent back is still readable.
pub fn answer(
    ctx: &Context,
    id: &str,
    decision: &str,
    choice: Option<&str>,
    comment: Option<&str>,
) -> Result<(Gate, crate::mail::DeliveryOutcome), String> {
    answer_with(ctx, id, decision, choice, comment, None).map_err(|e| e.0)
}

/// Why `answer_with` did not answer: the gate was settled by somebody else first (a typed case, so
/// a caller can tell it from a failure without reading the words), or something failed.
pub(super) struct AnswerError(pub String, pub bool);

impl From<String> for AnswerError {
    fn from(why: String) -> Self {
        AnswerError(why, false)
    }
}

/// `answer`, for an answer whose comment is made of points chosen from a read-through (see
/// `send_back_read`): the worker's message names the read-through, and the answer keeps it.
pub(super) fn answer_with(
    ctx: &Context,
    id: &str,
    decision: &str,
    choice: Option<&str>,
    comment: Option<&str>,
    handoff: Option<&ReadHandoff>,
) -> Result<(Gate, crate::mail::DeliveryOutcome), AnswerError> {
    // Locked before the gate is read, and held until it is archived: the worker's close and
    // the board's sweep decide a gate under the same lock, and an answer delivered to a gate
    // somebody else has just closed would wake a worker to something that was settled.
    let held = if exists(&ctx.state, &ctx.repo.slug, Shelf::Open, id) {
        let lock = lock(ctx, Shelf::Open, id)?;
        if !exists(&ctx.state, &ctx.repo.slug, Shelf::Open, id) {
            return Err(AnswerError(
                already_decided(ctx, id).unwrap_or_else(|| format!("no open gate: {id}")),
                true,
            ));
        }
        Some(lock)
    } else {
        None
    };
    let mut gate = match get(&ctx.state, &ctx.repo.slug, id) {
        Ok(gate) => gate,
        Err(e) => {
            return Err(match already_decided(ctx, id) {
                Some(settled) => AnswerError(settled, true),
                None => e.into(),
            });
        }
    };
    // Nothing else means anything to a worker that is not waiting: an approval of a record
    // would wake it to be told to carry on with what it is already doing.
    if !gate.wait && (decision != "changes" || choice.is_some()) {
        return Err(format!(
            "a record can only be answered with changes, not {decision}{}",
            if choice.is_some() {
                " and a choice"
            } else {
                ""
            }
        )
        .into());
    }
    if let Some(choice) = choice
        && !gate.choices.iter().any(|c| c.id == choice)
    {
        return Err(format!("no such choice on this gate: {choice}").into());
    }

    let subject = answer_subject(&gate, decision);
    let body = answer_body_from(&gate, decision, choice, comment, handoff);
    let told = if gate.answered_by_hub() {
        // `gate` rather than `answer`: the hub pairs an `answer` with a question it asked a
        // worker, and this is a person deciding on something the hub put on the board.
        let message = crate::mail::Message {
            from: "dashboard".to_string(),
            // None, for the reason `task::hand_over` gives.
            worktree: None,
            kind: "gate".to_string(),
            subject: subject.clone(),
            body: body.clone(),
        };
        crate::mail::deliver_to_hub_announcing(ctx, &message, false)?
    } else {
        crate::mail::deliver_to_worker(
            ctx,
            std::path::Path::new(&gate.worktree),
            &ctx.repo.hub_name,
            &subject,
            &body,
            None,
        )?
    };

    let comment = comment
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    if !gate.wait {
        let at = stamp();
        // Read again under the record's lock rather than appended to the copy loaded before
        // delivery: the board and `adj gate answer` can send the same record back at once,
        // and the second write would otherwise drop the first answer the worker has already
        // been given.
        let lock = lock(ctx, Shelf::Record, id)?;
        let mut gate = load(&ctx.state, &ctx.repo.slug, Shelf::Record, id).unwrap_or(gate);
        gate.answers.push(Answer {
            decision: decision.to_string(),
            comment,
            answered_at: at.clone(),
            handoff: handoff.cloned(),
            extra: serde_json::Map::new(),
        });
        save(ctx, Shelf::Record, &gate)?;
        drop(lock);
        note_answered(ctx, gate.task.as_deref(), &at);
        return Ok((gate, told));
    }

    // Archived after delivery, not before: if the outbox could not be written the gate is
    // still open, and the person can try again rather than losing what they were shown.
    gate.decision = Some(decision.to_string());
    gate.choice = choice.map(str::to_string);
    gate.comment = comment;
    let at = stamp();
    gate.answered_at = Some(at.clone());
    if let Some(handoff) = handoff {
        gate.set_handoff(handoff);
    }
    archive(ctx, &gate)?;
    drop(held);
    note_answered(ctx, gate.task.as_deref(), &at);
    Ok((gate, told))
}

/// What became of a gate that is no longer open, said as an error for whoever tried to
/// answer it.
fn already_decided(ctx: &Context, id: &str) -> Option<String> {
    let archived = load(&ctx.state, &ctx.repo.slug, Shelf::Answered, id).ok()?;
    Some(format!(
        "gate {id} was already answered or closed ({})",
        archived.decision.as_deref().unwrap_or("unknown")
    ))
}

/// Note the answer on the gate's task, if it has one.
pub(super) fn note_answered(ctx: &Context, task: Option<&str>, at: &str) {
    if let Some(id) = task {
        crate::task::note_gate_answered(ctx, id, at);
    }
}
