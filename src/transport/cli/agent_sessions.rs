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
    let rows = registry::agent_sessions(&root)?;
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

/// Text from a hook payload, with control characters written out (`\u{1b}`): a tool's input is
/// whatever a model or a file put there, and an escape sequence must not reach the terminal.
fn plain(text: &str) -> String {
    text.chars()
        .map(|c| match c.is_control() {
            true => c.escape_default().to_string(),
            false => c.to_string(),
        })
        .collect()
}

/// One row as one line: status, sub-agents, how long in that status, the session, where, and
/// what it is doing or asking, or else the first line of what it said last.
fn line(row: &AgentSession, now: i64) -> String {
    let status = plain(row.status.as_ref().map_or("-", |status| status.as_str()));
    let subagents = match row.subagents.len() {
        0 => String::new(),
        n => format!("+{n} sub"),
    };
    let since = match row.updated_at.or(row.last_event_at) {
        Some(at) => ago((now - at).max(0)),
        None => "-".to_string(),
    };
    let place = plain(
        row.worktree
            .as_deref()
            .or(row.cwd.as_deref())
            .unwrap_or("-"),
    );
    let waiting = row.status == Some(registry::AgentStatus::Waiting);
    let mut detail = match waiting {
        true => row.request.clone(),
        false => row.activity.clone(),
    }
    .map(|text| plain(&text))
    .unwrap_or_default();
    // A turn that is over has no tool and asks for nothing: what it said last is the detail.
    if detail.is_empty()
        && let Some(first) = row
            .last_message
            .as_deref()
            .and_then(|message| message.lines().find(|line| !line.trim().is_empty()))
    {
        detail = plain(first.trim());
    }
    if let Some(pending) = &row.pending_status {
        detail.push_str(&format!(" ({} held)", plain(pending.as_str())));
    }
    format!(
        "{status:<8} {subagents:<7} {since:>12}  {}  {place}  {detail}",
        plain(&row.session_id)
    )
    .trim_end()
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::AgentStatus;

    #[test]
    fn what_a_payload_put_in_a_row_cannot_reach_the_terminal_as_control_characters() {
        let row = AgentSession {
            session_id: "s\u{7}1".to_string(),
            status: Some(AgentStatus::Other("x\u{1b}[2J".to_string())),
            pending_status: Some(AgentStatus::Other("p\u{9b}".to_string())),
            cwd: Some("/home/user/\u{1b}]0;title\u{7}".to_string()),
            activity: Some("Bash: echo \u{1b}[31m\nred".to_string()),
            ..AgentSession::default()
        };
        let text = line(&row, 100);
        assert!(!text.chars().any(char::is_control), "{text:?}");
        assert!(text.contains("\\u{1b}[31m\\nred"), "{text}");
        assert!(text.contains("s\\u{7}1"), "{text}");
        assert!(text.contains("/home/user/\\u{1b}]0;title\\u{7}"), "{text}");
    }

    #[test]
    fn the_last_message_is_the_detail_only_when_there_is_no_other() {
        let row = AgentSession {
            status: Some(AgentStatus::Done),
            last_message: Some("\nAll done \u{1b}[0m\nsecond".to_string()),
            ..AgentSession::default()
        };
        let text = line(&row, 100);
        assert!(text.ends_with("All done \\u{1b}[0m"), "{text}");
        assert!(!text.contains("second"), "{text}");
        let busy = AgentSession {
            activity: Some("Bash: ls".to_string()),
            ..row
        };
        assert!(line(&busy, 100).ends_with("Bash: ls"));
    }
}
