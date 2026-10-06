//! The hooks adjutant wants from the agents it starts: which events, and the command each one
//! runs.
//!
//! What a hook does with its payload is `adj hook claude`'s business (`transport::cli::hook`);
//! this is only the table that makes Claude Code call it, as the settings file passed with
//! `--settings`.

use serde_json::{Value, json};

use crate::infra::template::sh_quote;

/// The events that reach `adj hook claude`, and the tool matcher the three tool events carry.
///
/// Every entry runs the same command, since the receiver reads the event from the payload's
/// `hook_event_name`. No hook is `async`: the receiver is quick and a hook that is not waited
/// for is one whose events can arrive out of order.
pub const CLAUDE_EVENTS: &[(&str, Option<&str>)] = &[
    ("SessionStart", None),
    ("UserPromptSubmit", None),
    ("PostToolUse", Some("*")),
    ("PostToolUseFailure", Some("*")),
    ("PermissionRequest", Some("*")),
    ("Notification", None),
    ("Stop", None),
    ("StopFailure", None),
    ("SessionEnd", None),
    ("SubagentStart", None),
    ("SubagentStop", None),
];

const GUARD_OPEN: &str = "[ ! -x ";
const GUARD_MID: &str = " ] || ";

/// The command every hook runs: `exe hook claude`, which always exits 0.
///
/// Claude Code treats exit 2 as blocking (on `Stop` it keeps the agent from stopping) and any
/// other non-zero exit as a hook error. A binary that is gone, or too old to know `adj hook`
/// (clap refuses unknown arguments with exit 2), must therefore leave a running session's hooks
/// doing nothing rather than doing harm. The path is quoted wherever it appears: one with a
/// space would otherwise turn every hook into a no-op that the guard hides.
pub fn claude_hook_command(exe: &str) -> String {
    let quoted = sh_quote(exe);
    format!("{GUARD_OPEN}{quoted}{GUARD_MID}{quoted} hook claude || true")
}

/// The settings file for `--settings`: `command` under every event of `CLAUDE_EVENTS`.
pub fn claude_settings(command: &str) -> Value {
    let hooks: serde_json::Map<String, Value> = CLAUDE_EVENTS
        .iter()
        .map(|(event, matcher)| {
            let mut entry = serde_json::Map::new();
            if let Some(matcher) = matcher {
                entry.insert("matcher".to_string(), json!(matcher));
            }
            entry.insert(
                "hooks".to_string(),
                json!([{ "type": "command", "command": command }]),
            );
            (event.to_string(), json!([Value::Object(entry)]))
        })
        .collect();
    json!({ "hooks": hooks })
}

/// The binary a settings file written by `claude_settings` calls, read back out of its first
/// command. `None` for a file that is not one: it is somebody else's, or from a different
/// version of this table.
pub fn binary_of(settings: &Value) -> Option<String> {
    let (event, _) = CLAUDE_EVENTS.first()?;
    let command = settings
        .get("hooks")?
        .get(event)?
        .get(0)?
        .get("hooks")?
        .get(0)?
        .get("command")?
        .as_str()?;
    let rest = command.strip_prefix(GUARD_OPEN)?;
    let (exe, rest) = unquote_word(rest)?;
    rest.starts_with(GUARD_MID).then_some(exe)
}

/// One shell word, as `sh_quote` writes it, and what follows it.
fn unquote_word(text: &str) -> Option<(String, &str)> {
    let mut word = String::new();
    let mut chars = text.char_indices().peekable();
    let mut quoted = false;
    while let Some((at, c)) = chars.next() {
        match (quoted, c) {
            (false, '\'') => quoted = true,
            (true, '\'') => quoted = false,
            (false, '\\') => word.push(chars.next()?.1),
            (false, c) if c.is_whitespace() => return Some((word, &text[at..])),
            (_, c) => word.push(c),
        }
    }
    (!quoted).then_some((word, ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_has_one_entry_and_only_the_tool_events_have_a_matcher() {
        let settings = claude_settings("cmd");
        let hooks = settings["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), CLAUDE_EVENTS.len());
        for (event, matcher) in CLAUDE_EVENTS {
            let entries = hooks[*event].as_array().unwrap();
            assert_eq!(entries.len(), 1, "{event}");
            assert_eq!(
                entries[0].get("matcher").and_then(Value::as_str),
                *matcher,
                "{event}"
            );
            let commands = entries[0]["hooks"].as_array().unwrap();
            assert_eq!(commands.len(), 1, "{event}");
            assert_eq!(commands[0], json!({ "type": "command", "command": "cmd" }));
        }
        let with_matcher: Vec<_> = CLAUDE_EVENTS
            .iter()
            .filter(|(_, m)| m.is_some())
            .map(|(e, _)| *e)
            .collect();
        assert_eq!(
            with_matcher,
            ["PostToolUse", "PostToolUseFailure", "PermissionRequest"]
        );
        assert!(!settings.to_string().contains("async"));
    }

    #[test]
    fn the_binary_is_read_back_out_of_the_command() {
        for exe in [
            "/usr/local/bin/adj",
            "/Users/some one/bin/adj",
            "/it's/here/adj",
            "/a'b c'/adj",
            "/=odd/adj",
        ] {
            let settings = claude_settings(&claude_hook_command(exe));
            assert_eq!(binary_of(&settings).as_deref(), Some(exe));
        }
        assert_eq!(binary_of(&json!({})), None);
        assert_eq!(binary_of(&claude_settings("something else")), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_command_runs_the_binary_and_exits_zero_even_when_it_is_gone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("with space").join("adj");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        let ran = dir.path().join("ran");
        std::fs::write(
            &exe,
            format!("#!/bin/sh\necho \"$@\" > '{}'\nexit 2\n", ran.display()),
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = claude_hook_command(exe.to_str().unwrap());
        let status = std::process::Command::new("sh")
            .args(["-c", &command])
            .status()
            .unwrap();
        // The stub exits 2, which Claude Code would read as "block"; the guard turns it to 0.
        assert!(status.success());
        assert_eq!(std::fs::read_to_string(&ran).unwrap().trim(), "hook claude");

        std::fs::remove_file(&exe).unwrap();
        let status = std::process::Command::new("sh")
            .args(["-c", &command])
            .status()
            .unwrap();
        assert!(status.success());
    }
}
