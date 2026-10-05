use crate::kernel::prompts;
use crate::transport::mcp::client_name;
use serde_json::{Value, json};

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
