use crate::kernel::prompts;
use crate::transport::mcp::client_name;
use serde_json::{Value, json};

use super::string_key;

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_skill",
        "description": "One of adjutant's procedures: adj-hub (running the hub), adj-worker (taking a task from brief to handover), adj-report (handing a bug you found to the hub), adj-review-hub (the code-review hub), adj-reviewer (reading one pull request, read-only). Without section it returns the core: the role, the general rules and an index of triggers. Call it when you start as one of these, and again with section=<id> whenever the core's index names a section for what you are about to do or have just received; section=all is the whole text. Same text the MCP prompts serve.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "enum": ["adj-hub", "adj-worker", "adj-report", "adj-review-hub", "adj-reviewer"] },
                "arguments": { "type": "string", "description": "Free text substituted into the procedure where it asks for it." },
                "section": { "type": "string", "description": "Which part to return: core (default), all, or section ids separated by commas." },
                "agent": { "type": "string", "enum": ["claude", "claude-code", "agy", "antigravity", "generic", "codex"], "description": "Target agent format: claude (claude-code) | agy (antigravity) | generic (codex). Defaults to auto-detect." },
                "worktree": { "type": "string", "description": "Path to the worktree or repository root. Defaults to the server's working directory." },
            },
            "required": ["name"],
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    for key in ["worktree", "cwd"] {
        string_key(args, key)?;
    }
    let worktree = args["worktree"]
        .as_str()
        .or_else(|| args["cwd"].as_str())
        .filter(|s| !s.is_empty())
        .map(crate::infra::paths::expand_home);
    let client = client_name();
    let rendered = prompts::render_skill(&prompts::SkillRequest {
        name: args["name"].as_str().unwrap_or(""),
        arguments: args["arguments"].as_str().unwrap_or(""),
        section: args["section"].as_str(),
        agent: args["agent"].as_str(),
        client_name: client.as_deref(),
        runner_dir: worktree.as_deref(),
    })
    .map_err(|e| e.to_string())?;
    Ok(json!({
        "name": rendered.name,
        "agent": rendered.agent.as_str(),
        "description": rendered.description,
        "sections": rendered.sections,
        "content": rendered.text,
    }))
}
