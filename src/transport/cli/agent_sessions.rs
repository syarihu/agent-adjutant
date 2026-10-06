//! `adj agent-sessions`: what each agent session's hooks last reported.

use serde_json::json;

use super::args::AgentSessionsArgs;
use super::hub::ago;
use crate::infra::clock::now_secs;
use crate::registry::{self, AgentSession};

pub fn run(args: &AgentSessionsArgs) -> Result<(), String> {
    // The root is found the way the hook finds it, and no config is read: the ledger is one per
    // machine, not one per hub.
    let cwd = std::env::current_dir().ok();
    let root = registry::hook_state_root(cwd.as_deref());
    let rows = registry::agent_sessions(&root);
    if args.json {
        println!("{}", json!(rows));
        return Ok(());
    }
    if rows.is_empty() {
        println!("No agent sessions.");
        return Ok(());
    }
    let now = now_secs();
    for row in &rows {
        println!("{}", line(row, now));
    }
    Ok(())
}

/// One row as one line: status, sub-agents, how long in that status, the session, where, and
/// what it is doing or asking.
fn line(row: &AgentSession, now: i64) -> String {
    let status = row.status.as_ref().map_or("-", |status| status.as_str());
    let subagents = match row.subagents.len() {
        0 => String::new(),
        n => format!("+{n} sub"),
    };
    let since = match row.updated_at.or(row.last_event_at) {
        Some(at) => ago((now - at).max(0)),
        None => "-".to_string(),
    };
    let place = row
        .worktree
        .as_deref()
        .or(row.cwd.as_deref())
        .unwrap_or("-");
    let waiting = row.status == Some(registry::AgentStatus::Waiting);
    let mut detail = match waiting {
        true => row.request.clone(),
        false => row.activity.clone(),
    }
    .unwrap_or_default();
    if let Some(pending) = &row.pending_status {
        detail.push_str(&format!(" ({} held)", pending.as_str()));
    }
    format!(
        "{status:<8} {subagents:<7} {since:>12}  {}  {place}  {detail}",
        row.session_id
    )
    .trim_end()
    .to_string()
}
