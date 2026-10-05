use crate::gate::close as gate_close_payload;
use serde_json::{Value, json};

use super::{cwd_property, hub_property, repo_property, resolve_repo};

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_gate_close",
        "description": "Archive an open gate without delivering an answer to the outbox, the same as `adj gate close`: used when the question was answered directly in the terminal tab or rendered moot, so the gate does not stay on the board waiting. When the person answered in the terminal, pass `terminal: true` and put what they decided in `comment`: the gate is then recorded as answered in the terminal.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The gate id to close." },
                "comment": { "type": "string", "description": "Optional reason for closing. With `terminal`, what the person decided." },
                "terminal": { "type": "boolean", "description": "The person answered this gate in your terminal: record it as answered in the terminal (put what they decided in comment)." },
                "repo": repo_property(),
                "hub": hub_property(),
                "cwd": cwd_property(),
            },
            "required": ["id"],
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let id = args["id"].as_str().ok_or("a gate needs an id")?;
    let comment = args.get("comment").and_then(Value::as_str);
    let ctx = crate::registry::context_of(resolve_repo(args)?)?;
    let terminal = args
        .get("terminal")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let gate = gate_close_payload(&ctx, id, comment, terminal)?;
    let on_board = gate.answered_on_board();
    Ok(json!({ "gate": gate, "closed": !on_board, "alreadyAnswered": on_board }))
}
