use super::*;

// ── iTerm2 ───────────────────────────────────────────────────────────

pub fn applescript_literal(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Naming the tab is deliberately *not* done here.
///
/// `set name of session` looked like the obvious way and is the one mechanism that does not
/// generalise: a profile whose title format is driven by user variables ignores it
/// outright, which is a machine where the tab silently keeps the wrong name. Every terminal
/// can instead be told by the shell running inside the tab — so the caller prepends that to
/// the line, through the same `terminal.title` setting a session uses to name itself.
pub(super) fn iterm_spawn_script(line: &str) -> String {
    format!(
        "tell application \"iTerm2\"\n  \
           tell current window\n    \
             set newTab to (create tab with default profile)\n    \
             tell current session of newTab\n      \
               write text \"{}\"\n    \
             end tell\n  \
           end tell\n\
         end tell",
        // `write text` into a default-profile tab runs in an interactive shell, so PATH,
        // hooks and prompt are all loaded. The `create tab … command "…"` form replaces the
        // shell and loses them.
        applescript_literal(line)
    )
}

pub(super) fn iterm_focus_script(tty: &str) -> String {
    format!(
        "tell application \"iTerm2\"\n  \
           activate\n  \
           repeat with w in windows\n    \
             repeat with t in tabs of w\n      \
               repeat with s in sessions of t\n        \
                 if tty of s is \"/dev/{}\" then\n          \
                   try\n            select w\n          end try\n          \
                   tell t to select\n          \
                   tell s to select\n          \
                   return\n        \
                 end if\n      \
               end repeat\n    \
             end repeat\n  \
           end repeat\n\
         end tell",
        applescript_literal(tty)
    )
}

/// The same walk as `iterm_focus_script`, with `close` where that one selects: a tty is the
/// only handle anyone has on which of a dozen tabs belongs to the pid they know.
///
/// The marker's placement is the point of the shape: inside the branch that matched the
/// tty, with the fall-through returning nothing.
///
/// No `activate` here. `focus` raises iTerm2 because being raised is what was asked for;
/// pulling the whole app forward in order to dispose of a tab takes the person's attention
/// for something they will not be looking at.
pub(super) fn iterm_close_script(tty: &str) -> String {
    format!(
        "tell application \"iTerm2\"\n  \
           repeat with w in windows\n    \
             repeat with t in tabs of w\n      \
               repeat with s in sessions of t\n        \
                 if tty of s is \"/dev/{}\" then\n          \
                   close s\n          \
                   return \"{CLOSED_MARKER}\"\n        \
                 end if\n      \
               end repeat\n    \
             end repeat\n  \
           end repeat\n\
         end tell\n\
         return \"\"",
        applescript_literal(tty)
    )
}

pub(super) fn osascript(script: &str) -> Result<String, String> {
    use std::io::Write;
    let mut child = Command::new("osascript")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run osascript: {e}"))?;
    child
        .stdin
        .as_mut()
        .ok_or("cannot write to osascript")?
        .write_all(script.as_bytes())
        .map_err(|e| format!("cannot write to osascript: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("osascript did not finish: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if err.contains("current window") {
            return Err("no iTerm2 window is open".to_string());
        }
        return Err(if err.is_empty() {
            "osascript failed".to_string()
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A new iTerm2 window running `line`. A window of its own rather than a tab of the current
/// one: the person may be in another application, or have no window open at all.
pub(super) fn iterm_attach_script(line: &str) -> String {
    format!(
        "tell application \"iTerm2\"\n  \
           activate\n  \
           set newWindow to (create window with default profile)\n  \
           tell current session of newWindow\n    \
             write text \"{}\"\n  \
           end tell\n\
         end tell",
        applescript_literal(line)
    )
}

/// Run `line` in a new iTerm2 window.
pub fn iterm_attach(line: &str) -> Result<String, String> {
    osascript(&iterm_attach_script(line))
}

/// Whether iTerm2 is installed, by looking for the application. A path test and not a launch
/// services query, because the board asks on every poll.
pub fn iterm_available() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty());
    std::path::Path::new("/Applications/iTerm.app").is_dir()
        || home.is_some_and(|h| {
            std::path::Path::new(&h)
                .join("Applications/iTerm.app")
                .is_dir()
        })
}
