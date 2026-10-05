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

pub(crate) use crate::transport::wording::skill_text;

/// What the hub has left for the worker in this worktree.
pub fn outbox(worktree: Option<&str>, clear: bool) -> Result<(), String> {
    let worktree = match worktree {
        Some(path) => crate::infra::paths::expand_home(path),
        None => std::env::current_dir()
            .map_err(|e| format!("cannot determine the current directory: {e}"))?,
    };
    if clear {
        mail::clear_outbox(&worktree)?;
        println!("cleared the outbox");
        return Ok(());
    }
    let text = mail::read_outbox(&worktree).text;
    if text.trim().is_empty() {
        println!("(empty)");
        return Ok(());
    }
    print!("{text}");
    Ok(())
}
