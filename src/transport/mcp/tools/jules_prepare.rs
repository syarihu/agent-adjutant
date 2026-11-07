use serde_json::{Value, json};

use crate::jules::{PrepareRequest, PrepareTarget, prepare};

use super::{cwd_property, hub_property, repo_property, resolve_repo, text_param};

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_jules_prepare",
        "description": "Hand tasks to Jules, step one: before its plan is written, instead of assigning the issue, cutting a worktree and writing the task record by hand. For each issue or task record it assigns a GitHub issue to you, decides the base (`baseBranch`), cuts a detached worktree, writes the record (implementer jules, up to a PR, stop at plan) and marks it dispatched, then returns what the planning sub-agent's brief needs: record, key, title, worktree, base, planFile, gateFile, verify. Several in one call; each that cannot be prepared is in `failed` with the reason. Same as `adj jules prepare`.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "issues": {
                    "type": "array",
                    "items": { "type": ["string", "integer"] },
                    "description": "GitHub issue numbers or URLs to prepare."
                },
                "ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task record ids that already exist to prepare."
                },
                "base": {
                    "type": "string",
                    "description": "Branching point on origin (default: the task's base or baseBranch)."
                },
                "repo": repo_property(),
                "hub": hub_property(),
                "cwd": cwd_property(),
            },
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let ctx = crate::registry::context_of(resolve_repo(args)?)?;

    let mut targets = Vec::new();
    if let Some(issues) = args.get("issues").and_then(Value::as_array) {
        for item in issues {
            if let Some(n) = item.as_i64() {
                targets.push(PrepareTarget::Issue(n.to_string()));
            } else if let Some(s) = item.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    targets.push(PrepareTarget::Issue(trimmed.to_string()));
                }
            }
        }
    }
    if let Some(ids) = args.get("ids").and_then(Value::as_array) {
        for item in ids {
            if let Some(s) = item.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    targets.push(PrepareTarget::Task(trimmed.to_string()));
                }
            }
        }
    }

    if targets.is_empty() {
        return Err("pass at least one issue or id".to_string());
    }

    let base = text_param(args, "base").map(str::to_string);
    let req = PrepareRequest { targets, base };

    let outcome = prepare(&ctx, &req);
    serde_json::to_value(&outcome).map_err(|e| e.to_string())
}
