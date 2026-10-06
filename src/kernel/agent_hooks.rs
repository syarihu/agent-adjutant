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
    guarded(exe, "hook claude")
}

/// `claude_hook_command` for the entries `adj setup claude` writes into a global settings file:
/// the same, with `--global` marking them as adjutant's so they can be found again.
pub fn claude_global_hook_command(exe: &str) -> String {
    guarded(exe, "hook claude --global")
}

fn guarded(exe: &str, args: &str) -> String {
    let quoted = sh_quote(exe);
    format!("{GUARD_OPEN}{quoted}{GUARD_MID}{quoted} {args} || true")
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

/// Whether `command` is one `claude_global_hook_command` wrote, whatever binary it names: the
/// whole guarded form `[ ! -x P ] || P hook claude --global || true`, or the bare `P hook claude
/// --global` (optionally `|| true`) with nothing before it. Read by words and anchored at both
/// ends, so a user's `echo x; adj hook claude --global` or `echo adj hook claude --global` is not
/// taken for ours. Only single quotes are understood, the only ones adjutant writes; a command
/// with an unbalanced one is not ours.
pub fn is_global_hook_command(command: &str) -> bool {
    let mut words = Vec::new();
    let mut rest = command.trim_start();
    while !rest.is_empty() {
        let Some((word, after)) = unquote_word(rest) else {
            return false;
        };
        words.push(word);
        rest = after.trim_start();
    }
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let flag = ["hook", "claude", "--global"];
    match words.as_slice() {
        ["[", "!", "-x", path, "]", "||", again, rest @ ..] => {
            !path.is_empty() && path == again && rest == [flag[0], flag[1], flag[2], "||", "true"]
        }
        [path, rest @ ..] => {
            !path.is_empty()
                && rest.starts_with(&flag)
                && matches!(&rest[flag.len()..], [] | ["||", "true"])
        }
        [] => false,
    }
}

const SHAPE: &str = "its \"hooks\" table is not the shape Claude Code reads";

/// The `hooks` table of a settings file, when there is one and it is an object.
fn hooks_table(settings: &Value) -> Result<Option<&serde_json::Map<String, Value>>, String> {
    let root = settings
        .as_object()
        .ok_or_else(|| "the settings are not a JSON object".to_string())?;
    match root.get("hooks") {
        None => Ok(None),
        Some(Value::Object(hooks)) => Ok(Some(hooks)),
        Some(_) => Err(format!("{SHAPE}: \"hooks\" is not an object")),
    }
}

fn not_an_array(event: &str) -> String {
    format!("{SHAPE}: \"hooks.{event}\" is not an array")
}

/// The `command` of every handler in the groups of one event that is ours.
fn ours_in(groups: &mut [Value]) -> impl Iterator<Item = &mut Value> {
    groups
        .iter_mut()
        .filter_map(|group| group.get_mut("hooks")?.as_array_mut())
        .flat_map(|handlers| handlers.iter_mut())
        .filter(|handler| is_ours(handler))
}

fn is_ours(handler: &Value) -> bool {
    handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(is_global_hook_command)
}

/// How many handlers in `settings` are ours.
pub fn count_global_hooks(settings: &Value) -> usize {
    let Ok(Some(hooks)) = hooks_table(settings) else {
        return 0;
    };
    hooks
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks")?.as_array())
        .flatten()
        .filter(|handler| is_ours(handler))
        .count()
}

/// `settings` with `command` under every event of `CLAUDE_EVENTS`, next to whatever is there.
///
/// An event that already has one of our handlers has its command rewritten in place (the binary
/// may have moved); the others get a group of their own, never a handler inside a group the user
/// wrote. Nothing is ever removed. A settings file that is not the shape Claude Code reads is
/// refused rather than repaired, since the user's hooks in it are not ours to reinterpret.
pub fn add_global_hooks(settings: &Value, command: &str) -> Result<Value, String> {
    hooks_table(settings)?;
    let mut out = settings.clone();
    let hooks = out
        .as_object_mut()
        .and_then(|root| {
            root.entry("hooks")
                .or_insert_with(|| json!({}))
                .as_object_mut()
        })
        .ok_or_else(|| "the settings are not a JSON object".to_string())?;
    for (event, matcher) in CLAUDE_EVENTS {
        let groups = hooks
            .entry(event.to_string())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| not_an_array(event))?;
        if let Some(handler) = ours_in(groups).next() {
            handler["command"] = json!(command);
            continue;
        }
        let mut group = serde_json::Map::new();
        if let Some(matcher) = matcher {
            group.insert("matcher".to_string(), json!(matcher));
        }
        group.insert(
            "hooks".to_string(),
            json!([{ "type": "command", "command": command }]),
        );
        groups.push(Value::Object(group));
    }
    Ok(out)
}

/// `settings` without the handlers `add_global_hooks` wrote, and how many that was. A group or an
/// event left empty by it goes too; one that was empty before is left alone, and so is everything
/// else.
pub fn remove_global_hooks(settings: &Value) -> Result<(Value, usize), String> {
    let Some(table) = hooks_table(settings)? else {
        return Ok((settings.clone(), 0));
    };
    if let Some((event, _)) = table.iter().find(|(_, groups)| !groups.is_array()) {
        return Err(not_an_array(event));
    }
    let mut out = settings.clone();
    let mut removed = 0;
    let hooks = out["hooks"].as_object_mut().expect("checked above");
    let mut emptied = Vec::new();
    for (event, groups) in hooks.iter_mut() {
        let groups = groups.as_array_mut().expect("checked above");
        let mut here = 0;
        groups.retain_mut(|group| {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let had = handlers.len();
            handlers.retain(|handler| !is_ours(handler));
            here += had - handlers.len();
            !(handlers.is_empty() && had > 0)
        });
        if here > 0 && groups.is_empty() {
            emptied.push(event.clone());
        }
        removed += here;
    }
    for event in emptied {
        hooks.remove(&event);
    }
    Ok((out, removed))
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

    const ODD_PATHS: [&str; 5] = [
        "/usr/local/bin/adj",
        "/Users/some one/bin/adj",
        "/it's/here/adj",
        "/a'b c'/adj",
        "/=odd/adj",
    ];

    fn user_settings() -> Value {
        json!({
            "permissions": { "allow": ["Bash(ls)"] },
            "env": { "A": "1" },
            "statusLine": { "type": "command", "command": "echo hi" },
            "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }],
                "PostToolUse": [{
                    "matcher": "Edit",
                    "hooks": [{ "type": "command", "command": "fmt" }]
                }],
                "PreToolUse": [{ "hooks": [{ "type": "command", "command": "guard" }] }]
            }
        })
    }

    #[test]
    fn the_global_command_is_the_injected_one_with_the_flag_and_names_the_same_binary() {
        for exe in ODD_PATHS {
            let injected = claude_hook_command(exe);
            let global = claude_global_hook_command(exe);
            assert_eq!(
                global,
                injected.replace(" hook claude ||", " hook claude --global ||")
            );
            assert_eq!(binary_of(&claude_settings(&global)).as_deref(), Some(exe));
        }
    }

    #[test]
    fn the_table_has_the_events_the_tests_count_on() {
        // tests/setup.rs counts entries as this many; change both together.
        assert_eq!(CLAUDE_EVENTS.len(), 11);
    }

    #[test]
    fn our_commands_are_recognised_by_word_whatever_the_path() {
        for exe in ODD_PATHS {
            assert!(
                is_global_hook_command(&claude_global_hook_command(exe)),
                "{exe}"
            );
        }
        assert!(is_global_hook_command("adj hook claude --global"));
        assert!(!is_global_hook_command("echo x; adj hook claude --global"));
        assert!(!is_global_hook_command("echo adj hook claude --global"));
        assert!(!is_global_hook_command(
            "[ ! -x a ] || b hook claude --global || true"
        ));
        assert!(is_global_hook_command(
            "/x/adj hook claude --global || true"
        ));
        for other in [
            claude_hook_command("/usr/local/bin/adj"),
            "adj hook claude --status-line".to_string(),
            "adj hook claude --global-x".to_string(),
            "adj hook claude --global --other".to_string(),
            "echo hook claude".to_string(),
            "hook claude --global".to_string(),
            "'/unbalanced hook claude --global".to_string(),
            "adj hook claude --global && rm -rf x".to_string(),
            String::new(),
        ] {
            assert!(!is_global_hook_command(&other), "{other}");
        }
    }

    #[test]
    fn adding_to_nothing_gives_exactly_the_injected_table() {
        let command = claude_global_hook_command("/bin/adj");
        assert_eq!(
            add_global_hooks(&json!({}), &command).unwrap(),
            claude_settings(&command)
        );
    }

    #[test]
    fn adding_keeps_the_users_hooks_and_keys_and_appends_ours_as_separate_groups() {
        let command = claude_global_hook_command("/bin/adj");
        let before = user_settings();
        let after = add_global_hooks(&before, &command).unwrap();
        assert_eq!(after["permissions"], before["permissions"]);
        assert_eq!(after["env"], before["env"]);
        assert_eq!(after["statusLine"], before["statusLine"]);
        assert_eq!(after["hooks"]["PreToolUse"], before["hooks"]["PreToolUse"]);
        let stop = after["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop[0], before["hooks"]["Stop"][0]);
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[1]["hooks"][0]["command"], json!(command));
        let post = after["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(post[0], before["hooks"]["PostToolUse"][0]);
        assert_eq!(post[1]["matcher"], "*");
        assert_eq!(count_global_hooks(&after), CLAUDE_EVENTS.len());
        assert_eq!(count_global_hooks(&before), 0);
    }

    #[test]
    fn adding_twice_changes_nothing() {
        let command = claude_global_hook_command("/bin/adj");
        let once = add_global_hooks(&user_settings(), &command).unwrap();
        assert_eq!(add_global_hooks(&once, &command).unwrap(), once);
    }

    #[test]
    fn a_moved_binary_is_rewritten_in_place_and_the_users_groups_are_untouched() {
        let old = claude_global_hook_command("/gone/a b/adj");
        let new = claude_global_hook_command("/bin/adj");
        let first = add_global_hooks(&user_settings(), &old).unwrap();
        let again = add_global_hooks(&first, &new).unwrap();
        assert_eq!(count_global_hooks(&again), CLAUDE_EVENTS.len());
        assert!(!again.to_string().contains("/gone/"));
        let expected = add_global_hooks(&user_settings(), &new).unwrap();
        assert_eq!(again, expected);
    }

    #[test]
    fn removing_takes_only_ours_even_from_a_group_shared_with_a_user_handler() {
        let injected = claude_hook_command("/bin/adj");
        let mut settings = user_settings();
        settings["hooks"]["Stop"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .extend([
                json!({ "type": "command", "command": claude_global_hook_command("/x y/adj") }),
                json!({ "type": "command", "command": injected }),
            ]);
        let with_ours =
            add_global_hooks(&settings, &claude_global_hook_command("/bin/adj")).unwrap();
        let (after, removed) = remove_global_hooks(&with_ours).unwrap();
        assert_eq!(removed, CLAUDE_EVENTS.len());
        let mut kept = settings.clone();
        kept["hooks"]["Stop"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .remove(1);
        assert_eq!(after, kept);
        assert_eq!(count_global_hooks(&after), 0);
        let (same, none) = remove_global_hooks(&after).unwrap();
        assert_eq!((same, none), (after, 0));
    }

    #[test]
    fn removing_from_what_adding_made_leaves_an_empty_hooks_table() {
        let added = add_global_hooks(&json!({}), &claude_global_hook_command("/bin/adj")).unwrap();
        assert_eq!(
            remove_global_hooks(&added).unwrap(),
            (json!({ "hooks": {} }), CLAUDE_EVENTS.len())
        );
    }

    #[test]
    fn an_unexpected_shape_is_refused_by_both() {
        let command = claude_global_hook_command("/bin/adj");
        for bad in [
            json!([]),
            json!("text"),
            json!({ "hooks": [] }),
            json!({ "hooks": null }),
            json!({ "hooks": { "Stop": {} } }),
        ] {
            assert!(add_global_hooks(&bad, &command).is_err(), "{bad}");
            assert!(remove_global_hooks(&bad).is_err(), "{bad}");
        }
        // An event adjutant does not use is only a problem for removing.
        let odd = json!({ "hooks": { "PreToolUse": "x" } });
        assert!(add_global_hooks(&odd, &command).is_ok());
        assert!(remove_global_hooks(&odd).is_err());
    }
}
