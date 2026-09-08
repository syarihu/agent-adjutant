//! Telling the human something happened.
//!
//! Two moments need it: a worker finishing and wanting hands-on verification, and a report
//! arriving for a hub nobody is watching. Both are "come back to this tab", and both are
//! useless if the channel is one the person does not look at — hence a template rather than
//! a hardcoded notifier.

use crate::config::Hook;
use crate::template::{Sub, render, sh_quote};

/// A notification only interrupts if it makes a noise; a silent banner in the corner is the
/// same as no notification for someone who has walked away from the machine.
pub fn default_command(title: &str, message: &str) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let script = format!(
        "display notification {} with title {} sound name \"Glass\"",
        applescript_string(message),
        applescript_string(title)
    );
    Some(format!("osascript -e {}", sh_quote(&script)))
}

fn applescript_string(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ");
    format!("\"{escaped}\"")
}

/// The command line that would deliver this notification, or `None` when there is none to
/// deliver — because the user turned it off, or because this platform has no built-in.
/// `None` is not a failure: a missing notification never justifies failing the thing it was
/// announcing.
pub fn command(hook: &Hook, title: &str, message: &str) -> Option<String> {
    if hook.is_off() {
        return None;
    }
    match hook.template() {
        Some(template) => Some(render(
            template,
            &[
                ("title", Sub::Quoted(title)),
                ("message", Sub::Quoted(message)),
            ],
        )),
        None => default_command(title, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_webhook_is_a_config_line() {
        let cmd = command(
            &Hook::Command("curl -s -X POST -d {message} https://example.invalid/hook".into()),
            "adjutant",
            "WID-957 needs hands-on verification",
        )
        .unwrap();
        assert_eq!(
            cmd,
            "curl -s -X POST -d 'WID-957 needs hands-on verification' https://example.invalid/hook"
        );
    }

    #[test]
    fn a_message_cannot_break_out_of_the_applescript_string() {
        let cmd = default_command("adjutant", "he said \"go\"\nrm -rf /").unwrap_or_default();
        if cmd.is_empty() {
            return; // not macOS
        }
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd.replace("osascript -e", "printf %s"))
            .output()
            .unwrap();
        let script = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(script.contains("he said \\\"go\\\" rm -rf /"), "{script}");
        assert_eq!(script.lines().count(), 1, "{script}");
    }

    #[test]
    fn a_template_that_uses_neither_placeholder_is_still_a_valid_command() {
        assert_eq!(
            command(&Hook::Command("printf '\\a'".into()), "t", "m").unwrap(),
            "printf '\\a'"
        );
    }

    #[test]
    fn off_is_a_different_answer_from_unset() {
        assert_eq!(command(&Hook::Off, "t", "m"), None);
        // Unset still tries the platform's own notifier, wherever there is one.
        assert_eq!(
            command(&Hook::BuiltIn, "t", "m").is_some(),
            cfg!(target_os = "macos")
        );
    }
}
