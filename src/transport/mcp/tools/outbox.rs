use crate::mail;
use serde_json::{Value, json};

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
