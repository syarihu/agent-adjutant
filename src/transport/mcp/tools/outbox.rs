use crate::mail;
use serde_json::{Value, json};

use super::cwd_property;

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_outbox",
        "description": "What the hub has left for the worker in a worktree. action=read (default) returns everything waiting, action=clear says it has all been dealt with. A worker reads this every time it comes back from asking the user — that is where messages pile up.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["read", "clear"], "description": "Default: read." },
                "worktree": { "type": "string", "description": "Default: the server's working directory." },
                "cwd": cwd_property(),
            },
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let worktree = match args["worktree"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| args["cwd"].as_str())
        .filter(|s| !s.is_empty())
    {
        Some(path) => crate::infra::paths::expand_home(path),
        None => std::env::current_dir()
            .map_err(|e| format!("cannot determine the current directory: {e}"))?,
    };
    match args["action"].as_str().unwrap_or("read") {
        "read" => {
            let outbox = mail::read_outbox(&worktree);
            Ok(json!({
                "path": outbox.path.to_string_lossy(),
                "empty": outbox.text.trim().is_empty(),
                "content": outbox.text,
            }))
        }
        "clear" => {
            mail::clear_outbox(&worktree)?;
            Ok(json!({ "cleared": true }))
        }
        other => Err(format!("action must be read or clear: {other}")),
    }
}
