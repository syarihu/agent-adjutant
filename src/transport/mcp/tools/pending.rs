use crate::mail;
use serde_json::{Value, json};
use std::path::Path;

use super::resolve_repo;

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let info = resolve_repo(args)?;
    let root = crate::registry::state_root(Some(Path::new(&info.main)));
    let action = args["action"].as_str().unwrap_or("list");
    match action {
        "list" => {
            let pending = mail::pending(&root, &info.slug);
            let entries: Vec<Value> = pending
                .messages
                .into_iter()
                .map(|e| {
                    json!({"name": e.name, "from": e.from, "worktree": e.worktree,
                           "kind": e.kind, "subject": e.subject})
                })
                .collect();
            Ok(json!({
                "hubName": info.hub_name,
                "dir": pending.dir.to_string_lossy(),
                "count": entries.len(),
                "messages": entries,
            }))
        }
        "read" => {
            let name = args["name"].as_str().unwrap_or("");
            Ok(json!({ "name": name, "content": mail::read(&root, &info.slug, name)? }))
        }
        "ack" => {
            let name = args["name"].as_str().unwrap_or("");
            let moved = mail::ack(&root, &info.slug, name)?;
            Ok(json!({ "name": name, "archived": moved.to_string_lossy() }))
        }
        other => Err(format!("action must be list, read or ack: {other}")),
    }
}
