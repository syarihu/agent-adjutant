//! Telling the human something happened.
//!
//! Two moments need it: a worker finishing and wanting hands-on verification, and a report
//! arriving for a hub nobody is watching. Both are "come back to this tab", and both are
//! useless if the channel is one the person does not look at — hence a template rather than
//! a hardcoded notifier.

use crate::config::Hook;
use crate::repo::RepoInfo;
use crate::template::{Sub, contains_placeholder, render, sh_quote};

/// The terminal every other built-in already assumes. Hardcoded in the *default* only,
/// which is the thing `notification` exists to replace on a machine running something else.
const DEFAULT_ACTIVATE: &str = "com.googlecode.iterm2";

/// A notification only interrupts if it makes a noise; a silent banner in the corner is the
/// same as no notification for someone who has walked away from the machine.
///
/// `terminal-notifier` when it is installed, `osascript` when it is not. The order is not a
/// preference: macOS credits a notification posted by command-line `osascript` to Script
/// Editor, so the banner arrives from an app nobody asked for and clicking it opens an empty
/// Script Editor rather than the session that wanted attention. The fallback is still worth
/// having — a banner you cannot usefully click beats no banner — so this follows the same
/// "use it if it is there" rule as the other optional neighbours rather than making a brew
/// install a hard requirement.
pub fn default_command(title: &str, message: &str) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    Some(if on_path("terminal-notifier") {
        terminal_notifier_command(title, message)
    } else {
        osascript_command(title, message)
    })
}

/// `-activate` rather than `-sender`: spoofing another app's bundle id is stripped on newer
/// macOS, while this only says what to raise when the banner is clicked — which is the part
/// the person actually wanted.
fn terminal_notifier_command(title: &str, message: &str) -> String {
    format!(
        "terminal-notifier -title {} -message {} -sound Glass -activate {DEFAULT_ACTIVATE}",
        sh_quote(title),
        sh_quote(message)
    )
}

fn osascript_command(title: &str, message: &str) -> String {
    let script = format!(
        "display notification {} with title {} sound name \"Glass\"",
        applescript_string(message),
        applescript_string(title)
    );
    format!("osascript -e {}", sh_quote(&script))
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| in_path(&path, program))
}

/// Whether `program` is an executable file in this `PATH`. Split out from the environment so
/// a test can answer for a directory it built rather than for the developer's own machine.
/// A directory of that name, or a file nobody may run, is not a notifier.
fn in_path(path: &std::ffi::OsStr, program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path).any(|dir| {
        std::fs::metadata(dir.join(program))
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
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
pub fn command(hook: &Hook, nwo: &str, title: &str, message: &str) -> Option<String> {
    if hook.is_off() {
        return None;
    }
    match hook.template() {
        Some(template) => Some(render(
            template,
            &[
                ("title", Sub::Quoted(title)),
                ("message", Sub::Quoted(message)),
                ("nwo", Sub::Quoted(nwo)),
            ],
        )),
        None => default_command(title, message),
    }
}

/// The same, for a message *about a repository* — which is every notification this tool
/// raises on its own. The title sentence lives here rather than at each call site so that
/// `{title}` and `{nwo}` cannot end up describing different repositories.
pub fn repo_command(hook: &Hook, info: &RepoInfo, message: &str) -> Option<String> {
    command(
        hook,
        &info.nwo,
        &format!("adjutant / {}", info.repo),
        message,
    )
}

/// Whether this notifier has to be told which repository the message is about. `adjutant
/// notify` runs from anywhere, and a template that raises a hub cannot be answered with an
/// empty repository — `adj focus --repo ''` is a worse outcome than a warning.
pub fn needs_repo(hook: &Hook) -> bool {
    hook.template()
        .is_some_and(|t| contains_placeholder(t, "nwo"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_webhook_is_a_config_line() {
        let cmd = command(
            &Hook::Command("curl -s -X POST -d {message} https://example.invalid/hook".into()),
            "acme/widget",
            "adjutant",
            "WID-957 needs hands-on verification",
        )
        .unwrap();
        assert_eq!(
            cmd,
            "curl -s -X POST -d 'WID-957 needs hands-on verification' https://example.invalid/hook"
        );
    }

    /// The recommended shape: a script that gets the three values as arguments and decides
    /// for itself what to run on a click. Deliberately not the `-execute '… {nwo}'` form —
    /// the README steers away from nesting a quoted command inside a template, so a test has
    /// no business pinning it as the normal way to write one.
    #[test]
    fn a_template_can_ask_which_repository_the_message_is_about() {
        assert_eq!(
            command(
                &Hook::Command("adj-notify {title} {message} {nwo}".into()),
                "acme/widget",
                "adjutant / widget",
                "done"
            )
            .unwrap(),
            "adj-notify 'adjutant / widget' done acme/widget"
        );
    }

    /// Whatever the values are, they arrive as exactly three arguments. A repository with no
    /// usable remote is named after its directory, so spaces and quotes in `{nwo}` are not
    /// hypothetical, and the message is written by whoever sent the report.
    #[test]
    fn the_three_values_stay_three_arguments_through_a_real_shell() {
        for (nwo, title, message) in [
            ("", "adjutant", "done"),
            ("/src/my repo", "adjutant / my repo", "it's done"),
            (
                "acme/widget",
                "adjutant / widget",
                "subject: {nwo} $(echo pwned)",
            ),
        ] {
            let rendered = command(
                &Hook::Command("printf '%s\\n' {title} {message} {nwo}".into()),
                nwo,
                title,
                message,
            )
            .unwrap();
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(&rendered)
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .collect::<Vec<_>>(),
                [title, message, nwo],
                "{rendered}"
            );
        }
    }

    #[test]
    fn the_title_and_the_repository_placeholder_name_the_same_repository() {
        let info = RepoInfo {
            main: "/src/widget".into(),
            nwo: "acme/widget".into(),
            repo: "widget".into(),
            slug: "acme-widget".into(),
            hub_name: "adjutant-acme-widget".into(),
            nwo_source: "origin",
        };
        assert_eq!(
            repo_command(&Hook::Command("n {title} {nwo}".into()), &info, "done").unwrap(),
            "n 'adjutant / widget' acme/widget"
        );
    }

    #[test]
    fn only_a_template_that_uses_the_repository_needs_one() {
        assert!(needs_repo(&Hook::Command("n {nwo}".into())));
        assert!(!needs_repo(&Hook::Command("n {message}".into())));
        // The built-in raises the terminal rather than one session, so it needs nothing.
        assert!(!needs_repo(&Hook::BuiltIn));
        assert!(!needs_repo(&Hook::Off));
    }

    #[test]
    fn a_message_cannot_break_out_of_the_applescript_string() {
        let cmd = osascript_command("adjutant", "he said \"go\"\nrm -rf /");
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
    fn a_message_cannot_break_out_of_the_notifier_arguments() {
        let cmd = terminal_notifier_command("adjutant / widget", "it's $(rm -rf /) done");
        assert_eq!(
            cmd,
            "terminal-notifier -title 'adjutant / widget' -message 'it'\\''s $(rm -rf /) done' \
             -sound Glass -activate com.googlecode.iterm2"
        );
    }

    /// Only an executable file counts: a directory called `terminal-notifier`, or a file
    /// nobody may run, would send `default_command` down a branch that cannot deliver.
    #[test]
    fn the_path_search_wants_something_it_can_actually_run() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let path = std::ffi::OsString::from(format!("/nonexistent:{}", bin.display()));

        assert!(!in_path(&path, "terminal-notifier"));

        let file = bin.join("terminal-notifier");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        assert!(!in_path(&path, "terminal-notifier"), "not executable yet");

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(in_path(&path, "terminal-notifier"));

        std::fs::create_dir(bin.join("notify-send")).unwrap();
        assert!(
            !in_path(&path, "notify-send"),
            "a directory is not a program"
        );
    }

    #[test]
    fn off_is_a_different_answer_from_unset() {
        assert_eq!(command(&Hook::Off, "acme/widget", "t", "m"), None);
        // Unset still tries the platform's own notifier, wherever there is one.
        assert_eq!(
            command(&Hook::BuiltIn, "acme/widget", "t", "m").is_some(),
            cfg!(target_os = "macos")
        );
    }

    #[test]
    fn a_template_that_uses_no_placeholder_is_still_a_valid_command() {
        assert_eq!(
            command(
                &Hook::Command("printf '\\a'".into()),
                "acme/widget",
                "t",
                "m"
            )
            .unwrap(),
            "printf '\\a'"
        );
    }
}
