use crate::task::refresh as task_refresh;
use crate::transport::wording::refresh_json as task_refresh_json;
use serde_json::{Value, json};

use super::{cwd_property, hub_property, repo_property, resolve_repo};

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_refresh",
        "description": "Bring the task records up to date with their pull requests, the same as `adj task refresh`: every record with a `pr` that is not done or cancelled is read with `gh` in one query, its state is kept on the record, and the ones whose PR was merged are moved to done. A PR still open, one `gh` cannot read, or one closed without merging is left alone and listed instead (a closed one is the person's to decide, never cancelled here) — say those to the person rather than deciding for them. Each entry says whose `turn` it is. A hub calls this once at startup.",
        "inputSchema": {
            "type": "object",
            "properties": { "repo": repo_property(), "hub": hub_property(), "cwd": cwd_property() },
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let ctx = crate::registry::context_of(resolve_repo(args)?)?;
    let checked = task_refresh(&ctx)?;
    Ok(task_refresh_json(&checked))
}
