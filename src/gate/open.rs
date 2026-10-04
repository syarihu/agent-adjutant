//! Opening a gate, or keeping one as a record.

use serde_json::{Value, json};

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
pub fn open(ctx: &Context, payload: &Value) -> Result<(Gate, bool), String> {
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("a gate needs a kind")?;
    let kind: Kind =
        serde_json::from_value(json!(kind)).map_err(|_| format!("no such gate kind: {kind}"))?;
    // Checked before deserialising so the error names the field, rather than arriving as
    // a serde message about a struct the caller never saw.
    if payload
        .get("title")
        .and_then(Value::as_str)
        .is_none_or(|t| t.trim().is_empty())
    {
        return Err("a gate needs a title".to_string());
    }

    // A dispatch gate asks whether to start a task, and a relay gate whether to pass a task's
    // review comments on; the answer to either is acted on by reading the task out of the
    // message. Without one the hub would be told "approve" and not what.
    if matches!(kind, Kind::Dispatch | Kind::Relay)
        && payload
            .get("task")
            .and_then(Value::as_str)
            .is_none_or(|t| t.trim().is_empty())
    {
        return Err(format!(
            "a {} gate needs the task it asks about",
            kind.as_str()
        ));
    }

    // Who waits on the answer, for a plan: the hub opens one for a task handed to Jules. Read
    // before the id is claimed, for the reason given for `stoppedBy` below.
    let opener: Opener = match payload.get("openedBy") {
        None | Some(Value::Null) => Opener::Worker,
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|_| format!("no such opener: {value} (worker or hub)"))?,
    };
    if opener == Opener::Hub {
        // Every other kind's opener follows from the kind; only a plan is opened by either.
        if kind != Kind::Plan {
            return Err(format!(
                "openedBy is for a plan; a {} gate's opener follows from its kind",
                kind.as_str()
            ));
        }
        // The hub is told which task an answer is about by the message alone, as with a
        // dispatch gate: the gate is archived by the time it reads it.
        if payload
            .get("task")
            .and_then(Value::as_str)
            .is_none_or(|t| t.trim().is_empty())
        {
            return Err("a plan the hub opens needs the task it is for".to_string());
        }
    }

    let wait = match payload.get("wait") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(wait)) => *wait,
        Some(_) => return Err("wait must be true or false".to_string()),
    };
    if !wait && !kind.can_be_recorded() {
        return Err(format!(
            "a {} gate cannot be kept as a record; only diff and verify can",
            kind.as_str()
        ));
    }
    // `null` too: serde's default covers a missing field, not a present one of the wrong
    // type, and a refusal from serde comes after the id is claimed.
    if payload.get("stoppedBy").is_some_and(|v| !v.is_array()) {
        return Err("stoppedBy must be a list of rules".to_string());
    }
    // A diff or verify gate that waits does so because a rule fired, and the board shows
    // which. One that names none leaves a person looking at a stop nobody can explain.
    if wait
        && kind.can_be_recorded()
        && payload
            .get("stoppedBy")
            .and_then(Value::as_array)
            .is_none_or(|rules| rules.is_empty())
    {
        return Err(format!(
            "a waiting {} gate needs stoppedBy; keep it as a record with \"wait\": false if no rule stopped it",
            kind.as_str()
        ));
    }
    // Why a gate stops only means something where it could have been a record instead, and
    // a record that says why it stopped the worker is one of the two statements being false.
    if payload
        .get("stoppedBy")
        .and_then(Value::as_array)
        .is_some_and(|rules| !rules.is_empty())
    {
        if !kind.can_be_recorded() {
            return Err(format!(
                "a {} gate always waits; stoppedBy is for diff and verify",
                kind.as_str()
            ));
        }
        if !wait {
            return Err("a record does not stop the worker; drop stoppedBy or wait".to_string());
        }
        // Read here rather than left to serde below, which runs after the id is claimed and
        // would leave an empty gate file behind for an unknown rule.
        for rule in &payload["stoppedBy"].as_array().cloned().unwrap_or_default() {
            let parsed: StopRule = serde_json::from_value(rule.clone())
                .map_err(|_| format!("no such stop rule: {rule}"))?;
            if !parsed.applies_to(kind) {
                return Err(format!(
                    "{} cannot stop a {} gate",
                    parsed.as_str(),
                    kind.as_str()
                ));
            }
        }
    }

    // The payload may name the worktree; otherwise it is derived from where the caller
    // stands. This is the address the answer is delivered to, so an agent that mistypes it
    // waits on an outbox nobody writes to.
    let worktree = match payload.get("worktree").and_then(Value::as_str) {
        Some(path) => path.to_string(),
        None => crate::kernel::identity::current_worktree(None)
            .ok_or("not inside a worktree, and no --worktree was given")?,
    };

    let stamp = stamp();
    let shelf = if wait { Shelf::Open } else { Shelf::Record };
    let id = claim_id(ctx, shelf, &stamp, kind)?;

    let mut value = payload.clone();
    let fields = value.as_object_mut().ok_or("expected an object")?;
    fields.insert("id".to_string(), json!(id));
    fields.insert("worktree".to_string(), json!(worktree));
    fields.insert("openedAt".to_string(), json!(stamp));
    fields
        .entry("options")
        .or_insert(json!(kind.default_options()));
    fields.insert("wait".to_string(), json!(wait));
    // `null` read as the worker above, and serde's default covers only a missing field.
    if fields.get("openedBy").is_some_and(Value::is_null) {
        fields.remove("openedBy");
    }
    // A record's answers are appended by whoever answers it, never brought in with it.
    fields.remove("answers");
    let mut gate: Gate = serde_json::from_value(value).map_err(|e| format!("bad gate: {e}"))?;
    // Only keys read from disk are carried; a caller's unknown keys are dropped, as before.
    gate.extra.clear();

    save(ctx, shelf, &gate)?;
    Ok((
        gate,
        crate::registry::running(&ctx.state, &ctx.repo).is_some(),
    ))
}
