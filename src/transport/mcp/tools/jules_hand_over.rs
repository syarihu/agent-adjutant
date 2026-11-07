use serde_json::{Value, json};

use crate::jules::hand_over;

use super::{cwd_property, hub_property, repo_property, resolve_repo, string_key, text_param};

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_jules_hand_over",
        "description": "Hand tasks to Jules, last step: once a person approved the plan gate of a task handed to Jules, instead of `adj jules start` and removing the worktree by hand. Checks the planning worktree is clean and detached (refuses, starting nothing, if not), starts the Jules session from `.claude/jules-plan.md`, deletes the plan files, removes the worktree, clears it from the record and runs `onWorktreeRemove`. `discard: true` removes it without starting anything (a rejected plan). Same as `adj jules hand-over`.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Task record id whose plan was approved."
                },
                "discard": {
                    "type": "boolean",
                    "description": "Remove the planning worktree without starting a Jules session (a rejected plan). Default: false."
                },
                "repo": repo_property(),
                "hub": hub_property(),
                "cwd": cwd_property(),
            },
            "required": ["id"],
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let ctx = crate::registry::context_of(resolve_repo(args)?)?;
    string_key(args, "id")?;
    let id = text_param(args, "id").ok_or_else(|| "id is required".to_string())?;
    // Refused rather than read as false: a `discard` that is not a boolean was meant to say
    // something, and false starts a Jules session from a plan the caller wanted thrown away.
    let discard = match args.get("discard") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(discard)) => *discard,
        Some(other) => return Err(format!("discard must be true or false, not {other}")),
    };

    let res = hand_over(&ctx, id, discard)?;
    serde_json::to_value(&res).map_err(|e| e.to_string())
}
