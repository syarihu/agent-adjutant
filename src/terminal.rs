//! Opening a tab somewhere, bringing one to the front, and closing one when its work is
//! over.
//!
//! iTerm2 is the built-in because it needs no setup on the machine this grew up on, but it
//! is only a default: `terminal.spawn` / `terminal.focus` in the config replace it with any
//! command line, so tmux, WezTerm, Ghostty or a plain `open -a` all work without this file
//! learning about them.
//!
//! `focus`, `title` and `wake` are optional in a way `spawn` is not. Failing to open a tab
//! loses the work; failing to raise a window, name it or poke it loses nothing, so an
//! unsupported one is a quiet no-op rather than an error. `close` belongs with `spawn`
//! rather than with those: whoever asked for it is about to remove the worktree that tab is
//! sitting in, so a failure nobody was told about is a worker killed by the cleanup.

use std::process::Command;

use crate::config::{Hook, Wake};
use crate::template::{Sub, contains_placeholder, render, sh_quote};

pub struct SpawnRequest<'a> {
    pub cwd: &'a str,
    pub title: &'a str,
    /// An already-assembled shell command line. Built by the caller (see `runner`) so that
    /// quoting happens once, at the point that knows what the parts mean.
    pub command: &'a str,
    /// A command that names the tab, to run inside it before the real one. Used only where
    /// `{command}` is a shell line; a terminal that titles its own tabs does that with
    /// `{title}` instead. `None` when nothing should name it.
    pub title_command: Option<&'a str>,
}

/// What a spawn or focus did, or — under `--dry-run` — would have done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Performed {
    pub description: String,
    pub script: String,
    pub ran: bool,
}

pub fn spawn(
    template: Option<&str>,
    req: &SpawnRequest,
    dry_run: bool,
) -> Result<Performed, String> {
    if !std::path::Path::new(req.cwd).is_dir() {
        return Err(format!("no such directory: {}", req.cwd));
    }
    if req.command.trim().is_empty() {
        return Err("the command to run is empty".to_string());
    }
    let title = sanitise_title(req.title, default_title(req.cwd));
    // `{cwd}` in the template is the signal for what `{command}` *is*.
    //
    // A template that asks for the directory separately (`tmux new-window -c {cwd} …
    // {command}`) is putting `{command}` in an argv slot, so it has to receive one plain
    // command: prepending `cd … &&` or `( … ) &&` there is a shell syntax error, and every
    // spawn template in the shipped documentation looks like that. Such a terminal also
    // names its own tabs, through `{title}`.
    //
    // A template with no `{cwd}` — and the built-in, which types into a shell — is taking a
    // shell line, so it gets the `cd` and the tab naming in front of the command.
    let takes_argv = template
        .map(|t| contains_placeholder(t, "cwd"))
        .unwrap_or(false);
    let line = if takes_argv {
        req.command.to_string()
    } else {
        let mut line = format!("cd {} && ", sh_quote(req.cwd));
        if let Some(name_it) = req.title_command {
            // A title is decoration: a machine where naming fails still has to get the
            // work started.
            line.push_str(&format!("( {name_it} || true ) && "));
        }
        line.push_str(req.command);
        line
    };

    match template {
        Some(template) => {
            let cmd = render(
                template,
                &[
                    ("cwd", Sub::Quoted(req.cwd)),
                    ("title", Sub::Quoted(&title)),
                    ("command", Sub::Raw(&line)),
                ],
            );
            if dry_run {
                return Ok(Performed {
                    description: format!("will start in a new tab: {title} ({})", req.cwd),
                    script: cmd,
                    ran: false,
                });
            }
            run_shell(&cmd)?;
            Ok(Performed {
                description: format!("started in a new tab: {title} ({})", req.cwd),
                script: cmd,
                ran: true,
            })
        }
        None => {
            let script = iterm_spawn_script(&line);
            if dry_run {
                return Ok(Performed {
                    description: format!("will start in a new tab: {title} ({})", req.cwd),
                    script,
                    ran: false,
                });
            }
            osascript(&script)?;
            Ok(Performed {
                description: format!("started in a new tab: {title} ({})", req.cwd),
                script,
                ran: true,
            })
        }
    }
}

pub fn focus(
    template: Option<&str>,
    pid: u32,
    title: &str,
    dry_run: bool,
) -> Result<Performed, String> {
    let tty = tty_of(pid);
    if let Some(template) = template {
        let cmd = render(
            template,
            &[
                ("pid", Sub::Quoted(&pid.to_string())),
                ("tty", Sub::Quoted(tty.as_deref().unwrap_or(""))),
                ("title", Sub::Quoted(title)),
            ],
        );
        if !dry_run {
            run_shell(&cmd)?;
        }
        return Ok(Performed {
            description: match dry_run {
                true => format!("will focus the tab (pid {pid})"),
                // It already has. The future tense here was printed after the command had
                // run, which reads as "about to" to whoever is looking for what happened.
                false => format!("focused the tab (pid {pid})"),
            },
            script: cmd,
            ran: !dry_run,
        });
    }
    let Some(tty) = tty else {
        return Ok(Performed {
            description: format!("no terminal found for pid {pid}; not focusing"),
            script: String::new(),
            ran: false,
        });
    };
    let script = iterm_focus_script(&tty);
    if dry_run {
        return Ok(Performed {
            description: format!("will focus the tab (pid {pid}, {tty})"),
            script,
            ran: false,
        });
    }
    // A window that cannot be raised is not worth failing a send over.
    let _ = osascript(&script);
    Ok(Performed {
        description: format!("focused the tab (pid {pid}, {tty})"),
        script,
        ran: true,
    })
}

/// What the built-in close prints when it found the tab and asked it to close.
///
/// The same device `wake` uses, for the same reason: walking every window and matching no
/// tty is a script that ran to the end and exited 0, indistinguishable from the one that
/// reached a session — so the one path that reached a session says so out loud.
///
/// What it does *not* say is that the tab is gone. iTerm2 can be set to confirm closing a
/// session with a process still in it, and cancelling that dialog is not an AppleScript
/// error: the script goes on and prints this. So the marker rules out "there was no such
/// tab", and nothing more. Whether the worker actually died is a question about the
/// process, and it is asked one layer up, by the caller that acts on the answer.
const CLOSED_MARKER: &str = "adjutant:closed";

/// Did the command report that it closed something? A template answers for itself with its
/// exit status — it is someone else's command and only it knows what success means there.
/// The built-in knows more about itself than an exit status can carry, and prints it.
fn reported_closing(built_in: bool, output: &str) -> bool {
    !built_in || output.contains(CLOSED_MARKER)
}

/// Close the tab a session is sitting in.
///
/// The end of a task rather than a courtesy: closing the tab hangs up the session, which is
/// what the caller is after before it removes the worktree underneath.
///
/// `ran` means the close command reported doing something — not that the process in that
/// tab is gone. Nothing at this layer can establish the second: a confirmation dialog and a
/// template pointed at the wrong pane both produce a perfectly successful close command. A
/// caller that is about to delete something has to ask the process itself.
pub fn close(hook: &Hook, pid: u32, title: &str, dry_run: bool) -> Result<Performed, String> {
    close_with(run_shell, tty_of(pid), hook, pid, title, dry_run)
}

/// The same, with the thing that runs the command handed in.
///
/// Split out for the reason `wake_with` is: the decision about whether a tab was actually
/// closed cannot be reached by a test, which has no iTerm2 session of its own to lose — and
/// that decision is the whole defect this shape exists to prevent.
fn close_with(
    run: impl Fn(&str) -> Result<String, String>,
    tty: Option<String>,
    hook: &Hook,
    pid: u32,
    title: &str,
    dry_run: bool,
) -> Result<Performed, String> {
    if hook.is_off() {
        // Off is an answer, not an absence: somebody said not to close tabs here, and the
        // built-in would otherwise close one. `ran` stays false, which is what stops the
        // caller from treating the worktree as finished with.
        return Ok(Performed {
            description: "closing tabs is turned off; the tab was left open".to_string(),
            script: String::new(),
            ran: false,
        });
    }
    let mut built_in = false;
    let command = match hook.template() {
        Some(template) => render(
            template,
            &[
                ("pid", Sub::Quoted(&pid.to_string())),
                ("tty", Sub::Quoted(tty.as_deref().unwrap_or(""))),
                ("title", Sub::Quoted(title)),
            ],
        ),
        None => match tty.as_deref() {
            Some(tty) => {
                built_in = true;
                // Through a shell line rather than `osascript`'s stdin, the way `wake` goes:
                // what the script prints is the answer here, and one runner for both paths
                // is what lets a test supply that answer.
                format!("osascript -e {}", sh_quote(&iterm_close_script(tty)))
            }
            None => {
                return Ok(Performed {
                    description: format!("no terminal found for pid {pid}; not closing"),
                    script: String::new(),
                    ran: false,
                });
            }
        },
    };
    if dry_run {
        return Ok(Performed {
            description: format!("will close the tab (pid {pid})"),
            script: command,
            ran: false,
        });
    }
    // Not swallowed the way `focus` and `wake` swallow their own failures. A window that
    // would not come forward costs a person one click, and a bell that did not ring loses
    // nothing that was not already delivered; a tab that would not close is a live worker in
    // a worktree the caller is about to delete, so the caller has to hear about it.
    let out = run(&command)?;
    if !reported_closing(built_in, &out) {
        return Ok(Performed {
            description: format!("no tab of this terminal is running pid {pid}; nothing closed"),
            script: command,
            ran: false,
        });
    }
    Ok(Performed {
        description: format!("closed the tab (pid {pid})"),
        script: command,
        ran: true,
    })
}

/// The controlling terminal of a process, as `ttys004`. `None` means it has none.
pub fn tty_of(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-o", "tty=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    tty_in(&String::from_utf8_lossy(&out.stdout))
}

/// The tty in what `ps -o tty=` printed, if it named one.
///
/// Split out because the two systems this runs on spell "none" differently — `??` on macOS,
/// `?` on Linux — and a machine only ever demonstrates its own. Taken for a tty name, either
/// one sends every caller looking through a terminal's tabs for `/dev/?`, which no session
/// can be sitting on: `close` would then report having closed nothing, and say so as a
/// failure the cleanup stops on.
fn tty_in(printed: &str) -> Option<String> {
    match printed.trim() {
        "" | "?" | "??" => None,
        tty => Some(tty.to_string()),
    }
}

// ── naming the tab this process is in ────────────────────────────────

/// OSC 0 — the escape sequence every terminal worth using understands. Written to the tty
/// rather than to stdout: stdout is whatever captured this process, which for an agent's
/// shell tool is a pipe going back into the transcript.
fn default_title_command(title: &str, tty: &str) -> String {
    format!(
        "printf '\\033]0;%s\\007' {} > /dev/{}",
        sh_quote(title),
        tty
    )
}

/// Name the tab this process is running in.
///
/// Distinct from the title `spawn` gives a tab it is creating: this one is for a session
/// naming *itself*, which is the only way a hub's own tab gets a name.
pub fn set_title(hook: &Hook, title: &str, dry_run: bool) -> Result<Performed, String> {
    if hook.is_off() {
        return Ok(Performed {
            description: "tab naming is turned off".to_string(),
            script: String::new(),
            ran: false,
        });
    }
    let title = sanitise_title(title, "adjutant".to_string());
    let command = match hook.template() {
        Some(template) => render(template, &[("title", Sub::Quoted(&title))]),
        None => match own_tty() {
            Some(tty) => default_title_command(&title, &tty),
            None => {
                return Ok(Performed {
                    description: "no terminal found for this process; not naming the tab"
                        .to_string(),
                    script: String::new(),
                    ran: false,
                });
            }
        },
    };
    if !dry_run {
        // A tab with the wrong name is a cosmetic problem; failing the caller over it is not.
        let _ = run_shell(&command);
    }
    Ok(Performed {
        description: format!("named the tab: {title}"),
        script: command,
        ran: !dry_run,
    })
}

/// This process's terminal, found by walking up the process tree.
///
/// A tool invoked by an agent often has no controlling terminal of its own, but one of its
/// ancestors is the session sitting in the tab — that is the one being named.
pub fn own_tty() -> Option<String> {
    let mut pid = std::process::id();
    for _ in 0..16 {
        if let Some(tty) = tty_of(pid)
            && std::path::Path::new(&format!("/dev/{tty}")).exists()
        {
            return Some(tty);
        }
        pid = parent_of(pid)?;
        if pid <= 1 {
            return None;
        }
    }
    None
}

fn parent_of(pid: u32) -> Option<u32> {
    let out = Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

// ── waking a running session ─────────────────────────────────────────

/// Type a line into the tab a running session is sitting in.
///
/// iTerm2's `write text` is the built-in because it is the one mechanism that actually
/// reaches an interactive agent: the message goes to its prompt exactly as if the person
/// had typed it. Any other terminal with a scripting interface can be dropped in as a
/// template.
/// What the built-in wake prints when it actually typed into a session. Its absence is the
/// only way to tell "typed into the tab" from "walked every window and found no such tab":
/// both are a script that ran to the end and exited 0.
const WOKE_MARKER: &str = "adjutant:woke";

/// Did the command actually reach a session? A template answers for itself with its exit
/// status; the built-in has to say so out loud, because running to the end having found
/// nothing is indistinguishable from running to the end having typed a line.
fn woke(built_in: bool, output: &str) -> bool {
    !built_in || output.contains(WOKE_MARKER)
}

fn default_wake_command(tty: &str, line: &str) -> String {
    let script = format!(
        "tell application \"iTerm2\"\n  \
           repeat with w in windows\n    \
             repeat with t in tabs of w\n      \
               repeat with s in sessions of t\n        \
                 if tty of s is \"/dev/{}\" then\n          \
                   tell s to write text \"{}\"\n          \
                   return \"{WOKE_MARKER}\"\n        \
                 end if\n      \
               end repeat\n    \
             end repeat\n  \
           end repeat\n\
         end tell\n\
         return \"\"",
        applescript_literal(tty),
        applescript_literal(line)
    );
    format!("osascript -e {}", sh_quote(&script))
}

/// What a woken session is told. Deliberately an instruction rather than a copy of the
/// message: the message is already in the box, and re-typing it into a prompt would put the
/// same text in two places with only one of them acked.
///
/// The two directions read different boxes, so they are told different things — one line
/// for both would send half the sessions to look in the wrong place.
/// Both name the tool *and* the command. Observed on a real session: a deferred MCP tool
/// costs several lookups before it can be called, and an agent without MCP at all cannot
/// call it — while every agent can run a command. Naming one way in is a session that
/// stalls on the machine where that way is missing.
pub const HUB_WAKE_LINE: &str =
    "受信箱に届いたのだ。adjutant_pending（無ければ `adj pending`）で確認して処理するのだ";
pub const WORKER_WAKE_LINE: &str =
    "hub から連絡が来たのだ。adjutant_outbox（無ければ `adj outbox`）で確認して処理するのだ";

/// `default_line` is what to type when the config has not overridden it — the caller knows
/// which direction this is, and the two directions read different boxes.
pub fn wake(
    wake: &Wake,
    pid: u32,
    subject: &str,
    default_line: &str,
    dry_run: bool,
) -> Result<Performed, String> {
    wake_with(
        run_shell,
        tty_of(pid),
        wake,
        pid,
        subject,
        default_line,
        dry_run,
    )
}

/// The same, with the thing that runs the command handed in.
///
/// Split out so a test can reach the branch that decides whether anything was woken: the
/// built-in path ends in `osascript` on a tty this process does not have, so a test that
/// cannot supply both can only check the helpers *around* the decision — and the defect
/// this exists to prevent was in the decision itself.
fn wake_with(
    run: impl Fn(&str) -> Result<String, String>,
    tty: Option<String>,
    wake: &Wake,
    pid: u32,
    subject: &str,
    default_line: &str,
    dry_run: bool,
) -> Result<Performed, String> {
    let hook = &wake.hook;
    let line = wake.line_or(default_line);
    if hook.is_off() {
        return Ok(Performed {
            description: "waking is turned off".to_string(),
            script: String::new(),
            ran: false,
        });
    }
    // A template is answered for by its exit status and nothing else — it is someone else's
    // command and only it knows what success means. The built-in knows more about itself
    // than that, and says so.
    let mut built_in = false;
    let command = match hook.template() {
        Some(template) => render(
            template,
            &[
                ("pid", Sub::Quoted(&pid.to_string())),
                ("tty", Sub::Quoted(tty.as_deref().unwrap_or(""))),
                ("subject", Sub::Quoted(subject)),
                ("line", Sub::Quoted(line)),
            ],
        ),
        None => match tty {
            Some(tty) => {
                built_in = true;
                default_wake_command(&tty, line)
            }
            None => {
                return Ok(Performed {
                    description: format!("no terminal found for pid {pid}; nothing to wake"),
                    script: String::new(),
                    ran: false,
                });
            }
        },
    };
    if dry_run {
        return Ok(Performed {
            description: format!("will wake the session (pid {pid})"),
            script: command,
            ran: false,
        });
    }
    match run(&command) {
        // The message is already delivered by the time this runs. Failing to ring the bell
        // must not turn a successful send into an error.
        Err(e) => Ok(Performed {
            description: format!("cannot wake the session: {e}"),
            script: command,
            ran: false,
        }),
        // The built-in walks every iTerm2 window looking for one tty and quietly does
        // nothing when no tab has it — which is what a session in any other terminal looks
        // like. Reading that as "woke it" was the expensive half: `ran` also decides whether
        // to fall back to a notification, so the one person who needed telling was the one
        // who was not told.
        Ok(out) if !woke(built_in, &out) => Ok(Performed {
            description: format!("no tab of this terminal is running pid {pid}; nothing woken"),
            script: command,
            ran: false,
        }),
        Ok(_) => Ok(Performed {
            description: format!("woke the session (pid {pid})"),
            script: command,
            ran: true,
        }),
    }
}

// ── titles ───────────────────────────────────────────────────────────

fn default_title(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string())
}

/// Newlines are the dangerous character here: one inside an AppleScript string literal
/// submits the half-typed command in the new tab. Quotes and backslashes survive, because
/// `applescript_literal` escapes them properly.
pub fn sanitise_title(title: &str, fallback: String) -> String {
    let collapsed: Vec<&str> = title.split_whitespace().collect();
    let title = collapsed.join(" ");
    let title = if title.is_empty() { fallback } else { title };
    if display_width(&title) <= 30 {
        return title;
    }
    let mut out = String::new();
    for c in title.chars() {
        if display_width(&out) + char_width(c) > 28 {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

fn char_width(c: char) -> usize {
    // East Asian Wide and Fullwidth take two cells. The ranges here are the ones a task
    // title actually lands in: CJK, kana, and fullwidth punctuation.
    let c = c as u32;
    let wide = (0x1100..=0x115F).contains(&c)
        || (0x2E80..=0x303E).contains(&c)
        || (0x3041..=0x33FF).contains(&c)
        || (0x3400..=0x4DBF).contains(&c)
        || (0x4E00..=0x9FFF).contains(&c)
        || (0xA000..=0xA4CF).contains(&c)
        || (0xAC00..=0xD7A3).contains(&c)
        || (0xF900..=0xFAFF).contains(&c)
        || (0xFE30..=0xFE6F).contains(&c)
        || (0xFF00..=0xFF60).contains(&c)
        || (0xFFE0..=0xFFE6).contains(&c)
        || (0x1F300..=0x1FAFF).contains(&c);
    if wide { 2 } else { 1 }
}

fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

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
fn iterm_spawn_script(line: &str) -> String {
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

fn iterm_focus_script(tty: &str) -> String {
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
fn iterm_close_script(tty: &str) -> String {
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

fn osascript(script: &str) -> Result<String, String> {
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

pub fn run_shell(command: &str) -> Result<String, String> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| format!("cannot run the command: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("command failed: {command}")
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_with_a_newline_cannot_submit_a_line_in_the_new_tab() {
        let title = sanitise_title("WID-957\nrm -rf /", "fallback".into());
        assert_eq!(title, "WID-957 rm -rf /");
        // One newline inside the AppleScript string literal would run `rm -rf /` in the new
        // tab, so the check is on the generated script, not just on the title.
        let script = iterm_spawn_script(&format!(
            "cd /tmp && tmux rename-window {}",
            sh_quote(&title)
        ));
        assert_eq!(script.lines().filter(|l| l.contains("rm -rf /")).count(), 1);
        assert!(script.contains("'WID-957 rm -rf /'"), "{script}");
    }

    #[test]
    fn a_quote_in_a_title_is_escaped_rather_than_dropped() {
        assert_eq!(applescript_literal(r#"it"s \ here"#), r#"it\"s \\ here"#);
    }

    #[test]
    fn a_long_title_is_truncated_by_display_width_not_byte_count() {
        let title = sanitise_title(&"あ".repeat(40), "fallback".into());
        assert!(display_width(&title) <= 30, "{title}");
        assert!(title.ends_with('…'));
    }

    #[test]
    fn a_short_title_is_left_exactly_as_it_is() {
        assert_eq!(
            sanitise_title("WID-957 検索が潰れる", "x".into()),
            "WID-957 検索が潰れる"
        );
    }

    #[test]
    fn an_empty_title_falls_back_to_the_directory_name() {
        assert_eq!(sanitise_title("   ", "widget".into()), "widget");
    }

    #[test]
    fn the_builtin_spawn_cds_first_and_keeps_the_command_in_one_piece() {
        let out = spawn(
            None,
            &SpawnRequest {
                cwd: "/tmp",
                title: "WID-957",
                command: "claude 'go now'",
                title_command: None,
            },
            true,
        )
        .unwrap();
        assert!(!out.ran);
        assert!(
            out.script.contains("cd /tmp && claude 'go now'"),
            "{}",
            out.script
        );
        assert!(out.script.contains("create tab with default profile"));
        // Naming happens in the tab, not through the terminal's API.
        assert!(!out.script.contains("set name to"), "{}", out.script);
    }

    #[test]
    fn a_terminal_template_receives_the_whole_command_as_one_shell_line() {
        let out = spawn(
            Some("tmux new-window -c {cwd} -n {title} {command}"),
            &SpawnRequest {
                cwd: "/tmp",
                title: "WID-957",
                command: "claude 'go now'",
                title_command: None,
            },
            true,
        )
        .unwrap();
        // No `cd` in front: the template said `-c {cwd}`, and doubling up would hand tmux
        // `cd` as the command to run.
        assert_eq!(
            out.script,
            "tmux new-window -c /tmp -n WID-957 claude 'go now'"
        );
    }

    #[test]
    fn every_documented_spawn_template_produces_a_valid_command() {
        // Each of these puts {command} in an argv slot: the three the shipped example
        // config offers, plus two a person would plausibly write. Prepending `cd … &&`
        // or `( … ) &&` there is a syntax error, so this asserts on a real shell's
        // verdict rather than on the string's shape.
        for template in [
            "tmux new-window -d -c {cwd} -n {title} {command}",
            "ghostty --working-directory={cwd} -e {command}",
            "wezterm cli spawn --cwd {cwd} -- {command}",
            "sh -c '{command}'",
            "open -a Terminal {command}",
        ] {
            let done = spawn(
                Some(template),
                &SpawnRequest {
                    cwd: "/tmp",
                    title: "WID-1",
                    command: "echo hi",
                    title_command: Some("name-it --title WID-1"),
                },
                true,
            )
            .unwrap();
            let checked = std::process::Command::new("sh")
                .args(["-n", "-c", &done.script])
                .output()
                .unwrap();
            assert!(
                checked.status.success(),
                "{template} produced invalid shell: {}\n{}",
                done.script,
                String::from_utf8_lossy(&checked.stderr)
            );
        }
    }

    #[test]
    fn a_template_taking_an_argv_gets_one_plain_command() {
        // `{cwd}` says the terminal handles the directory, so it is putting {command} in an
        // argv slot: no `cd`, and no tab naming either — that terminal names its own tabs
        // through `{title}`.
        let done = spawn(
            Some("tmux new-window -c {cwd} -n {title} {command}"),
            &SpawnRequest {
                cwd: "/tmp",
                title: "WID-1",
                command: "echo hi",
                title_command: Some("name-it --title WID-1"),
            },
            true,
        )
        .unwrap();
        assert_eq!(done.script, "tmux new-window -c /tmp -n WID-1 echo hi");
    }

    #[test]
    fn a_terminal_template_with_no_cwd_placeholder_gets_a_cd_instead() {
        let out = spawn(
            Some("open -a Terminal {command}"),
            &SpawnRequest {
                cwd: "/tmp",
                title: "WID-957",
                command: "claude 'go now'",
                title_command: Some("name-it"),
            },
            true,
        )
        .unwrap();
        assert_eq!(
            out.script,
            "open -a Terminal cd /tmp && ( name-it || true ) && claude 'go now'"
        );
    }

    #[test]
    fn spawning_into_a_directory_that_is_not_there_fails_before_opening_anything() {
        let err = spawn(
            None,
            &SpawnRequest {
                cwd: "/definitely/not/here",
                title: "x",
                command: "true",
                title_command: None,
            },
            true,
        )
        .unwrap_err();
        assert!(err.contains("no such directory"), "{err}");
    }

    #[test]
    fn focus_without_a_template_on_a_pid_with_no_terminal_is_a_quiet_no_op() {
        // pid 1 has no controlling terminal on macOS or Linux.
        let out = focus(None, 1, "hub", true).unwrap();
        assert!(!out.ran);
    }

    #[test]
    fn a_process_with_no_terminal_is_recognised_on_either_system() {
        // The spelling is the system's, not the process's: macOS prints `??` where Linux
        // prints `?`, and whichever machine this is running on can only show one of them.
        assert_eq!(tty_in("??\n"), None);
        assert_eq!(tty_in("?\n"), None);
        assert_eq!(tty_in(""), None);
        assert_eq!(tty_in("ttys004\n"), Some("ttys004".into()));
        assert_eq!(tty_in(" pts/3 \n"), Some("pts/3".into()));
    }

    #[test]
    fn closing_without_a_template_on_a_pid_with_no_terminal_is_a_quiet_no_op() {
        // Same shape as `focus`: pid 1 has no controlling terminal, and there is no tab to
        // dispose of for a session nobody can locate.
        let out = close(&Hook::BuiltIn, 1, "WID-957", true).unwrap();
        assert!(!out.ran);
        assert!(out.description.contains("not closing"), "{out:?}");
        assert!(out.script.is_empty(), "{out:?}");
    }

    #[test]
    fn closing_can_be_turned_off_and_then_reaches_nothing() {
        // `false` is a different answer from an absent key, and the difference is the whole
        // point here: read as unset, the built-in would dispose of the very tab somebody
        // had just declared off limits. `ran` has to stay false too, or the caller reads
        // "turned off" as "the tab is gone" and carries on removing the worktree.
        let done = close_with(
            |_| panic!("a close that is turned off ran a command"),
            Some("ttys004".to_string()),
            &Hook::Off,
            std::process::id(),
            "WID-957",
            false,
        )
        .unwrap();
        assert!(!done.ran, "{done:?}");
        assert!(done.script.is_empty(), "{done:?}");
        assert!(done.description.contains("turned off"), "{done:?}");
    }

    #[test]
    fn a_close_template_is_handed_the_pid_and_the_tty_and_runs_only_for_real() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("closed");
        // Quoted, because a `TMPDIR` with a space in it is the machine's business and not a
        // defect in what is under test — unquoted, this test failed on a correct `close`.
        let template = format!(
            "printf '%s|%s' {{pid}} {{tty}} > {}",
            sh_quote(&marker.to_string_lossy())
        );

        let template = Hook::Command(template);
        let planned = close(&template, 4321, "WID-957", true).unwrap();
        assert!(!planned.ran);
        assert!(planned.script.contains("4321"), "{}", planned.script);
        // A pid with no terminal still substitutes, as the empty string. Left standing, the
        // literal `{tty}` would be handed to a shell as an argument.
        assert!(!planned.script.contains("{tty}"), "{}", planned.script);
        assert!(!marker.exists(), "a dry run ran the template");

        let ours = std::process::id();
        let done = close(&template, ours, "WID-957", false).unwrap();
        assert!(done.ran);
        let recorded = std::fs::read_to_string(&marker).unwrap();
        let (pid, tty) = recorded.split_once('|').unwrap();
        assert_eq!(pid, ours.to_string());
        // Whatever this test is running under — a terminal or a pipe with no tty at all —
        // the template is given the answer for the pid it was asked about.
        assert_eq!(tty, tty_of(ours).unwrap_or_default());
    }

    /// The finding, one command over from where it was found the first time: the built-in
    /// walks every iTerm2 window and, having matched no tty, runs to the end and exits 0 —
    /// which is what a worker in any other terminal, in tmux, or over ssh looks like. So
    /// `ran` has to mean "the command reported reaching a session", and a walk that matched
    /// nothing has to be `false`.
    ///
    /// Driven through `close_with` rather than through the helper, so the branch that reads
    /// the answer cannot be deleted with this test still passing.
    #[test]
    fn the_builtin_close_only_reports_success_when_it_reached_a_session() {
        let ours = std::process::id();

        let quiet = close_with(
            |_| Ok(String::new()),
            Some("ttys004".to_string()),
            &Hook::BuiltIn,
            ours,
            "WID-957",
            false,
        )
        .unwrap();
        assert!(!quiet.ran, "{quiet:?}");
        assert!(quiet.description.contains("nothing closed"), "{quiet:?}");

        // The same command, having closed a session, says so.
        let done = close_with(
            |_| Ok(CLOSED_MARKER.to_string()),
            Some("ttys004".to_string()),
            &Hook::BuiltIn,
            ours,
            "WID-957",
            false,
        )
        .unwrap();
        assert!(done.ran, "{done:?}");

        // A configured template is answered for by its exit status alone.
        let template = close_with(
            |_| Ok(String::new()),
            None,
            &Hook::Command("close-tab --pid {pid}".to_string()),
            ours,
            "WID-957",
            false,
        )
        .unwrap();
        assert!(template.ran, "{template:?}");

        // A command that failed is not a closed tab, and unlike `wake` it is not swallowed.
        let failed = close_with(
            |_| Err("no iTerm2 window is open".to_string()),
            Some("ttys004".to_string()),
            &Hook::BuiltIn,
            ours,
            "WID-957",
            false,
        );
        assert!(failed.is_err(), "{failed:?}");

        // The contract the two halves share, checked where it can actually break: the
        // marker has to sit *inside* the branch that matched the tty. Hoisted out of that
        // branch it would be printed by a walk that closed nothing — which is the defect
        // itself — and an assertion that only knew the marker came after `close s` would
        // have passed anyway.
        let script = iterm_close_script("ttys004");
        let matched = script.find("if tty of s is \"/dev/ttys004\" then").unwrap();
        let closes = script.find("close s").unwrap();
        let marked = script.find(CLOSED_MARKER).unwrap();
        let branch_ends = script.find("end if").unwrap();
        assert!(matched < closes, "{script}");
        assert!(closes < marked, "{script}");
        assert!(marked < branch_ends, "{script}");
        // And the walk that matched nothing has to fall out saying nothing.
        assert!(script.trim_end().ends_with("return \"\""), "{script}");
    }

    #[test]
    fn naming_this_tab_is_a_template_like_everything_else() {
        let done = set_title(
            &Hook::Command("tmux rename-window {title}".into()),
            "🗂 hub widget",
            true,
        )
        .unwrap();
        assert_eq!(done.script, "tmux rename-window '🗂 hub widget'");
    }

    #[test]
    fn the_builtin_title_writes_to_a_terminal_rather_than_to_stdout() {
        // stdout is whatever captured the process — for an agent's shell tool, the
        // transcript. The escape sequence has to reach the tty or it is just noise.
        let done = set_title(&Hook::BuiltIn, "hub", true).unwrap();
        if !done.script.is_empty() {
            assert!(done.script.contains("> /dev/"), "{}", done.script);
            assert!(done.script.contains("033]0;"), "{}", done.script);
        }
    }

    #[test]
    fn a_title_that_is_turned_off_runs_nothing_at_all() {
        let done = set_title(&Hook::Off, "hub", false).unwrap();
        assert!(!done.ran);
        assert!(done.script.is_empty());
    }

    #[test]
    fn waking_a_hub_is_a_template_and_can_be_turned_off() {
        let done = wake(
            &Wake {
                hook: Hook::Command("tmux send-keys -t {tty} {line} Enter".into()),
                line: None,
            },
            4321,
            "画像が潰れる",
            HUB_WAKE_LINE,
            true,
        )
        .unwrap();
        assert!(
            done.script.starts_with("tmux send-keys -t "),
            "{}",
            done.script
        );
        assert!(done.script.contains(HUB_WAKE_LINE), "{}", done.script);

        let off = wake(
            &Wake {
                hook: Hook::Off,
                line: None,
            },
            4321,
            "s",
            HUB_WAKE_LINE,
            false,
        )
        .unwrap();
        assert!(!off.ran);
        assert!(off.script.is_empty());
    }

    #[test]
    fn each_direction_is_pointed_at_its_own_box() {
        // The message is already in the box. Typing it into the prompt too would put the
        // same text in two places, and only one of them gets acked — so the line is an
        // instruction, and it has to name the box that direction actually reads.
        assert!(
            HUB_WAKE_LINE.contains("adjutant_pending") && HUB_WAKE_LINE.contains("adj pending")
        );
        assert!(
            WORKER_WAKE_LINE.contains("adjutant_outbox") && WORKER_WAKE_LINE.contains("adj outbox")
        );
        assert_ne!(HUB_WAKE_LINE, WORKER_WAKE_LINE);
    }

    #[test]
    fn the_sentence_can_be_replaced_without_restating_how_to_poke() {
        // The default names MCP tools. An agent that only has the CLI, or one that wants a
        // slash command, needs a different sentence through the same terminal.
        let done = wake(
            &Wake {
                hook: Hook::Command("tmux send-keys -t {tty} {line} Enter".into()),
                line: Some("check adjutant pending".into()),
            },
            4321,
            "s",
            HUB_WAKE_LINE,
            true,
        )
        .unwrap();
        assert!(
            done.script.contains("check adjutant pending"),
            "{}",
            done.script
        );
        assert!(!done.script.contains("adjutant_pending"), "{}", done.script);
    }

    #[test]
    fn waking_a_pid_with_no_terminal_is_a_quiet_no_op() {
        let done = wake(&Wake::default(), 1, "s", HUB_WAKE_LINE, false).unwrap();
        assert!(!done.ran);
    }

    #[test]
    fn a_focus_template_gets_the_pid() {
        let out = focus(Some("raise-tab --pid {pid}"), 4321, "hub", true).unwrap();
        assert_eq!(out.script, "raise-tab --pid 4321");
    }

    /// The finding: a worker in any terminal other than iTerm2 was told it had been woken,
    /// and because `ran` is also what suppresses the fallback notification, nobody was told
    /// anything at all. Driven through `wake` itself — checking only the helper let the
    /// branch that reads it be deleted with this test still passing.
    #[test]
    fn the_builtin_wake_only_claims_success_when_it_typed_something() {
        let built_in = Wake::default();
        let ours = std::process::id();

        // The built-in, having walked every window and found no matching tab, prints
        // nothing and exits 0 — which is what a session in another terminal looks like.
        let quiet = wake_with(
            |_| Ok(String::new()),
            Some("ttys004".to_string()),
            &built_in,
            ours,
            "s",
            "check your inbox",
            false,
        )
        .unwrap();
        assert!(!quiet.ran, "{quiet:?}");
        assert!(quiet.description.contains("nothing woken"), "{quiet:?}");

        // The same command, having typed into a session, says so.
        let woken = wake_with(
            |_| Ok(WOKE_MARKER.to_string()),
            Some("ttys004".to_string()),
            &built_in,
            ours,
            "s",
            "check your inbox",
            false,
        )
        .unwrap();
        assert!(woken.ran, "{woken:?}");

        // A configured template is answered for by its exit status: it is someone else's
        // command and only it knows what success means there.
        let template = Wake {
            hook: crate::config::Hook::Command("true {pid}".to_string()),
            line: None,
        };
        let ran = wake_with(
            |_| Ok(String::new()),
            None,
            &template,
            ours,
            "s",
            "check your inbox",
            false,
        )
        .unwrap();
        assert!(ran.ran, "{ran:?}");

        // The contract the two halves share: the marker is printed on the one path that
        // types into a session, and the fall-through returns nothing.
        let script = default_wake_command("ttys004", "check your inbox");
        let typed = script.find("write text").unwrap();
        let marked = script.find(WOKE_MARKER).unwrap();
        assert!(marked > typed, "{script}");
        assert!(script.trim_end().ends_with("return \"\"'"), "{script}");
    }
}
