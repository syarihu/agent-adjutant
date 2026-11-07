use serde_json::{Value, json};

use super::{cwd_param, cwd_property, repo_property, resolve_repo, string_key, text_param};
use crate::board::worktree;

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_worktree",
        "description": "Whenever a worktree is made, listed or cleaned up, instead of `git worktree` (or `proctor worktree`) directly. Actions: `add` creates a linked worktree and its branch on the resolved base and runs postCreate hooks; `ls` lists all linked worktrees (leaving out the main checkout) with their git state, tasks, and removal blockers; `rm` safely removes a worktree and closes any running session, but refuses — removing nothing — whenever uncommitted, untracked or unpushed work would be lost, and never forces.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "ls", "rm"],
                    "description": "The operation to perform: add, ls, or rm."
                },
                "name": {
                    "type": "string",
                    "description": "Worktree name (e.g. app-1234), used for branch naming and hooks."
                },
                "user": {
                    "type": "string",
                    "description": "Branch owner (default: $USER)."
                },
                "pattern": {
                    "type": "string",
                    "description": "The task source's branchPattern."
                },
                "branch": {
                    "type": "string",
                    "description": "Branch name to create and check out (for add), or expected branch (for rm)."
                },
                "path": {
                    "type": "string",
                    "description": "Path for the worktree to add or remove."
                },
                "base": {
                    "type": "string",
                    "description": "Branching point on origin (default: repository baseBranch or auto)."
                },
                "cwd": cwd_property(),
                "repo": repo_property(),
            },
            "required": ["action"],
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    for key in [
        "action", "name", "user", "pattern", "branch", "path", "base", "cwd", "repo",
    ] {
        string_key(args, key)?;
    }

    let action = match args.get("action").and_then(Value::as_str) {
        Some("add") => "add",
        Some("ls") => "ls",
        Some("rm") => "rm",
        Some(other) => {
            return Err(format!(
                "unknown action '{other}': must be one of add, ls, rm"
            ));
        }
        None => return Err("pass `action`: add, ls, or rm".to_string()),
    };

    let repo_info = resolve_repo(args)?;
    let ctx = crate::registry::context_of(repo_info)?;

    match action {
        "add" => {
            let req = worktree::AddRequest {
                name: text_param(args, "name").map(str::to_string),
                user: text_param(args, "user").map(str::to_string),
                pattern: text_param(args, "pattern").map(str::to_string),
                branch: text_param(args, "branch").map(str::to_string),
                path: text_param(args, "path").map(str::to_string),
                base: text_param(args, "base").map(str::to_string),
            };
            let outcome = worktree::add(&ctx, req)?;
            serde_json::to_value(&outcome).map_err(|e| format!("cannot serialize outcome: {e}"))
        }
        "ls" => {
            let outcome = worktree::list(&ctx)?;
            serde_json::to_value(&outcome).map_err(|e| format!("cannot serialize outcome: {e}"))
        }
        "rm" => {
            let Some(path) = text_param(args, "path") else {
                return Err("pass `path`: the worktree path to remove".to_string());
            };
            let req = worktree::RemoveRequest {
                path: path.to_string(),
                branch: text_param(args, "branch").map(str::to_string),
                cwd: cwd_param(args),
            };
            let outcome = worktree::remove(&ctx, req)?;
            serde_json::to_value(&outcome).map_err(|e| format!("cannot serialize outcome: {e}"))
        }
        _ => unreachable!(),
    }
}
