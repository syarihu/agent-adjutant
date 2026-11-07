use serde_json::{Value, json};

use super::{cwd_param, cwd_property, repo_property, string_key, text_param};
use crate::task::check_pr_base;

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_pr_check_base",
        "description": "Right after opening a PR from a worktree, before asking anyone (Copilot or another bot) to review it: check that the PR's base matches the brief's Base branch. Reads the base from the worktree's `.claude/task-brief.md`, compares it with the PR's base and moves the PR there when they differ (same effect as `gh pr edit --base`). A PR whose head is not the worktree's branch is refused, so a wrong number cannot move someone else's PR. An error means the base could not be confirmed, so stop and tell the user instead of requesting a review. Use it instead of `gh pr view --json baseRefName` / `gh pr edit --base`.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "pr": {
                    "type": "integer",
                    "description": "The pull request number."
                },
                "cwd": cwd_property(),
                "repo": repo_property(),
            },
            "required": ["pr"],
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let pr = match args.get("pr") {
        Some(Value::Number(n)) => n.as_u64().ok_or("`pr` must be an integer")?,
        Some(_) => return Err("`pr` must be an integer".to_string()),
        None => return Err("pass `pr`: the pull request number".to_string()),
    };
    for key in ["cwd", "repo"] {
        string_key(args, key)?;
    }
    let cwd = cwd_param(args);
    let repo = text_param(args, "repo");
    let result = check_pr_base(cwd.as_deref(), pr, repo)?;
    serde_json::to_value(&result).map_err(|e| format!("cannot serialize result: {e}"))
}
