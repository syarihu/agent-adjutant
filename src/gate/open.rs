//! Opening a gate, or keeping one as a record.

use super::store::{claim_id, save, stamp};
use super::*;

use crate::registry::Context;

/// Write a gate down and say whether anybody is there to see it.
///
/// The second half of that answer is the whole reason this reports rather than just
/// succeeding: with no dashboard running, a gate is a message into a directory nobody
/// opens, and an agent that waited on one would wait for ever. The procedure branches on
/// it and falls back to asking in its own tab.
///
/// With `"wait": false` the gate is kept as a record instead: written beside the open queue
/// rather than in it, so it asks nobody for anything, and the caller goes on with its work.
pub fn open(ctx: &Context, request: GateRequest) -> Result<(Gate, bool), String> {
    let kind = request.kind;
    let has_task = request
        .task
        .as_deref()
        .is_some_and(|t| !t.trim().is_empty());

    // A dispatch gate asks whether to start a task, and a relay gate whether to pass a task's
    // review comments on; the answer to either is acted on by reading the task out of the
    // message. Without one the hub would be told "approve" and not what.
    if matches!(kind, Kind::Dispatch | Kind::Relay) && !has_task {
        return Err(format!(
            "a {} gate needs the task it asks about",
            kind.as_str()
        ));
    }

    // Who waits on the answer, for a plan: the hub opens one for a task handed to Jules.
    if request.opened_by == Opener::Hub {
        // Every other kind's opener follows from the kind; only a plan is opened by either.
        if kind != Kind::Plan {
            return Err(format!(
                "openedBy is for a plan; a {} gate's opener follows from its kind",
                kind.as_str()
            ));
        }
        // The hub is told which task an answer is about by the message alone, as with a
        // dispatch gate: the gate is archived by the time it reads it.
        if !has_task {
            return Err("a plan the hub opens needs the task it is for".to_string());
        }
    }

    if !request.wait && !kind.can_be_recorded() {
        return Err(format!(
            "a {} gate cannot be kept as a record; only diff and verify can",
            kind.as_str()
        ));
    }
    // A diff or verify gate that waits does so because a rule fired, and the board shows
    // which. One that names none leaves a person looking at a stop nobody can explain.
    if request.wait && kind.can_be_recorded() && request.stopped_by.is_empty() {
        return Err(format!(
            "a waiting {} gate needs stoppedBy; keep it as a record with \"wait\": false if no rule stopped it",
            kind.as_str()
        ));
    }
    // Why a gate stops only means something where it could have been a record instead, and
    // a record that says why it stopped the worker is one of the two statements being false.
    if !request.stopped_by.is_empty() {
        if !kind.can_be_recorded() {
            return Err(format!(
                "a {} gate always waits; stoppedBy is for diff and verify",
                kind.as_str()
            ));
        }
        if !request.wait {
            return Err("a record does not stop the worker; drop stoppedBy or wait".to_string());
        }
        for rule in &request.stopped_by {
            if !rule.applies_to(kind) {
                return Err(format!(
                    "{} cannot stop a {} gate",
                    rule.as_str(),
                    kind.as_str()
                ));
            }
        }
    }

    // The address the answer is delivered to, so an agent that mistypes it waits on an
    // outbox nobody writes to. The caller fills it in from where it stands when the payload
    // does not name one.
    let worktree = request
        .worktree
        .clone()
        .ok_or("not inside a worktree, and no --worktree was given")?;

    let stamp = stamp();
    let shelf = if request.wait {
        Shelf::Open
    } else {
        Shelf::Record
    };
    let id = claim_id(ctx, shelf, &stamp, kind)?;

    let GateRequest {
        kind,
        title,
        worktree: _,
        opened_by,
        task,
        wait,
        stopped_by,
        options,
        facts,
        focus,
        decided,
        unsure,
        body,
        run,
        diff,
        choices,
        rounds,
        problem,
        goal,
        review_rounds,
        findings,
        commands,
        manual,
    } = request;
    let gate = Gate {
        id,
        kind,
        worktree,
        opened_by,
        task,
        title,
        facts,
        focus,
        decided,
        unsure,
        body,
        run,
        diff,
        choices,
        options: options.unwrap_or_else(|| kind.default_options()),
        rounds,
        problem,
        goal,
        review_rounds,
        findings,
        commands,
        manual,
        stopped_by,
        wait,
        opened_at: stamp,
        // Written when it is answered, never brought in with the request.
        decision: None,
        choice: None,
        comment: None,
        answered_at: None,
        answers: Vec::new(),
        extra: serde_json::Map::new(),
    };

    save(ctx, shelf, &gate)?;
    Ok((
        gate,
        crate::registry::running(&ctx.state, &ctx.repo).is_some(),
    ))
}
