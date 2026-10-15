//! Deciding a gate the way the board's buttons do: the one operation behind
//! `POST /api/gates/<id>` and the aide's `adjutant_gate_answer`.
//!
//! What a decision comes to — closing, or answering, with the comment that goes with it — was
//! spread over the page (the comment a choice sends) and the handler (close against answer). It
//! is here so that a second caller (the aide, and later an answer to a wait from its row) sends
//! exactly what the buttons send. The board's route (`Offered::Any`) behaves as it always did and
//! only works out the comment; the stricter refusals, all made before anything is written, are for
//! the aide's route (`Offered::Only`).

use super::answer::answer_with;
use super::*;

use crate::registry::Context;

/// How a decision is held to what the gate offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offered {
    /// Only what the gate's own buttons are: its options, a choice when it has choices, closing a
    /// waiting gate, and `changes` for a record, with every refusal made before anything is
    /// written. For a caller that picks a decision from words.
    Only,
    /// Nothing is refused here that the plain `answer` and `close` accepted: the comment is worked
    /// out and the rest is left to them. The board's route, whose cards pick a decision by column
    /// and not from the gate's options.
    Any,
}

/// What `decide` did.
#[derive(Debug)]
pub enum Decided {
    /// The ball was handed back: delivered to the hub or the worker.
    Answered {
        gate: Gate,
        told: crate::mail::DeliveryOutcome,
    },
    /// The gate was closed with nothing delivered. `already` is for a gate that had been settled
    /// before: the archived gate is what comes back, and nothing was written.
    Closed { gate: Gate, already: bool },
}

/// Why `decide` did not decide.
#[derive(Debug, PartialEq, Eq)]
pub enum DecideError {
    /// Somebody else settled the gate first; the words say how.
    Settled(String),
    /// The decision is not one this gate takes. Nothing was written.
    Refused(String),
    /// Something failed.
    Failed(String),
}

impl DecideError {
    pub fn message(&self) -> &str {
        match self {
            DecideError::Settled(why) | DecideError::Refused(why) | DecideError::Failed(why) => why,
        }
    }
}

/// Whether `decision` closes the gate rather than answering it.
pub fn closes(decision: &str) -> bool {
    matches!(decision, "close" | "dismiss")
}

/// The decisions `gate` takes: what its buttons are. The options as they are, as the page draws
/// them: opening a gate fills in the kind's defaults when none were given, so an empty list is a
/// gate with no option buttons (only its choices, or only closing).
pub fn offered(gate: &Gate) -> Vec<String> {
    // A record is no longer open: the page offers only the send-back form, whatever its options
    // say, and there is nothing to close.
    if !gate.wait {
        return vec!["changes".to_string()];
    }
    let mut out = gate.options.clone();
    if !gate.choices.is_empty() {
        out.push("choice".to_string());
    }
    out.push("close".to_string());
    out
}

/// The comment a decision sends: what was typed, else the draft of the choice picked (for
/// `choice` only). Under `Offered::Only` it also refuses, before anything is written, what the
/// gate does not take: a decision it does not offer, a choice id with any decision but `choice`
/// or one the gate lacks, `choice` with no id, a record answered with anything but `changes`, and
/// a record sent back with no words. Under `Offered::Any` it refuses nothing: the board's route
/// must accept whatever the old `answer` and `close` accepted, and `answer_with` still refuses
/// what it always refused.
pub fn check(
    gate: &Gate,
    decision: &str,
    choice: Option<&str>,
    comment: Option<&str>,
    offered_only: Offered,
) -> Result<Option<String>, String> {
    let drafted = match (decision, choice) {
        ("choice", Some(id)) => gate
            .choices
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| typed(c.comment.as_deref())),
        _ => None,
    };
    let sent = match closes(decision) {
        true => typed(comment),
        false => typed(comment).or(drafted),
    };
    if offered_only == Offered::Any {
        return Ok(sent);
    }
    let takes = offered(gate);
    let named = if decision == "dismiss" {
        "close"
    } else {
        decision
    };
    if !takes.iter().any(|o| o == named) {
        return Err(format!(
            "this gate does not take {decision}: it takes {}",
            takes.join(", ")
        ));
    }
    if closes(decision) {
        if let Some(id) = choice {
            return Err(format!(
                "a choice ({id}) goes only with decision=choice, not {decision}"
            ));
        }
        return Ok(sent);
    }
    // Nothing else means anything to a worker that is not waiting.
    if !gate.wait && (decision != "changes" || choice.is_some()) {
        return Err(format!(
            "a record can only be answered with changes, not {decision}{}",
            if choice.is_some() {
                " and a choice"
            } else {
                ""
            }
        ));
    }
    match (decision, choice) {
        ("choice", None) => return Err("a choice needs `choice`: which one".to_string()),
        ("choice", Some(id)) if !gate.choices.iter().any(|c| c.id == id) => {
            return Err(format!("no such choice on this gate: {id}"));
        }
        ("choice", Some(_)) | (_, None) => {}
        (_, Some(id)) => {
            return Err(format!(
                "a choice ({id}) goes only with decision=choice, not {decision}"
            ));
        }
    }
    // Sending back something the worker has moved past, without saying what is wrong, gives it
    // nothing to act on.
    if !gate.wait && sent.is_none() {
        return Err("write why in the comment: a record sent back needs it".to_string());
    }
    Ok(sent)
}

fn typed(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// Decide the gate `id`: close it, or answer it with `decision`, `choice` and the comment (see
/// `check`). Everything is checked before anything is written, so a refusal leaves the gate open
/// and the outbox as it was.
pub fn decide(
    ctx: &Context,
    id: &str,
    decision: &str,
    choice: Option<&str>,
    comment: Option<&str>,
    offered_only: Offered,
) -> Result<Decided, DecideError> {
    let read = get(&ctx.state, &ctx.repo.slug, id);
    if closes(decision) {
        let sent = match &read {
            Ok(gate) => check(gate, decision, choice, comment, offered_only)
                .map_err(DecideError::Refused)?,
            Err(_) => typed(comment),
        };
        // Open when it can still be read as one that waits: an archived gate is not read by `get`.
        let already = !matches!(&read, Ok(gate) if gate.wait);
        let gate = close(ctx, id, sent.as_deref(), false).map_err(DecideError::Failed)?;
        return Ok(Decided::Closed { gate, already });
    }
    let sent = match &read {
        Ok(gate) => {
            check(gate, decision, choice, comment, offered_only).map_err(DecideError::Refused)?
        }
        // Not open and not a record: `answer_with` says whether it was settled or never there.
        Err(_) => typed(comment),
    };
    match answer_with(ctx, id, decision, choice, sent.as_deref(), None) {
        Ok((gate, told)) => Ok(Decided::Answered { gate, told }),
        Err(e) if e.1 => Err(DecideError::Settled(e.0)),
        Err(e) => Err(DecideError::Failed(e.0)),
    }
}
