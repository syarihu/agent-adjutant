use serde_json::{Value, json};

use super::{board_context, text_param};
use crate::gate::{DecideError, Decided, Gate, Offered};

const DECISIONS: [&str; 8] = [
    "approve", "changes", "reject", "ack", "ask", "answer", "choice", "close",
];

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_gate_answer",
        "description": "Answer a gate the way the board's buttons do: the same decision, choice and comment, by the same path as `POST /api/gates/<id>` and `adj gate answer`. The answer goes to the worker's outbox, or to the hub's inbox for a gate the hub opened (adjutant_gate_read says which as `answeredBy`). Call it ONLY after the person has said yes in the chat to exactly what will be sent: which gate, which decision or choice, and the comment. `decision` must be one the gate offers (for a waiting gate its `options`, `choice` when it has choices and `close`; for a record only `changes`); anything else is refused before anything is written. `decision=choice` needs `choice`, a choice id of the gate; with no `comment` it sends the choice's draft comment, as the button does. `close` archives the gate with nothing delivered. A record sent back (`changes`) needs a comment saying why. A gate somebody else already settled is reported and nothing is sent. `board` is the `slug` from adjutant_work and `id` the gate id.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "board": { "type": "string", "description": "The `slug` of the gate's board, from adjutant_work." },
                "id": { "type": "string", "description": "The gate id." },
                "decision": { "type": "string", "enum": DECISIONS, "description": "What the person decided." },
                "choice": { "type": "string", "description": "decision=choice: the id of the choice, `choices[].id` of adjutant_gate_read." },
                "comment": { "type": "string", "description": "The person's words to send with it. Blank sends the choice's draft (decision=choice) or nothing." },
            },
            "required": ["board", "id", "decision"],
        },
    })
}

/// A string key that is present must be a string: `choice: 1` is not "no choice".
fn string_key(args: &Value, key: &str) -> Result<(), String> {
    match &args[key] {
        Value::Null | Value::String(_) => Ok(()),
        _ => Err(format!("`{key}` must be a string")),
    }
}

/// What the answer carried as its comment: the gate's, or the last the record was sent back with.
fn sent_comment(gate: &Gate) -> Option<String> {
    match gate.wait {
        true => gate.comment.clone(),
        false => gate.answers.last().and_then(|a| a.comment.clone()),
    }
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    for key in ["board", "id", "decision", "choice", "comment"] {
        string_key(args, key)?;
    }
    let id = text_param(args, "id").ok_or("a gate needs an id")?;
    if !crate::task::is_plain_id(id) {
        return Err(format!("not a gate id: {id}"));
    }
    let decision = text_param(args, "decision").ok_or("a decision is required")?;
    if !DECISIONS.contains(&decision) {
        return Err(format!("decision must be one of {}", DECISIONS.join(", ")));
    }
    let choice = text_param(args, "choice");
    let comment = args["comment"].as_str();
    let ctx = board_context(args)?;
    // Who receives the answer, read before it is archived. A gate that cannot be read is left to
    // `decide`, which says whether it was settled or never there.
    let to = crate::gate::get(&ctx.state, &ctx.repo.slug, id)
        .map(|gate| {
            if gate.answered_by_hub() {
                "hub"
            } else {
                "worker"
            }
        })
        .unwrap_or("worker");
    let decided = crate::gate::decide(&ctx, id, decision, choice, comment, Offered::Only).map_err(
        |e| match e {
            DecideError::Settled(why) => format!("{why}: nothing was sent"),
            DecideError::Refused(why) => format!("{why}: nothing was sent"),
            DecideError::Failed(why) => why,
        },
    )?;
    let head = json!({
        "board": ctx.repo.slug,
        "repo": ctx.repo.nwo,
        "decision": decision,
        "choice": choice,
    });
    let mut out = head;
    match decided {
        Decided::Answered { gate: after, told } => {
            out["gate"] = json!({ "id": after.id, "kind": after.kind, "title": after.title, "task": after.task });
            out["comment"] = json!(sent_comment(&after));
            out["to"] = json!(to);
            out["present"] = json!(told.is_present());
            out["woken"] = json!(told.was_woken());
        }
        Decided::Closed {
            gate: after,
            already,
        } => {
            out["gate"] = json!({ "id": after.id, "kind": after.kind, "title": after.title, "task": after.task });
            out["comment"] = json!(after.comment);
            out["closed"] = json!(!already);
            if already {
                out["alreadySettled"] = json!(after.decision);
            }
        }
    }
    Ok(out)
}
