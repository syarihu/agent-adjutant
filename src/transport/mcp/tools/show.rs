use serde_json::{Value, json};

use super::text_param;
use crate::board::aide::{ShowRequest, show};

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_show",
        "description": "Put something in the display area next to the aide's terminal on the page. kind=gate shows one gate (`board` = the gate's `slug` from adjutant_work, and `id`); kind=waits shows the list of everything that waits on the person. Showing something of another topic clears the area and keeps the old topic by name; something of the same topic is added on top, newest first. It writes nothing but the display area.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["gate", "waits"], "description": "What to show." },
                "board": { "type": "string", "description": "kind=gate: the `slug` of the gate's board." },
                "id": { "type": "string", "description": "kind=gate: the gate id." },
            },
            "required": ["kind"],
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let board = text_param(args, "board");
    let id = text_param(args, "id");
    let request = match text_param(args, "kind") {
        Some("gate") => ShowRequest::Gate {
            board: board
                .ok_or("kind=gate needs `board`, the slug of the gate's board")?
                .to_string(),
            id: id.ok_or("kind=gate needs `id`")?.to_string(),
        },
        Some("waits") => {
            if board.is_some() || id.is_some() {
                return Err("kind=waits takes no `board` or `id`".to_string());
            }
            ShowRequest::Waits
        }
        Some(_) | None => return Err("kind must be gate or waits".to_string()),
    };
    let shown = show(&crate::registry::state_root(None), request)?;
    let mut out = serde_json::to_value(&shown.display).map_err(|e| e.to_string())?;
    out["cleared"] = json!(shown.cleared);
    if shown.replaced_unreadable {
        out["replacedUnreadable"] = json!(true);
    }
    Ok(out)
}
