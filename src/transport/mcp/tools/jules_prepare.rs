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

    // Like a non-string `cwd` or `repo`, a target of the wrong shape is refused rather than
    // dropped: dropped, the call would prepare fewer tasks than it was asked for and say nothing.
    let list = |key: &str| -> Result<Vec<Value>, String> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(items)) => Ok(items.clone()),
            Some(_) => Err(format!("`{key}` must be an array")),
        }
    };
    let mut targets = Vec::new();
    for item in list("issues")? {
        match item {
            Value::Number(n) if n.as_u64().is_some() => {
                targets.push(PrepareTarget::Issue(n.to_string()))
            }
            Value::String(s) if !s.trim().is_empty() => {
                targets.push(PrepareTarget::Issue(s.trim().to_string()))
            }
            other => {
                return Err(format!(
                    "`issues` entries must be issue numbers or URLs, not {other}"
                ));
            }
        }
    }
    for item in list("ids")? {
        match item {
            Value::String(s) if !s.trim().is_empty() => {
                targets.push(PrepareTarget::Task(s.trim().to_string()))
            }
            other => return Err(format!("`ids` entries must be task ids, not {other}")),
        }
    }

    if targets.is_empty() {
        return Err("pass at least one issue or id".to_string());
    }

    // A base that is not a string is refused: read as absent it would fall back to
    // `baseBranch` and cut the worktree from a branch nobody named.
    let base = match args.get("base") {
        None | Some(Value::Null) => None,
        Some(Value::String(_)) => text_param(args, "base").map(str::to_string),
        Some(other) => return Err(format!("base must be a branch name, not {other}")),
    };
    let req = PrepareRequest { targets, base };

    let outcome = prepare(&ctx, &req);
    serde_json::to_value(&outcome).map_err(|e| e.to_string())
}
