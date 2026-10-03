use super::*;

/// Print one of the procedures.
///
/// The same text the MCP prompt and `adjutant_skill` serve. It exists as a subcommand
/// because a shell command is the one way in that every agent has: MCP prompt support
/// differs between agents, and a tool has to be loaded before it can be called — a woken
/// session that cannot reach its procedure is a session that does nothing.
pub fn skill(name: &str, arguments: &str, agent: Option<&str>) -> Result<(), String> {
    print!("{}", skill_text(name, arguments, agent)?);
    Ok(())
}

/// The text `skill` prints, rendered for the agent the override or the configured runner names.
pub(crate) fn skill_text(
    name: &str,
    arguments: &str,
    agent: Option<&str>,
) -> Result<String, String> {
    let prompt = crate::prompts::find(name).ok_or_else(|| {
        format!(
            "no such procedure: {name} ({})",
            crate::prompts::PROMPTS
                .iter()
                .map(|p| p.name)
                .collect::<Vec<_>>()
                .join(" / ")
        )
    })?;
    let runner = crate::mcp::resolve_runner_for(None, name);
    let runner_agent = runner.as_deref().map(crate::runner::agent_from_runner);
    let resolved_agent = crate::prompts::resolve_agent(agent, None, runner_agent.as_deref());
    Ok(crate::prompts::render_for(
        prompt,
        arguments,
        resolved_agent,
    ))
}

/// What the hub has left for the worker in this worktree.
pub fn outbox(worktree: Option<&str>, clear: bool) -> Result<(), String> {
    let worktree = match worktree {
        Some(path) => config::expand_home(path),
        None => std::env::current_dir()
            .map_err(|e| format!("cannot determine the current directory: {e}"))?,
    };
    if clear {
        messaging::clear_outbox(&worktree)?;
        println!("cleared the outbox");
        return Ok(());
    }
    let text = messaging::read_outbox(&worktree);
    if text.trim().is_empty() {
        println!("(empty)");
        return Ok(());
    }
    print!("{text}");
    Ok(())
}
