//! `adj hook <agent>`: the receiver an agent's hooks run. It reads one payload from stdin, hands
//! the event to the session ledger, and never fails the agent: whatever goes wrong is said on
//! stderr and the exit code is 0.
//!
//! The payload's shape is the agent's (Claude Code's or Codex's), so it is read here and
//! `registry` is handed a typed `AgentEvent`; `registry` holds who is running and where, not what
//! an agent's JSON looks like.

use serde_json::Value;
use std::io::Read;
use std::panic::AssertUnwindSafe;
use std::path::Path;

use super::args::HookArgs;
use crate::infra::clock::now_secs;
use crate::infra::env::{CLAUDE_CONFIG_DIR_ENV, CLAUDE_PID_ENV, CODEX_HOME_ENV};
use crate::infra::paths::home_dir;
use crate::infra::terminal::is_wake_line;
use crate::registry::{self, AgentEvent, HookEvent, LAST_MESSAGE_CHARS, RateWindow};

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
    let codex = match args.agent.as_str() {
        "claude" => false,
        "codex" => true,
        other => {
            say(format_args!("adjutant hook: unknown agent {other:?}"));
            return 0;
        }
    };
    if codex && args.status_line {
        say(format_args!(
            "adjutant hook: --status-line is Claude Code's status line; Codex has none"
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
    if args.status_line {
        // Whatever the status line's own script prints is what gets drawn, so nothing goes to
        // stdout here, whatever `hook_event_name` the payload carries.
        match std::panic::catch_unwind(AssertUnwindSafe(|| record_status_line(&payload))) {
            Ok(Ok(())) => {}
            Ok(Err(message)) => say(format_args!("adjutant hook: {message}")),
            Err(_) => say(format_args!(
                "adjutant hook: panicked while recording the status line"
            )),
        }
        return 0;
    }
    let permission =
        payload.get("hook_event_name").and_then(Value::as_str) == Some("PermissionRequest");
    match std::panic::catch_unwind(AssertUnwindSafe(|| record(&payload, codex))) {
        Ok(Ok(())) => {}
        Ok(Err(message)) => say(format_args!("adjutant hook: {message}")),
        Err(_) => say(format_args!(
            "adjutant hook: panicked while recording the event"
        )),
    }
    // The one event whose answer the agent reads: `{}` leaves the decision to its own prompt.
    // Said even when recording failed, or the failure would be the agent's. Codex reads a hook's
    // stdout as a decision, so nothing is printed for it, not even `{}`.
    if permission && !codex {
        // Not `println!`, which panics on a closed stdout, outside the `catch_unwind`.
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{{}}");
        let _ = out.flush();
    }
    0
}

fn record(payload: &Value, codex: bool) -> Result<(), String> {
    let event = if codex {
        codex_event(
            payload,
            std::env::var(CODEX_HOME_ENV).ok().as_deref(),
            &home_dir(),
            now_secs(),
        )?
    } else {
        claude_event(
            payload,
            std::env::var(CLAUDE_PID_ENV).ok().as_deref(),
            std::env::var(CLAUDE_CONFIG_DIR_ENV).ok().as_deref(),
            &home_dir(),
            now_secs(),
        )?
    };
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
        "UserPromptSubmit" => HookEvent::UserPromptSubmit {
            typed: typed(payload),
        },
        "PostToolUse" => HookEvent::PostToolUse,
        "PostToolUseFailure" => HookEvent::PostToolUseFailure,
        "PermissionRequest" => HookEvent::PermissionRequest,
        "Notification" => HookEvent::Notification {
            kind: text(payload, "notification_type"),
            message: text(payload, "message").map(|message| first_line(&message)),
        },
        "Stop" => HookEvent::Stop {
            message: last_message(payload),
        },
        "StopFailure" => HookEvent::StopFailure {
            message: last_message(payload),
        },
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

/// A Codex hook payload as the event it reports. Codex gives a hook no pid, so the row has none
/// and falls back to the age rule; `Interrupt` (the turn cut short) is the same to the ledger as
/// the turn ending.
fn codex_event(
    payload: &Value,
    codex_home: Option<&str>,
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
        "UserPromptSubmit" => HookEvent::UserPromptSubmit {
            typed: typed(payload),
        },
        "PostToolUse" => HookEvent::PostToolUse,
        "PermissionRequest" => HookEvent::PermissionRequest,
        "Stop" => HookEvent::Stop {
            message: last_message(payload),
        },
        // Cut short, so there is no message of its own to keep.
        "Interrupt" => HookEvent::Stop { message: None },
        "SessionEnd" => HookEvent::SessionEnd,
        "SubagentStart" => HookEvent::SubagentStart,
        "SubagentStop" => HookEvent::SubagentStop,
        _ => HookEvent::Other(name),
    };
    let config_dir = codex_home
        .filter(|dir| !dir.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| home.join(".codex").to_string_lossy().to_string());
    Ok(AgentEvent {
        agent: "codex".to_string(),
        session_id,
        hook,
        agent_id: text(payload, "agent_id"),
        agent_type: text(payload, "agent_type"),
        cwd: text(payload, "cwd"),
        summary: tool_summary(payload),
        pid: None,
        config_dir: Some(config_dir),
        at,
    })
}

fn record_status_line(payload: &Value) -> Result<(), String> {
    let Some(event) = claude_status_line(payload, now_secs())? else {
        return Ok(());
    };
    let cwd = text(payload, "cwd").or_else(|| {
        payload
            .get("workspace")
            .and_then(|workspace| text(workspace, "current_dir"))
    });
    let root = registry::hook_state_root(cwd.as_deref().map(Path::new));
    registry::record_agent_event(&root, &event)
}

/// A Claude Code status line input as the figures it shows, or `None` when it shows none of
/// them. The `cwd` is not part of the event: a status line only fills a row that exists, and
/// the row already says where its session is.
fn claude_status_line(payload: &Value, at: i64) -> Result<Option<AgentEvent>, String> {
    let session_id = text(payload, "session_id").unwrap_or_default();
    if !registry::valid_session_id(&session_id) {
        return Err(format!("not a usable session id: {session_id:?}"));
    }
    let model = match payload.get("model") {
        Some(Value::String(name)) => Some(name.clone()).filter(|name| !name.trim().is_empty()),
        Some(model @ Value::Object(_)) => text(model, "display_name").or_else(|| text(model, "id")),
        _ => None,
    };
    let context_percent = payload
        .get("context_window")
        .and_then(|window| percent(window.get("used_percentage")))
        .map(f64::round);
    let limits = payload.get("rate_limits");
    let window = |key: &str| {
        let window = limits?.get(key)?;
        Some(RateWindow {
            used_percent: Some(percent(window.get("used_percentage"))?),
            resets_at: window
                .get("resets_at")
                .and_then(Value::as_f64)
                .map(|at| at as i64),
            other: serde_json::Map::new(),
        })
    };
    let (five_hour, seven_day) = (window("five_hour"), window("seven_day"));
    if model.is_none() && context_percent.is_none() && five_hour.is_none() && seven_day.is_none() {
        return Ok(None);
    }
    Ok(Some(AgentEvent {
        agent: "claude".to_string(),
        session_id,
        hook: HookEvent::StatusLine {
            model,
            context_percent,
            five_hour,
            seven_day,
        },
        agent_id: None,
        agent_type: None,
        cwd: None,
        summary: None,
        pid: None,
        config_dir: None,
        at,
    }))
}

/// A percentage a status line gave: a finite number, not below zero.
fn percent(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|percent| percent.is_finite() && *percent >= 0.0)
}

/// Whether a `UserPromptSubmit` payload's prompt is the person's: present, and not one of the
/// wake lines adjutant types into a terminal. A payload with no `prompt` says nothing of who
/// wrote it, so it is not counted.
fn typed(payload: &Value) -> bool {
    text(payload, "prompt").is_some_and(|prompt| !is_wake_line(&prompt))
}

/// A string key, with blank the same as absent.
fn text(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

/// What the agent said last in the turn the payload ends: trimmed, blank the same as absent,
/// line breaks kept, and cut to `LAST_MESSAGE_CHARS` on a character boundary with an ellipsis.
fn last_message(payload: &Value) -> Option<String> {
    let message = text(payload, "last_assistant_message")?;
    let message = message.trim();
    Some(match message.char_indices().nth(LAST_MESSAGE_CHARS) {
        Some((end, _)) => format!("{}…", message[..end].trim_end()),
        None => message.to_string(),
    })
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
    .find_map(|key| input.and_then(|input| text(input, key)))
    // `AskUserQuestion` names none of those: what it asks is the first question.
    .or_else(|| {
        input?
            .get("questions")?
            .as_array()?
            .first()
            .and_then(|first| text(first, "question"))
    });
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
