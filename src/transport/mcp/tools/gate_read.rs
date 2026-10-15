use serde_json::{Value, json};

use super::text_param;

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_gate_read",
        "description": "One gate in full, open or kept as a record: every field the worker wrote, not the summary 「いまの仕事」 shows. Read only; it marks nothing as looked at. A gate is found by the `slug` of its board and its id, both as adjutant_work gives them: ids are unique only within a board.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "board": { "type": "string", "description": "The `slug` of the gate's board, from adjutant_work." },
                "id": { "type": "string", "description": "The gate id." },
            },
            "required": ["board", "id"],
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let board = text_param(args, "board").ok_or("a gate is found by its board: pass `board`")?;
    let id = text_param(args, "id").ok_or("a gate needs an id")?;
    let root = crate::registry::state_root(None);
    // The board first: `gate::get` builds a path from the slug, and only a board the address
    // book knows is one.
    let address = crate::registry::address_of(&root, board)
        .ok_or_else(|| format!("no such board: {board}"))?;
    if !crate::task::is_plain_id(id) {
        return Err(format!("not a gate id: {id}"));
    }
    let gate = crate::gate::get(&root, board, id)?;
    Ok(json!({
        "board": board,
        "repo": address.nwo,
        "hub": address.hub,
        "gate": gate,
    }))
}
