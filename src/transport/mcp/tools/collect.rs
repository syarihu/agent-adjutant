use serde_json::{Value, json};

use super::{cwd_param, cwd_property, repo_property, string_key, text_param};
use crate::task::collect;

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_collect",
        "description": "At hub startup and whenever a person asks for the task list (the Dashboard), and in a parent-task hub for what sits under its parent: collect the github and github-project task sources in one call, instead of sending a collection sub-agent for them. Returns the machine rows the collection briefs define plus `assignee` on task rows and `draft` / `ci` on pr rows (task / pr / worktree; with `parent`: parent / subtask / worktree), `failed` (lookups that failed: their rows are `?`, never `-`, and a failed list writes no rows), `leftOut` (repositories missing from issueKeys), `takenAt`, for the Dashboard `agentSources` (the jira / linear sources a collection sub-agent must still collect), and with `parent` `me` (who you are) and `unmatchedWorktrees` (worktrees that matched no subtask). With `parent`, a key that is not a github-family task of this repository (jira, linear, no match, several matches) is an error: collect a jira / linear parent with the collection sub-agent. Any other error means the parent, its sub-issue list or who you are could not be read: the collection stopped, so report the error and list nothing.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "parent": { "type": "string", "description": "The parent task's key (ALPHA-233). Leave it out for the Dashboard." },
                "cwd": cwd_property(),
                "repo": repo_property(),
            },
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    for key in ["cwd", "repo", "parent"] {
        string_key(args, key)?;
    }
    let cwd = cwd_param(args);
    let result = collect(
        cwd.as_deref(),
        text_param(args, "repo"),
        text_param(args, "parent"),
    )?;
    serde_json::to_value(&result).map_err(|e| format!("cannot serialize result: {e}"))
}
