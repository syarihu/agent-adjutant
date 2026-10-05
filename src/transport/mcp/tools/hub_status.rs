use crate::mail;
use crate::registry;
use serde_json::{Value, json};
use std::path::Path;

use super::resolve_repo;

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let info = resolve_repo(args)?;
    let root = crate::registry::state_root(Some(Path::new(&info.main)));
    let status = registry::hub_status(&root, &info.slug, &info.hub_name);
    let pending = mail::pending(&root, &info.slug);
    let mut out = status_json(&status, &pending.dir);
    out["repo"] = json!(info.nwo);
    out["hub"] = json!(info.hub);
    out["main"] = json!(info.main);
    out["waiting"] = json!(pending.messages.len());
    Ok(out)
}

fn status_json(status: &registry::HubStatus, inbox: &Path) -> Value {
    json!({
        "hubName": status.hub_name,
        "slug": status.slug,
        "present": status.present,
        "stale": status.stale,
        "pid": status.pid,
        "cwd": status.cwd,
        "startedAt": status.started_at,
        "inbox": inbox.to_string_lossy(),
    })
}
