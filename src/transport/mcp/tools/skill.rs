use crate::kernel::prompts;
use crate::transport::mcp::client_name;
use serde_json::{Value, json};

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_skill",
        "description": "The full text of one of adjutant's procedures: adj-hub (running the hub), adj-worker (taking a task from brief to handover), adj-report (handing a bug you found to the hub). Same text the MCP prompts serve; use this tool when prompts are not available to you.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "enum": ["adj-hub", "adj-worker", "adj-report"] },
                "arguments": { "type": "string", "description": "Free text substituted into the procedure where it asks for it." },
                "agent": { "type": "string", "enum": ["claude", "claude-code", "agy", "antigravity", "generic", "codex"], "description": "Target agent format: claude (claude-code) | agy (antigravity) | generic (codex). Defaults to auto-detect." },
                "worktree": { "type": "string", "description": "Path to the worktree or repository root. Defaults to the server's working directory." },
            },
            "required": ["name"],
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let worktree = args["worktree"]
        .as_str()
        .or_else(|| args["cwd"].as_str())
        .filter(|s| !s.is_empty())
        .map(crate::infra::paths::expand_home);
    let client = client_name();
    let rendered = prompts::render_skill(&prompts::SkillRequest {
        name: args["name"].as_str().unwrap_or(""),
        arguments: args["arguments"].as_str().unwrap_or(""),
        agent: args["agent"].as_str(),
        client_name: client.as_deref(),
        runner_dir: worktree.as_deref(),
    })
    .map_err(|e| e.to_string())?;
    Ok(json!({
        "name": rendered.name,
        "agent": rendered.agent.as_str(),
        "description": rendered.description,
        "content": rendered.text,
    }))
}
