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
    crate::prompts::render_skill(&crate::prompts::SkillRequest {
        name,
        arguments,
        agent,
        client_name: None,
        runner_dir: None,
    })
    .map(|r| r.text)
    .map_err(|e| e.to_string())
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
