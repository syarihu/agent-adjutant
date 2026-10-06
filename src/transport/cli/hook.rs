//! `adj hook <agent>`: the receiver an agent's hooks run. It reads one payload from stdin, hands
//! the event to the session ledger, and never fails the agent: whatever goes wrong is said on
//! stderr and the exit code is 0.
//!
//! The payload's shape is Claude Code's, so it is read here and `registry` is handed a typed
//! `AgentEvent`; `registry` holds who is running and where, not what an agent's JSON looks like.

use serde_json::Value;
use std::io::Read;
use std::panic::AssertUnwindSafe;
use std::path::Path;

use super::args::HookArgs;
use crate::infra::clock::now_secs;
use crate::infra::env::{CLAUDE_CONFIG_DIR_ENV, CLAUDE_PID_ENV};
use crate::infra::paths::home_dir;
use crate::registry::{self, AgentEvent, HookEvent};

/// What is shown of a tool call or a notification: enough to tell what is going on, and a
/// bounded size for the row that keeps it.
const SHOWN_CHARS: usize = 80;

/// A line on stderr that cannot panic: `eprintln!` does on a closed stderr, outside `catch_unwind`.
fn say(message: std::fmt::Arguments) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{message}");
}

/// Always 0. Claude Code treats exit 2 as blocking and any other non-zero exit as a hook error.
pub fn hook(args: &HookArgs) -> i32 {
    if args.agent != "claude" {
        say(format_args!(
            "adjutant hook: unknown agent {:?}",
            args.agent
        ));
        return 0;
    }
    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        say(format_args!("adjutant hook: cannot read stdin: {e}"));
        return 0;
    }
    let payload: Value = match serde_json::from_str(&input) {
        Ok(payload) => payload,
        Err(e) => {
            say(format_args!("adjutant hook: the payload is not JSON: {e}"));
            return 0;
        }
    };
    let permission =
        payload.get("hook_event_name").and_then(Value::as_str) == Some("PermissionRequest");
    match std::panic::catch_unwind(AssertUnwindSafe(|| record(&payload))) {
        Ok(Ok(())) => {}
        Ok(Err(message)) => say(format_args!("adjutant hook: {message}")),
        Err(_) => say(format_args!(
            "adjutant hook: panicked while recording the event"
        )),
    }
    // The one event whose answer the agent reads: `{}` leaves the decision to its own prompt.
    // Said even when recording failed, or the failure would be the agent's.
    if permission {
        // Not `println!`, which panics on a closed stdout, outside the `catch_unwind`.
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{{}}");
        let _ = out.flush();
    }
    0
}

fn record(payload: &Value) -> Result<(), String> {
    let event = claude_event(
        payload,
        std::env::var(CLAUDE_PID_ENV).ok().as_deref(),
        std::env::var(CLAUDE_CONFIG_DIR_ENV).ok().as_deref(),
        &home_dir(),
        now_secs(),
    )?;
    let root = registry::hook_state_root(event.cwd.as_deref().map(Path::new));
    registry::record_agent_event(&root, &event)
}

/// A Claude Code hook payload as the event it reports. `pid` and `config_dir` are the hook's
/// `CLAUDE_PID` and `CLAUDE_CONFIG_DIR`, handed in so a test answers for the values it chose.
fn claude_event(
    payload: &Value,
    pid: Option<&str>,
    config_dir: Option<&str>,
    home: &Path,
    at: i64,
) -> Result<AgentEvent, String> {
    let session_id = text(payload, "session_id").unwrap_or_default();
    if !registry::valid_session_id(&session_id) {
        return Err(format!("not a usable session id: {session_id:?}"));
    }
    let name = text(payload, "hook_event_name").ok_or("the payload has no hook_event_name")?;
    let hook = match name.as_str() {
        "SessionStart" => HookEvent::SessionStart,
        "UserPromptSubmit" => HookEvent::UserPromptSubmit,
        "PostToolUse" => HookEvent::PostToolUse,
        "PostToolUseFailure" => HookEvent::PostToolUseFailure,
        "PermissionRequest" => HookEvent::PermissionRequest,
        "Notification" => HookEvent::Notification {
            kind: text(payload, "notification_type"),
            message: text(payload, "message").map(|message| first_line(&message)),
        },
        "Stop" => HookEvent::Stop,
        "StopFailure" => HookEvent::StopFailure,
        "SessionEnd" => HookEvent::SessionEnd,
        "SubagentStart" => HookEvent::SubagentStart,
        "SubagentStop" => HookEvent::SubagentStop,
        _ => HookEvent::Other(name),
    };
    // The same rule as `pick_review_engine::cache_path`: the variable when it says something,
    // else the default account's directory.
    let config_dir = config_dir
        .filter(|dir| !dir.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| home.join(".claude").to_string_lossy().to_string());
    Ok(AgentEvent {
        agent: "claude".to_string(),
        session_id,
        hook,
        agent_id: text(payload, "agent_id"),
        agent_type: text(payload, "agent_type"),
        cwd: text(payload, "cwd"),
        summary: tool_summary(payload),
        pid: pid
            .and_then(|pid| pid.trim().parse::<u32>().ok())
            .filter(|pid| *pid > 0),
        config_dir: Some(config_dir),
        at,
    })
}

/// A string key, with blank the same as absent.
fn text(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

/// `Edit: src/lib.rs`: the tool, and the first thing its input names.
fn tool_summary(payload: &Value) -> Option<String> {
    let tool = text(payload, "tool_name")?;
    let input = payload.get("tool_input");
    // The command before the description: the description is the model's own word for it, and
    // what a person is asked to allow is the command.
    let named = [
        "file_path",
        "notebook_path",
        "path",
        "pattern",
        "url",
        "command",
        "description",
    ]
    .iter()
    .find_map(|key| input.and_then(|input| text(input, key)));
    let summary = match named {
        Some(what) => format!("{tool}: {}", first_line(&what)),
        None => tool,
    };
    Some(summary.chars().take(SHOWN_CHARS).collect())
}

fn first_line(text: &str) -> String {
    let line = text.lines().find(|line| !line.trim().is_empty());
    line.unwrap_or_default()
        .trim()
        .chars()
        .take(SHOWN_CHARS)
        .collect()
}

#[cfg(test)]
mod tests;
