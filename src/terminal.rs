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

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::config::{TerminalSettings, Wake};
use crate::prompts::Agent;
use crate::session::SessionTerminal;
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
    /// The built-in tmux wake declined because of what the agent's screen showed, or could
    /// not confirm what it typed. Only then is the reason worth telling the person; every
    /// other failure is reported as it always was.
    pub screen: bool,
}

/// A pane inside a tmux window.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TmuxPane {
    pub pane_id: String,
    pub pane_pid: u32,
    pub pane_tty: String,
    pub window_id: String,
    pub session_name: String,
    pub window_index: u32,
    pub window_name: String,
    /// `#{window_activity}`: epoch seconds of the window's last activity. None on a line that
    /// has no such field (an older listing) or one that is not a number.
    #[serde(default)]
    pub window_activity: Option<i64>,
}

/// A client attached to a tmux server, as `list-clients` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxClient {
    pub session: String,
    /// A control-mode client (iTerm2's `-CC`), which shows every window of its session rather
    /// than one.
    pub control: bool,
    /// The window the client is looking at.
    pub window_id: String,
}

/// How long a command line may be before it is staged in a file instead of typed.
///
/// The built-in spawner hands the line to the terminal to type into a shell, and a long one
/// does not fail — it *arrives corrupted*, a chunk dropped somewhere in the middle, and what
/// runs is whatever that mangling happened to spell. Seen with an `agentEnv` carrying a
/// PATH: the tab ran a command containing `/usrdj-e2e/config.json`, which named no file
/// anybody could search for.
///
/// The exact limit is a property of the terminal and is not documented by any of them, so
/// this is deliberately well under any of the numbers involved rather than tuned to one.
const MAX_INLINE_COMMAND: usize = 900;

/// Put a long command line in a file and return the short one that runs it.
///
/// Left behind rather than self-deleting: the script is read by a shell in another process
/// at a time we do not get to know, and a file removed before that read is a tab that opens
/// on an error. Old ones are swept on the way past instead.
fn stage_command(line: &str) -> Result<String, String> {
    let dir = std::env::temp_dir();
    sweep_staged(&dir);
    let name = format!(
        "adjutant-spawn-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let path = dir.join(name);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        file.write_all(format!("#!/bin/sh\n{line}\n").as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, format!("#!/bin/sh\n{line}\n"))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(format!("sh {}", sh_quote(&path.to_string_lossy())))
}

/// Remove staged scripts older than a day. A tab that never opened leaves one behind, and
/// nothing else would ever collect it.
fn sweep_staged(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let day = std::time::Duration::from_secs(86_400);
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("adjutant-spawn-") || !name.ends_with(".sh") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map(|age| age > day).unwrap_or(false))
            .unwrap_or(false);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

pub fn spawn(
    terminal: &TerminalSettings,
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

    if let Some(template) = &terminal.spawn {
        let takes_argv = contains_placeholder(template, "cwd");
        let line = if takes_argv {
            req.command.to_string()
        } else {
            let mut line = format!("cd {} && ", sh_quote(req.cwd));
            if let Some(name_it) = req.title_command {
                line.push_str(&format!("( {name_it} || true ) && "));
            }
            line.push_str(req.command);
            line
        };
        let line = if !dry_run && line.len() > MAX_INLINE_COMMAND {
            stage_command(&line)?
        } else {
            line
        };
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
                screen: false,
            });
        }
        run_shell(&cmd)?;
        return Ok(Performed {
            description: format!("started in a new tab: {title} ({})", req.cwd),
            script: cmd,
            ran: true,
            screen: false,
        });
    }

    if terminal.is_tmux() {
        let line = if !dry_run && req.command.len() > MAX_INLINE_COMMAND {
            stage_command(req.command)?
        } else {
            req.command.to_string()
        };
        let script = tmux_spawn_script(
            terminal.tmux_socket(),
            terminal.tmux_session(),
            req.cwd,
            &title,
            &line,
        );
        if dry_run {
            return Ok(Performed {
                description: format!("will start in a new tab: {title} ({})", req.cwd),
                script,
                ran: false,
                screen: false,
            });
        }
        run_shell(&script)?;
        return Ok(Performed {
            description: format!("started in a new tab: {title} ({})", req.cwd),
            script,
            ran: true,
            screen: false,
        });
    }

    let mut line = format!("cd {} && ", sh_quote(req.cwd));
    if let Some(name_it) = req.title_command {
        line.push_str(&format!("( {name_it} || true ) && "));
    }
    line.push_str(req.command);
    let line = if !dry_run && line.len() > MAX_INLINE_COMMAND {
        stage_command(&line)?
    } else {
        line
    };
    let script = iterm_spawn_script(&line);
    if dry_run {
        return Ok(Performed {
            description: format!("will start in a new tab: {title} ({})", req.cwd),
            script,
            ran: false,
            screen: false,
        });
    }
    osascript(&script)?;
    Ok(Performed {
        description: format!("started in a new tab: {title} ({})", req.cwd),
        script,
        ran: true,
        screen: false,
    })
}

pub fn focus(
    terminal: &TerminalSettings,
    pid: u32,
    title: &str,
    dry_run: bool,
) -> Result<Performed, String> {
    let tty = tty_of(pid);
    if let Some(template) = &terminal.focus {
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
                false => format!("focused the tab (pid {pid})"),
            },
            script: cmd,
            ran: !dry_run,
            screen: false,
        });
    }

    if terminal.is_tmux() {
        let panes = list_tmux_panes(terminal.tmux_socket())?;
        let pane = find_matching_pane(&panes, Some(pid), tty.as_deref());
        let Some(pane) = pane else {
            return Ok(Performed {
                description: format!("no tmux pane found for pid {pid}; not focusing"),
                script: String::new(),
                ran: false,
                screen: false,
            });
        };
        let script =
            tmux_focus_script(terminal.tmux_socket(), &pane.window_id, Some(&pane.pane_id));
        if dry_run {
            return Ok(Performed {
                description: format!("will focus the tab (pid {pid})"),
                script,
                ran: false,
                screen: false,
            });
        }
        let result = run_shell(&script);
        let ran = result.is_ok();
        return Ok(Performed {
            description: match result {
                Ok(_) => format!("focused the tab (pid {pid})"),
                Err(e) => format!("failed to focus the tab (pid {pid}): {e}"),
            },
            script,
            ran,
            screen: false,
        });
    }

    let Some(tty) = tty else {
        return Ok(Performed {
            description: format!("no terminal found for pid {pid}; not focusing"),
            script: String::new(),
            ran: false,
            screen: false,
        });
    };
    let script = iterm_focus_script(&tty);
    if dry_run {
        return Ok(Performed {
            description: format!("will focus the tab (pid {pid}, {tty})"),
            script,
            ran: false,
            screen: false,
        });
    }
    // A window that cannot be raised is not worth failing a send over.
    let _ = osascript(&script);
    Ok(Performed {
        description: format!("focused the tab (pid {pid}, {tty})"),
        script,
        ran: true,
        screen: false,
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
pub fn close(
    terminal: &TerminalSettings,
    pid: u32,
    title: &str,
    dry_run: bool,
) -> Result<Performed, String> {
    close_with(run_shell, tty_of(pid), terminal, pid, title, dry_run)
}

/// The same, with the thing that runs the command handed in.
///
/// Split out for the reason `wake_with` is: the decision about whether a tab was actually
/// closed cannot be reached by a test, which has no iTerm2 session of its own to lose — and
/// that decision is the whole defect this shape exists to prevent.
fn close_with(
    run: impl Fn(&str) -> Result<String, String>,
    tty: Option<String>,
    terminal: &TerminalSettings,
    pid: u32,
    title: &str,
    dry_run: bool,
) -> Result<Performed, String> {
    let hook = &terminal.close;
    if hook.is_off() {
        // Off is an answer, not an absence: somebody said not to close tabs here, and the
        // built-in would otherwise close one. `ran` stays false, which is what stops the
        // caller from treating the worktree as finished with.
        return Ok(Performed {
            description: "closing tabs is turned off; the tab was left open".to_string(),
            script: String::new(),
            ran: false,
            screen: false,
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
        None => {
            if terminal.is_tmux() {
                built_in = true;
                let panes = list_tmux_panes_with(&run, terminal.tmux_socket())?;
                let pane = find_matching_pane(&panes, Some(pid), tty.as_deref());
                match pane {
                    Some(pane) => tmux_close_script(terminal.tmux_socket(), &pane.window_id),
                    None => {
                        return Ok(Performed {
                            description: format!("no tmux pane found for pid {pid}; not closing"),
                            script: String::new(),
                            ran: false,
                            screen: false,
                        });
                    }
                }
            } else {
                match tty.as_deref() {
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
                            screen: false,
                        });
                    }
                }
            }
        }
    };
    if dry_run {
        return Ok(Performed {
            description: format!("will close the tab (pid {pid})"),
            script: command,
            ran: false,
            screen: false,
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
            screen: false,
        });
    }
    Ok(Performed {
        description: format!("closed the tab (pid {pid})"),
        script: command,
        ran: true,
        screen: false,
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
pub fn set_title(
    terminal: &TerminalSettings,
    title: &str,
    dry_run: bool,
) -> Result<Performed, String> {
    let hook = &terminal.title;
    if hook.is_off() {
        return Ok(Performed {
            description: "tab naming is turned off".to_string(),
            script: String::new(),
            ran: false,
            screen: false,
        });
    }
    let title = sanitise_title(title, "adjutant".to_string());
    let command = match hook.template() {
        Some(template) => render(template, &[("title", Sub::Quoted(&title))]),
        None => {
            if terminal.is_tmux() {
                tmux_set_title_script(terminal.tmux_socket(), &title)
            } else {
                match own_tty() {
                    Some(tty) => default_title_command(&title, &tty),
                    None => {
                        return Ok(Performed {
                            description: "no terminal found for this process; not naming the tab"
                                .to_string(),
                            script: String::new(),
                            ran: false,
                            screen: false,
                        });
                    }
                }
            }
        }
    };
    if !dry_run {
        // A tab with the wrong name is a cosmetic problem; failing the caller over it is not.
        let _ = run_shell(&command);
    }
    Ok(Performed {
        description: format!("named the tab: {title}"),
        script: command,
        ran: !dry_run,
        screen: false,
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

/// How long to wait between typing the line and pressing Enter. The two writes have to
/// reach the agent as two reads; one that is busy and reads both at once sees a burst again.
const WAKE_ENTER_DELAY: &str = "0.2";

/// The line and its Enter go as two writes. `write text "<line>"` sends both in one burst,
/// and an agent's input box takes a long enough burst as a paste: the newline lands in the
/// box as text and nothing is submitted. Observed on Claude Code 2.1.282, where a line of
/// about 50 characters still submitted and one of 79 did not — and a paste threshold is the
/// agent's to move, so no line is short enough to count on.
fn default_wake_command(tty: &str, line: &str) -> String {
    let script = format!(
        "tell application \"iTerm2\"\n  \
           repeat with w in windows\n    \
             repeat with t in tabs of w\n      \
               repeat with s in sessions of t\n        \
                 if tty of s is \"/dev/{}\" then\n          \
                   tell s to write text \"{}\" newline NO\n          \
                   delay {WAKE_ENTER_DELAY}\n          \
                   tell s to write text \"\"\n          \
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
pub const HUB_WAKE_LINE: &str = "Something arrived in the inbox. Check it with adjutant_pending (or `adj pending` without it) and deal with it.";
pub const WORKER_WAKE_LINE: &str = "The hub sent you something. Check it with adjutant_outbox (or `adj outbox` without it) and deal with it.";

/// `default_line` is what to type when the config has not overridden it — the caller knows
/// which direction this is, and the two directions read different boxes.
///
/// `agent` is what the woken session runs. The built-in tmux wake reads the pane before typing
/// and needs to know whose screen it is looking at; `Generic` is typed into without looking.
pub struct WakeRequest<'a> {
    pub pid: u32,
    pub subject: &'a str,
    pub line: &'a str,
    pub agent: Agent,
    pub dry_run: bool,
}

pub fn wake(
    terminal: &TerminalSettings,
    wake: &Wake,
    pid: u32,
    subject: &str,
    default_line: &str,
    agent: Agent,
    dry_run: bool,
) -> Result<Performed, String> {
    let line = wake.line_or(default_line);
    wake_with(
        run_shell,
        tty_of(pid),
        terminal,
        wake,
        &WakeRequest {
            pid,
            subject,
            line,
            agent,
            dry_run,
        },
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
    terminal: &TerminalSettings,
    wake: &Wake,
    req: &WakeRequest,
) -> Result<Performed, String> {
    wake_with_clock(
        run,
        std::thread::sleep,
        &wake_lock_dir(),
        tty,
        terminal,
        wake,
        req,
    )
}

/// `wake_with`, with the thing that waits handed in as well: waiting for an agent to come
/// back to its prompt is a loop, and a test that slept through it would take the budget.
fn wake_with_clock(
    run: impl Fn(&str) -> Result<String, String>,
    wait: impl Fn(Duration),
    lock_dir: &Path,
    tty: Option<String>,
    terminal: &TerminalSettings,
    wake: &Wake,
    req: &WakeRequest,
) -> Result<Performed, String> {
    let hook = &wake.hook;
    let line = req.line;
    let pid = req.pid;
    let dry_run = req.dry_run;
    if hook.is_off() {
        return Ok(Performed {
            description: "waking is turned off".to_string(),
            script: String::new(),
            ran: false,
            screen: false,
        });
    }
    // A template is answered for by its exit status and nothing else — it is someone else's
    // command and only it knows what success means. The built-in knows more about itself
    // than that, and says so.
    let mut built_in = false;
    // The pane to read before typing, when the built-in tmux wake is the one in use.
    let mut look_at: Option<String> = None;
    let command = match hook.template() {
        Some(template) => render(
            template,
            &[
                ("pid", Sub::Quoted(&pid.to_string())),
                ("tty", Sub::Quoted(tty.as_deref().unwrap_or(""))),
                ("subject", Sub::Quoted(req.subject)),
                ("line", Sub::Quoted(line)),
            ],
        ),
        None => {
            if terminal.is_tmux() {
                built_in = true;
                let panes = list_tmux_panes_with(&run, terminal.tmux_socket())?;
                let pane = find_matching_pane(&panes, Some(pid), tty.as_deref());
                match pane {
                    Some(pane) => {
                        if req.agent != Agent::Generic {
                            look_at = Some(pane.pane_id.clone());
                        }
                        tmux_wake_script(terminal.tmux_socket(), &pane.pane_id, line)
                    }
                    None => {
                        return Ok(Performed {
                            description: format!(
                                "no tmux pane found for pid {pid}; nothing to wake"
                            ),
                            script: String::new(),
                            ran: false,
                            screen: false,
                        });
                    }
                }
            } else {
                match tty {
                    Some(tty) => {
                        built_in = true;
                        default_wake_command(&tty, line)
                    }
                    None => {
                        return Ok(Performed {
                            description: format!(
                                "no terminal found for pid {pid}; nothing to wake"
                            ),
                            script: String::new(),
                            ran: false,
                            screen: false,
                        });
                    }
                }
            }
        }
    };
    if dry_run {
        return Ok(Performed {
            description: format!("will wake the session (pid {pid})"),
            script: command,
            ran: false,
            screen: false,
        });
    }
    if let Some(pane_id) = look_at {
        return Ok(wake_after_looking(
            &run,
            &wait,
            terminal.tmux_socket(),
            &pane_id,
            lock_dir,
            req,
            command,
        ));
    }
    match run(&command) {
        // The message is already delivered by the time this runs. Failing to ring the bell
        // must not turn a successful send into an error.
        Err(e) => Ok(Performed {
            description: format!("cannot wake the session: {e}"),
            script: command,
            ran: false,
            screen: false,
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
            screen: false,
        }),
        Ok(_) => Ok(Performed {
            description: format!("woke the session (pid {pid})"),
            script: command,
            ran: true,
            screen: false,
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

// ── tmux ─────────────────────────────────────────────────────────────

pub fn tmux_cmd_prefix(socket: Option<&str>) -> String {
    match socket.filter(|s| !s.trim().is_empty()) {
        Some(s) if s.contains('/') => format!("tmux -S {}", sh_quote(s)),
        Some(s) => format!("tmux -L {}", sh_quote(s)),
        None => "tmux".to_string(),
    }
}

pub fn parse_tmux_panes(output: &str) -> Vec<TmuxPane> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim_end();
            if line.is_empty() {
                return None;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 7 {
                return None;
            }
            Some(TmuxPane {
                pane_id: parts[0].to_string(),
                pane_pid: parts[1].parse().ok()?,
                pane_tty: parts[2].to_string(),
                window_id: parts[3].to_string(),
                session_name: parts[4].to_string(),
                window_index: parts[5].parse().ok()?,
                window_name: parts[6].to_string(),
                window_activity: parts.get(7).and_then(|a| a.trim().parse().ok()),
            })
        })
        .collect()
}

pub fn find_matching_pane<'a>(
    panes: &'a [TmuxPane],
    pid: Option<u32>,
    tty: Option<&str>,
) -> Option<&'a TmuxPane> {
    if let Some(tty) = tty.filter(|t| !t.trim().is_empty()) {
        let norm_tty = tty.trim().trim_start_matches("/dev/");
        if let Some(pane) = panes
            .iter()
            .find(|p| p.pane_tty.trim_start_matches("/dev/") == norm_tty)
        {
            return Some(pane);
        }
    }
    if let Some(pid) = pid {
        if let Some(pane) = panes.iter().find(|p| p.pane_pid == pid) {
            return Some(pane);
        }
        if let Some(proc_tty) = tty_of(pid) {
            let norm_proc_tty = proc_tty.trim().trim_start_matches("/dev/");
            if let Some(pane) = panes
                .iter()
                .find(|p| p.pane_tty.trim_start_matches("/dev/") == norm_proc_tty)
            {
                return Some(pane);
            }
        }
        let mut curr = pid;
        for _ in 0..16 {
            if let Some(ppid) = parent_of(curr) {
                if let Some(pane) = panes.iter().find(|p| p.pane_pid == ppid) {
                    return Some(pane);
                }
                if ppid <= 1 {
                    break;
                }
                curr = ppid;
            } else {
                break;
            }
        }
    }
    None
}

pub fn list_tmux_panes_with(
    run: impl Fn(&str) -> Result<String, String>,
    socket: Option<&str>,
) -> Result<Vec<TmuxPane>, String> {
    let prefix = tmux_cmd_prefix(socket);
    let cmd = format!(
        "{prefix} list-panes -a -F '#{{pane_id}}\t#{{pane_pid}}\t#{{pane_tty}}\t#{{window_id}}\t#{{session_name}}\t#{{window_index}}\t#{{window_name}}\t#{{window_activity}}'"
    );
    match run(&cmd) {
        Ok(out) => Ok(parse_tmux_panes(&out)),
        Err(err) => {
            let lower = err.to_ascii_lowercase();
            if lower.contains("no server running")
                || lower.contains("error connecting to")
                || lower.contains("failed to connect")
                || lower.contains("no such file or directory")
            {
                Ok(Vec::new())
            } else {
                Err(err)
            }
        }
    }
}

pub fn list_tmux_panes(socket: Option<&str>) -> Result<Vec<TmuxPane>, String> {
    list_tmux_panes_with(run_shell, socket)
}

pub fn parse_tmux_clients(output: &str) -> Vec<TmuxClient> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.trim_end().split('\t').collect();
            if parts.len() < 3 {
                return None;
            }
            Some(TmuxClient {
                session: parts[0].to_string(),
                control: parts[1] == "1",
                window_id: parts[2].to_string(),
            })
        })
        .collect()
}

/// The clients attached to the server. A separate command from the pane listing, so a tmux
/// that does not know `list-clients` cannot take the panes down with it; every failure reads
/// as nobody attached.
pub fn list_tmux_clients_with(
    run: impl Fn(&str) -> Result<String, String>,
    socket: Option<&str>,
) -> Vec<TmuxClient> {
    let prefix = tmux_cmd_prefix(socket);
    let cmd = format!(
        "{prefix} list-clients -F '#{{client_session}}\t#{{client_control_mode}}\t#{{window_id}}'"
    );
    run(&cmd)
        .map(|out| parse_tmux_clients(&out))
        .unwrap_or_default()
}

pub fn list_tmux_clients(socket: Option<&str>) -> Vec<TmuxClient> {
    list_tmux_clients_with(run_shell, socket)
}

/// How many clients are attached to each window, by window id, for the windows in `panes`.
///
/// The board's own `adjboard-*` sessions are left out: they are a browser looking at the
/// window, not a person at it. The `adjterm-*` sessions made to open a session in a person's
/// terminal are counted, because a person is at those. `list-panes -a` lists a grouped window
/// once per session of the group, so the `adjboard-*` lines are ignored when working out which
/// windows a control-mode client sees.
pub fn attached_counts(
    panes: &[TmuxPane],
    clients: &[TmuxClient],
) -> std::collections::HashMap<String, u32> {
    let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut windows_of: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    for pane in panes {
        counts.entry(pane.window_id.clone()).or_insert(0);
        if pane.session_name.starts_with(BOARD_SESSION_PREFIX) {
            continue;
        }
        let windows = windows_of.entry(pane.session_name.as_str()).or_default();
        if !windows.contains(&pane.window_id.as_str()) {
            windows.push(pane.window_id.as_str());
        }
    }
    for client in clients {
        if client.session.starts_with(BOARD_SESSION_PREFIX) {
            continue;
        }
        if client.control {
            for window in windows_of
                .get(client.session.as_str())
                .into_iter()
                .flatten()
            {
                *counts.entry((*window).to_string()).or_insert(0) += 1;
            }
        } else {
            *counts.entry(client.window_id.clone()).or_insert(0) += 1;
        }
    }
    counts
}

pub fn find_tmux_pane(
    socket: Option<&str>,
    pid: Option<u32>,
    tty: Option<&str>,
) -> Result<Option<TmuxPane>, String> {
    let panes = list_tmux_panes(socket)?;
    Ok(find_matching_pane(&panes, pid, tty).cloned())
}

pub fn tmux_spawn_script(
    socket: Option<&str>,
    session: &str,
    cwd: &str,
    title: &str,
    command: &str,
) -> String {
    let prefix = tmux_cmd_prefix(socket);
    let session_q = sh_quote(session);
    // Targets name the session exactly (`=`), and the window target ends in `:` so the new
    // window takes the next free index. A bare `-t adjutant` is looked up as a window first and
    // matches window names by prefix, so a hub window called `adjutant-…` would be taken as the
    // target and the new window refused with "index N in use".
    let exact_q = sh_quote(&format!("={session}"));
    let next_q = sh_quote(&format!("={session}:"));
    let cwd_q = sh_quote(cwd);
    let title_q = sh_quote(title);
    let cmd_q = sh_quote(command);
    format!(
        "{prefix} has-session -t {exact_q} 2>/dev/null || {prefix} new-session -d -s {session_q} -n main; {prefix} new-window -d -t {next_q} -c {cwd_q} -n {title_q} {cmd_q}"
    )
}

pub fn tmux_wake_script(socket: Option<&str>, pane_id: &str, line: &str) -> String {
    let prefix = tmux_cmd_prefix(socket);
    let pane_q = sh_quote(pane_id);
    let line_q = sh_quote(line);
    format!(
        "{prefix} send-keys -l -t {pane_q} {line_q} && sleep {WAKE_ENTER_DELAY} && {prefix} send-keys -t {pane_q} Enter && echo {WOKE_MARKER}"
    )
}

// ── looking at a pane before typing into it ──────────────────────────
//
// A wake types a line and Enter into whatever the agent is showing. Enter answers a question
// the agent is asking, and a line typed while a person is halfway through a message is
// appended to theirs and sent. So the built-in tmux wake reads the pane first and types only
// at an empty prompt.
//
// What "an empty prompt" looks like is each agent's business, and the markers below were read
// off captures of the real thing (`src/fixtures/panes`). They are structural — a numbered
// list with a pointer, a key-hint line, a bordered input box — rather than the wording of
// one release, and a screen that matches none of them is not typed into.

/// The line `tmux_capture_script` prints between the screen and the pane's own state.
const PANE_META_SEPARATOR: &str = "@@adjutant:pane@@";

/// How long to wait for the agent to come back to its prompt, looking every so often. Long
/// enough for a turn that is just ending; a person who is answering a question is not waited
/// for, and falls back to being notified.
const WAKE_READY_BUDGET: Duration = Duration::from_secs(5);
const WAKE_READY_POLL: Duration = Duration::from_millis(500);

/// After typing: how many times to look for the line at the prompt, and how far apart. The
/// agent draws what it is sent asynchronously, so the first look can be too early.
const WAKE_ECHO_LOOKS: u32 = 5;
const WAKE_ECHO_POLL: Duration = Duration::from_millis(250);

/// A box at the bottom of the screen with more than this under it is not an input box but
/// something drawn over one.
const MAX_LINES_BELOW_INPUT: usize = 5;

/// What an agent's screen says it is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneState {
    /// Sitting at an empty input prompt: the one state a wake is typed into.
    Idle,
    /// A question, a permission prompt or a menu is up, and Enter would answer it.
    Asking,
    /// There is text in the input box: a person is midway through writing.
    Typing,
    /// In the middle of a turn. Typed text would be taken as a message to the running turn
    /// rather than the next one, which the wake was not written for.
    Working,
    /// The pane is scrolled back or searching (tmux copy mode); keys go to tmux, not the agent.
    CopyMode,
    /// None of the above could be told from the screen.
    Unknown,
}

impl PaneState {
    /// Why nothing was typed, for whoever is told the wake did not happen.
    pub fn why_not_typed(self) -> &'static str {
        match self {
            PaneState::Idle => "its prompt is empty",
            PaneState::Asking => "its screen shows a question or a menu",
            PaneState::Typing => "there is text typed at its prompt",
            PaneState::Working => "it is in the middle of a turn",
            PaneState::CopyMode => "its pane is in tmux copy mode",
            PaneState::Unknown => "its screen was not recognised",
        }
    }
}

/// A capture of a pane: what is on it, and what tmux says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneScreen {
    /// One line per row, with the colour and attribute escapes `capture-pane -e` leaves in.
    pub text: String,
    /// Whether the pane is in a tmux mode (copy mode and the like).
    pub in_mode: bool,
    pub cursor_x: u32,
    pub cursor_y: u32,
}

/// The command that prints a pane's screen, then `PANE_META_SEPARATOR`, then
/// `pane_in_mode`, `cursor_x` and `cursor_y` separated by tabs.
///
/// `-u` for the reason `tmux_window_home_script` gives: without a UTF-8 locale tmux rewrites
/// every non-ASCII character it prints to `_`, and the prompt glyph is one. `-e` keeps the
/// attributes, because the only thing telling an agent's placeholder text from typed text is
/// that the placeholder is drawn faint. `-J` joins rows the terminal wrapped.
pub fn tmux_capture_script(socket: Option<&str>, pane_id: &str) -> String {
    let prefix = tmux_cmd_prefix(socket).replacen("tmux", "tmux -u", 1);
    let pane_q = sh_quote(pane_id);
    format!(
        "{prefix} capture-pane -p -e -J -t {pane_q} && echo {PANE_META_SEPARATOR} && {prefix} display-message -p -t {pane_q} '#{{pane_in_mode}}\t#{{cursor_x}}\t#{{cursor_y}}'"
    )
}

pub fn parse_pane_screen(output: &str) -> PaneScreen {
    let (text, meta) = match output.rsplit_once(PANE_META_SEPARATOR) {
        Some((text, meta)) => (text, meta.trim()),
        None => (output, ""),
    };
    let mut fields = meta.split('\t');
    let mut next = || fields.next().and_then(|f| f.trim().parse::<u32>().ok());
    PaneScreen {
        text: text.trim_end().to_string(),
        in_mode: next().is_some_and(|n| n != 0),
        cursor_x: next().unwrap_or(0),
        cursor_y: next().unwrap_or(0),
    }
}

/// The character an agent draws in front of its input line. Old messages in the transcript
/// start with it too; the box around the live one is what tells them apart.
fn prompt_glyph(agent: Agent) -> char {
    match agent {
        Agent::Agy => '>',
        _ => '❯',
    }
}

/// Lines that mean a choice is on screen and Enter would make it, whichever agent draws it.
const ASKING_HINTS: &[&str] = &["(y/n)", "[y/n]", "(yes/no)", "[yes/no]"];
const CLAUDE_ASKING_HINTS: &[&str] = &[
    "esc to cancel",
    "enter to select",
    "enter to confirm",
    "tab to amend",
];
// agy's footer says `esc to cancel` while it works as well as while it asks, so that is not a
// marker for it; the navigation hint is only ever drawn with a list.
const AGY_ASKING_HINTS: &[&str] = &["↑/↓ navigate"];

/// A line the spinner is drawn on. Claude counts the seconds it has been at it in brackets
/// after an ellipsis (`✻ … (3s · thinking)`, and `1m 5s` once it passes a minute), with a verb the user can change, so it is the
/// shape that is looked for; the line left behind when a turn ends (`✻ Baked for 4s · done`)
/// has no ellipsis and no brackets.
fn claude_spinner(line: &str) -> bool {
    let line = line.trim_start();
    let Some(first) = line.chars().next() else {
        return false;
    };
    if !"✻✽✶✳✢·*".contains(first) {
        return false;
    }
    // Before the first tick there is no timer yet: `✻ Pondering…`. Not with the dot, which
    // also leads ordinary list lines.
    if first != '·' && line.trim_end().ends_with('…') {
        return true;
    }
    let Some((_, rest)) = line.split_once("… (") else {
        return false;
    };
    // The timer is `3s`, then `1m 5s`, then `1h 2m`: a turn of a minute or more still counts.
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].starts_with(['h', 'm', 's'])
}

/// agy draws a braille spinner in front of what it is doing.
fn agy_spinner(line: &str) -> bool {
    line.trim_start()
        .chars()
        .next()
        .is_some_and(|c| ('\u{2800}'..='\u{28ff}').contains(&c))
}

fn working_marker(agent: Agent, line: &str) -> bool {
    let lower = line.to_lowercase();
    match agent {
        Agent::Claude => claude_spinner(line) || lower.contains("esc to interrupt"),
        Agent::Agy => agy_spinner(line) || lower.trim_start().starts_with("esc to cancel"),
        Agent::Generic => false,
    }
}

/// `line` without the escape sequences `capture-pane -e` puts in it.
fn strip_escapes(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            // A control sequence ends at its first byte in @..~.
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

/// Whether everything drawn after the prompt glyph on this line is drawn faint, which is how
/// an agent shows the suggestion it fills an empty box with. Text someone typed is not.
fn faint_after_glyph(raw: &str, glyph: char) -> bool {
    let mut faint = false;
    let mut past_glyph = false;
    let mut any = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() != Some(&'[') {
                continue;
            }
            chars.next();
            let mut params = String::new();
            let mut last = ' ';
            for c in chars.by_ref() {
                last = c;
                if ('@'..='~').contains(&c) {
                    break;
                }
                params.push(c);
            }
            if last == 'm' {
                let mut parts = params.split(';');
                while let Some(p) = parts.next() {
                    match p {
                        "" | "0" | "22" => faint = false,
                        "2" => faint = true,
                        // A colour's own arguments are not attributes: `38;5;2` is not faint.
                        "38" | "48" => {
                            let skip = if parts.next() == Some("2") { 3 } else { 1 };
                            for _ in 0..skip {
                                parts.next();
                            }
                        }
                        _ => {}
                    }
                }
            }
            continue;
        }
        if !past_glyph {
            past_glyph = c == glyph;
        } else if !c.is_whitespace() {
            any = true;
            if !faint {
                return false;
            }
        }
    }
    any
}

/// A line that is a horizontal rule: the edge of the input box.
fn is_border(plain: &str) -> bool {
    let line = plain.trim();
    line.starts_with("───") && line.chars().filter(|&c| c == '─').count() >= 10
}

/// A line that is one of a numbered list's items with the pointer on it: `❯ 1. Yes`.
fn is_pointed_option(plain: &str) -> bool {
    let line = plain.trim_start();
    let Some(rest) = line.strip_prefix(['❯', '>', '›']) else {
        return false;
    };
    let rest = rest.trim_start();
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].starts_with(['.', ')'])
}

struct Line {
    raw: String,
    plain: String,
}

struct InputBox {
    /// What is typed in it, apart from a suggestion the agent drew there.
    text: String,
    /// The row of its top border.
    top: usize,
    /// The row of its bottom border.
    bottom: usize,
}

/// The input box nearest the bottom: a line starting with the prompt glyph that has a border
/// above it and, further down, another one.
fn input_box(agent: Agent, lines: &[Line]) -> Option<InputBox> {
    let glyph = prompt_glyph(agent);
    for top in (0..lines.len().saturating_sub(1)).rev() {
        if !is_border(&lines[top].plain) || !lines[top + 1].plain.trim_start().starts_with(glyph) {
            continue;
        }
        let Some(bottom) = (top + 2..lines.len()).find(|&i| is_border(&lines[i].plain)) else {
            continue;
        };
        let below = lines[bottom + 1..]
            .iter()
            .filter(|l| !l.plain.trim().is_empty())
            .count();
        if below > MAX_LINES_BELOW_INPUT {
            continue;
        }
        let first = &lines[top + 1];
        let mut text = first
            .plain
            .trim_start()
            .trim_start_matches(glyph)
            .trim()
            .to_string();
        let suggestion = agent == Agent::Claude && faint_after_glyph(&first.raw, glyph);
        if suggestion {
            text.clear();
        }
        for line in &lines[top + 2..bottom] {
            text.push(' ');
            text.push_str(line.plain.trim());
        }
        return Some(InputBox {
            text: text.trim().to_string(),
            top,
            bottom,
        });
    }
    None
}

/// What `pane_state` decided, and what it found typed in the input box on the way.
struct Reading {
    state: PaneState,
    typed: String,
}

fn read_pane(agent: Agent, screen: &PaneScreen) -> Reading {
    let reading = |state| Reading {
        state,
        typed: String::new(),
    };
    if agent == Agent::Generic {
        return reading(PaneState::Idle);
    }
    if screen.in_mode {
        return reading(PaneState::CopyMode);
    }
    let lines: Vec<Line> = screen
        .text
        .lines()
        .map(|raw| Line {
            plain: strip_escapes(raw),
            raw: raw.to_string(),
        })
        .collect();
    let hints = match agent {
        Agent::Agy => AGY_ASKING_HINTS,
        _ => CLAUDE_ASKING_HINTS,
    };
    let input = input_box(agent, &lines);
    // A pointed option is looked for only where a question would be drawn. Above a live input
    // box is the transcript, where an echo of what the person once typed (`❯ 1. do this`)
    // looks the same; a real menu or question replaces the box, so with none on screen every
    // line counts. The box's own line is above its bottom border and is not looked at, so
    // someone typing `1. foo` is still typing.
    let pointed_from = input.as_ref().map_or(0, |b| b.bottom + 1);
    let asking = lines.iter().enumerate().any(|(row, l)| {
        let lower = l.plain.to_lowercase();
        (row >= pointed_from && is_pointed_option(&l.plain))
            || ASKING_HINTS.iter().any(|h| lower.contains(h))
            || hints.iter().any(|h| lower.contains(h))
    });
    if asking {
        return reading(PaneState::Asking);
    }
    let Some(input) = input else {
        return reading(PaneState::Unknown);
    };
    if !input.text.is_empty() {
        return Reading {
            state: PaneState::Typing,
            typed: input.text,
        };
    }
    if lines.iter().any(|l| working_marker(agent, &l.plain)) {
        return reading(PaneState::Working);
    }
    reading(PaneState::Idle)
}

/// The longest line `last_output_line` returns, in characters.
const LAST_LINE_CHARS: usize = 200;

/// A line indented at least this far is the agent's right-aligned chrome (the effort level, a
/// key hint), not something it wrote. This can drop a line of output that really is indented
/// that deep; that is traded for not showing Claude's hints as the last thing it said.
const RIGHT_ALIGNED_INDENT: usize = 24;

/// The footer a menu or a question draws to say which keys answer it.
fn is_key_hint(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    let hint = |s: &str| s.starts_with("enter to select") || s.starts_with("esc to cancel");
    // Either the hint itself, or one of several joined by the dot a footer separates them with;
    // a sentence that only mentions the keys is output.
    hint(&lower) || (lower.contains('·') && lower.split('·').any(|part| hint(part.trim())))
}

/// Lines an agent draws between its output and its input box that are not output: hints about
/// itself and about the terminal it found itself in.
fn is_chrome(plain: &str) -> bool {
    let line = plain.trim();
    let indent = plain.chars().take_while(|c| c.is_whitespace()).count();
    indent >= RIGHT_ALIGNED_INDENT
        || line
            .strip_prefix('⎿')
            .is_some_and(|rest| rest.trim_start().starts_with("Tip:"))
        || line.starts_with("tmux detected")
        || is_key_hint(line)
}

/// The last thing the agent wrote, as one line of what its screen shows: the last line above
/// the input box that has anything in it, without the escapes and the agent's own chrome, cut
/// to `LAST_LINE_CHARS`. A screen with no input box (a generic agent's, or one showing a menu)
/// gives its last line. `None` for a blank screen.
pub fn last_output_line(agent: Agent, screen: &PaneScreen) -> Option<String> {
    let lines: Vec<Line> = screen
        .text
        .lines()
        .map(|raw| Line {
            plain: strip_escapes(raw),
            raw: raw.to_string(),
        })
        .collect();
    let end = match agent {
        Agent::Generic => lines.len(),
        _ => input_box(agent, &lines).map_or(lines.len(), |b| b.top),
    };
    let line = lines[..end]
        .iter()
        .rev()
        .map(|l| l.plain.as_str())
        .find(|l| !l.trim().is_empty() && !is_border(l) && !is_chrome(l))?
        .trim();
    let mut chars = line.chars();
    let cut: String = chars.by_ref().take(LAST_LINE_CHARS).collect();
    Some(if chars.next().is_some() {
        format!("{}…", cut.trim_end())
    } else {
        cut
    })
}

/// A tmux pane's screen, or `None` when tmux cannot show it (the pane is gone, no server).
pub fn look_at_tmux_pane(socket: Option<&str>, pane_id: &str) -> Option<PaneScreen> {
    look_at_pane(&run_shell, &tmux_capture_script(socket, pane_id)).ok()
}

/// What the agent's screen says it is doing. `Generic` is `Idle` without looking: nothing is
/// known of how its screen reads, and it is how the wake behaved before it looked at all.
pub fn pane_state(agent: Agent, screen: &PaneScreen) -> PaneState {
    read_pane(agent, screen).state
}

/// Does the input box hold the line and nothing else? Whitespace is ignored because a line
/// the box is too narrow for is wrapped at a space that was not in it. Anything more in the
/// box is someone else's typing that the line has been appended to, and Enter would send both.
fn shows_typed_line(agent: Agent, screen: &PaneScreen, line: &str) -> bool {
    let squeeze = |s: &str| s.split_whitespace().collect::<String>();
    let reading = read_pane(agent, screen);
    reading.state == PaneState::Typing && squeeze(&reading.typed) == squeeze(line)
}

/// The line, typed and not sent. The pause is the same one `tmux_wake_script` leaves before
/// Enter; here it is also what gives the agent time to draw the line before it is looked for.
fn tmux_type_script(socket: Option<&str>, pane_id: &str, line: &str) -> String {
    let prefix = tmux_cmd_prefix(socket);
    format!(
        "{prefix} send-keys -l -t {} {} && sleep {WAKE_ENTER_DELAY}",
        sh_quote(pane_id),
        sh_quote(line)
    )
}

fn tmux_enter_script(socket: Option<&str>, pane_id: &str) -> String {
    let prefix = tmux_cmd_prefix(socket);
    format!(
        "{prefix} send-keys -t {} Enter && echo {WOKE_MARKER}",
        sh_quote(pane_id)
    )
}

fn look_at_pane(
    run: &impl Fn(&str) -> Result<String, String>,
    capture: &str,
) -> Result<PaneScreen, String> {
    run(capture).map(|out| parse_pane_screen(&out))
}

/// Where the per-pane locks live. Not the state directory: that is a matter for a layer above
/// this one, and all a lock needs is a place every wake on this machine agrees on.
fn wake_lock_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("adjutant-wake-locks")
}

/// Take the lock that lets one wake at a time look at a pane and type into it.
///
/// Two workers reporting to one hub at the same moment would both see an empty prompt and
/// both type; the box would then hold two lines, neither would be sent, and the stale text
/// would block every wake after. An advisory lock on an open file, like the dispatch lock:
/// the system lets go of it when its holder dies. The wait comes out of the same budget as
/// waiting for the prompt. `Err` is running out of it; a lock that cannot be made at all is
/// not a reason to stop waking, so that is `Ok(None)`.
fn lock_pane(
    dir: &Path,
    socket: Option<&str>,
    pane_id: &str,
    wait: &impl Fn(Duration),
    waited: &mut Duration,
) -> Result<Option<std::fs::File>, ()> {
    let key: String = format!("{}-{pane_id}", socket.unwrap_or("default"))
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if std::fs::create_dir_all(dir).is_err() {
        return Ok(None);
    }
    // Never removed, for the reason the dispatch lock gives.
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!("{key}.lock")))
    else {
        return Ok(None);
    };
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => {
                if *waited >= WAKE_READY_BUDGET {
                    return Err(());
                }
                wait(WAKE_READY_POLL);
                *waited += WAKE_READY_POLL;
            }
            Err(std::fs::TryLockError::Error(_)) => return Ok(None),
        }
    }
}

/// The built-in tmux wake for an agent whose screen is known: wait for its prompt to be
/// empty, type the line, check that it landed at the prompt, and only then press Enter.
///
/// The second look is what keeps a screen that changed between the first look and the typing
/// from being answered: if the line is not where it should be, Enter is not pressed and the
/// caller is told, which is the same as a wake that could not be done. `script` is what the
/// wake amounts to, for the caller to report.
fn wake_after_looking(
    run: &impl Fn(&str) -> Result<String, String>,
    wait: &impl Fn(Duration),
    socket: Option<&str>,
    pane_id: &str,
    lock_dir: &Path,
    req: &WakeRequest,
    script: String,
) -> Performed {
    let (pid, agent, line) = (req.pid, req.agent, req.line);
    let capture = tmux_capture_script(socket, pane_id);
    let refused = |description: String, script: &str, screen: bool| Performed {
        description,
        script: script.to_string(),
        ran: false,
        screen,
    };
    let mut waited = Duration::ZERO;
    // Held until this returns: from the first look to Enter, or to giving up.
    let _turn = match lock_pane(lock_dir, socket, pane_id, wait, &mut waited) {
        Ok(held) => held,
        Err(()) => {
            return refused(
                format!(
                    "the wake was not typed into the session (pid {pid}): another wake was still typing into its pane"
                ),
                &capture,
                true,
            );
        }
    };
    loop {
        let screen = match look_at_pane(run, &capture) {
            Ok(screen) => screen,
            Err(e) => {
                return refused(
                    format!(
                        "the wake was not typed into the session (pid {pid}): its screen could not be read ({e})"
                    ),
                    &capture,
                    true,
                );
            }
        };
        let state = pane_state(agent, &screen);
        if state == PaneState::Idle {
            break;
        }
        if waited >= WAKE_READY_BUDGET {
            return refused(
                format!(
                    "the wake was not typed into the session (pid {pid}): {}",
                    state.why_not_typed()
                ),
                &capture,
                true,
            );
        }
        wait(WAKE_READY_POLL);
        waited += WAKE_READY_POLL;
    }
    let typed = tmux_type_script(socket, pane_id, line);
    if let Err(e) = run(&typed) {
        return refused(format!("cannot wake the session: {e}"), &typed, false);
    }
    let mut shown = false;
    for look in 0..WAKE_ECHO_LOOKS {
        if look > 0 {
            wait(WAKE_ECHO_POLL);
        }
        if let Ok(screen) = look_at_pane(run, &capture)
            && shows_typed_line(agent, &screen, line)
        {
            shown = true;
            break;
        }
    }
    if !shown {
        return refused(
            format!(
                "the wake was not sent to the session (pid {pid}): its prompt did not hold just the line typed, so Enter was not pressed; the line may have been left at the prompt"
            ),
            &script,
            true,
        );
    }
    match run(&tmux_enter_script(socket, pane_id)) {
        Err(e) => refused(format!("cannot wake the session: {e}"), &script, false),
        Ok(out) if !woke(true, &out) => refused(
            format!("no tab of this terminal is running pid {pid}; nothing woken"),
            &script,
            false,
        ),
        Ok(_) => Performed {
            description: format!("woke the session (pid {pid})"),
            script,
            ran: true,
            screen: false,
        },
    }
}

pub fn tmux_close_script(socket: Option<&str>, window_id: &str) -> String {
    let prefix = tmux_cmd_prefix(socket);
    let win_q = sh_quote(window_id);
    format!("{prefix} kill-window -t {win_q} && echo {CLOSED_MARKER}")
}

/// Close one pane. Where a hub is the only thing in its window this closes the window too,
/// and where it is not, only the hub goes.
pub fn tmux_kill_pane_script(socket: Option<&str>, pane_id: &str) -> String {
    let prefix = tmux_cmd_prefix(socket);
    let pane_q = sh_quote(pane_id);
    format!("{prefix} kill-pane -t {pane_q} && echo {CLOSED_MARKER}")
}

pub fn tmux_focus_script(socket: Option<&str>, window_id: &str, pane_id: Option<&str>) -> String {
    let prefix = tmux_cmd_prefix(socket);
    let win_q = sh_quote(window_id);
    let mut script = format!("{prefix} select-window -t {win_q}");
    if let Some(pane) = pane_id {
        script.push_str(&format!(" && {prefix} select-pane -t {}", sh_quote(pane)));
    }
    #[cfg(target_os = "macos")]
    {
        script.push_str(
            " && ( ( [ -n \"$ITERM_SESSION_ID\" ] || [ \"$TERM_PROGRAM\" = \"iTerm.app\" ] ) && osascript -e 'tell application \"iTerm2\" to activate' 2>/dev/null || true )",
        );
    }
    script
}

pub fn tmux_set_title_script(socket: Option<&str>, title: &str) -> String {
    let prefix = tmux_cmd_prefix(socket);
    let title_q = sh_quote(title);
    format!("{prefix} rename-window {title_q}")
}

// ── tmux, for the board terminal ─────────────────────────────────────
//
// The board shows a session by attaching a client of its own to it. A plain `attach-session`
// would share the session with whoever is already attached — their current window would
// follow the browser's — so each board connection gets a session of its own *in the same
// group*: it shares every window with the original and has a current window of its own.

/// What every session made for a board connection is called, followed by the pid of the
/// process that made it and a counter. A leftover one is recognised by it.
pub const BOARD_SESSION_PREFIX: &str = "adjboard-";

/// What every session made to open a session in the person's own terminal is called, followed
/// as above by a pid and a counter. Not `adjboard-`: those are left out of the attached counts
/// because they are a browser looking at a window, and this one is a person at it.
pub const OPEN_SESSION_PREFIX: &str = "adjterm-";

/// Whether a session name is one this tool made for looking at a window.
fn is_own_session(name: &str) -> bool {
    name.starts_with(BOARD_SESSION_PREFIX) || name.starts_with(OPEN_SESSION_PREFIX)
}

/// `-S <path>` for a socket that is a path, `-L <name>` for one that is a name, and nothing
/// for the default server: the arguments `tmux_cmd_prefix` spells as a shell prefix.
pub fn tmux_socket_args(socket: Option<&str>) -> Vec<String> {
    match socket.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) if s.contains('/') => vec!["-S".to_string(), s.to_string()],
        Some(s) => vec!["-L".to_string(), s.to_string()],
        None => Vec::new(),
    }
}

/// Where the tmux server `socket` names listens, spelled the way tmux itself would.
///
/// `socket` is read as `tmux_socket_args` reads it: a path as it is, a name as a file of that
/// name in tmux's socket directory, and nothing as the default server — which is the one
/// `$TMUX` names when the caller is itself inside tmux, and `default` otherwise. So the
/// different ways of saying "the same server" come out as one path.
///
/// The directory is `tmux-<uid>` under `$TMUX_TMPDIR`, or `/tmp` without it.
pub fn tmux_socket_path(
    socket: Option<&str>,
    tmux_env: Option<&str>,
    tmpdir: Option<&str>,
    uid: u32,
) -> std::path::PathBuf {
    let socket = socket.map(str::trim).filter(|s| !s.is_empty());
    if let Some(path) = socket.filter(|s| s.contains('/')) {
        return std::path::PathBuf::from(path);
    }
    let named = |name: &str| {
        let base = tmpdir.filter(|d| !d.is_empty()).unwrap_or("/tmp");
        std::path::Path::new(base)
            .join(format!("tmux-{uid}"))
            .join(name)
    };
    match socket {
        Some(name) => named(name),
        None => match tmux_env
            .and_then(|env| env.split(',').next())
            .filter(|path| !path.is_empty())
        {
            Some(path) => std::path::PathBuf::from(path),
            None => named("default"),
        },
    }
}

/// `(major, minor)` out of what `tmux -V` prints: `tmux 3.7c`, `tmux 3.1`, `tmux next-3.5`.
/// A build from the development branch (`tmux master`) is newer than any release.
pub fn parse_tmux_version(output: &str) -> Option<(u32, u32)> {
    let version = output.trim().strip_prefix("tmux")?.trim();
    if version == "master" {
        return Some((u32::MAX, 0));
    }
    let version = version.strip_prefix("next-").unwrap_or(version);
    let (major, rest) = version.split_once('.')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((major.parse().ok()?, digits.parse().ok()?))
}

/// The installed tmux's version, or `None` when there is no tmux to ask.
pub fn tmux_version() -> Option<(u32, u32)> {
    parse_tmux_version(&run_shell("tmux -V").ok()?)
}

/// Where a window lives now: its session and that session's group, asked of tmux by the
/// window id the record holds. It also says whether the window is still there at all, before
/// anything is created on its behalf.
///
/// `-u` because tmux answers in the locale it is run in: without one that is UTF-8 (a resident
/// started by launchd or systemd usually has none) it rewrites the tab between the fields and
/// every non-ASCII character of a session name to `_`, and the group named from that answer
/// would not exist. With `-u` the answer is kept as it is.
pub fn tmux_window_home_script(socket: Option<&str>, window_id: &str) -> String {
    format!(
        "{} display-message -p -t {} '#{{session_name}}\t#{{session_group}}'",
        tmux_cmd_prefix(socket).replacen("tmux", "tmux -u", 1),
        sh_quote(window_id)
    )
}

/// The group to join for `tmux_window_home_script`'s answer: the session's group when it has
/// one, and the session itself otherwise (which makes it the group's first member).
pub fn parse_window_home(output: &str) -> Option<String> {
    let (session, group) = output
        .trim_end()
        .split_once('\t')
        .unwrap_or((output.trim(), ""));
    let target = if group.is_empty() { session } else { group };
    (!target.is_empty()).then(|| target.to_string())
}

/// Make the session `name` in `group`, showing `window`.
pub fn board_attach_prepare_script(
    socket: Option<&str>,
    group: &str,
    name: &str,
    window: &str,
) -> String {
    let prefix = tmux_cmd_prefix(socket);
    format!(
        "{prefix} new-session -d -s {} -t {} && {prefix} select-window -t {}",
        sh_quote(name),
        sh_quote(group),
        sh_quote(&format!("={name}:{window}"))
    )
}

/// The window ids the session `name` has, one per line: what the board terminal checks its
/// target against. There is no hook for this on purpose: a `window-unlinked` hook on a session
/// that is later destroyed crashed the tmux server (3.4 on Linux, under load), which takes every
/// agent's window with it.
pub fn board_windows_script(socket: Option<&str>, name: &str) -> String {
    format!(
        "{} list-windows -t {} -F '#{{window_id}}'",
        tmux_cmd_prefix(socket),
        sh_quote(&format!("={name}"))
    )
}

/// Detach whatever is attached to the session `name`.
pub fn board_detach_script(socket: Option<&str>, name: &str) -> String {
    format!(
        "{} detach-client -s {}",
        tmux_cmd_prefix(socket),
        sh_quote(&format!("={name}"))
    )
}

/// The command line of the client a board connection runs on its terminal. `-E` leaves the
/// session's environment alone; `active-pane` (tmux 3.3 and later) keeps the client from
/// resizing panes it is not looking at.
///
/// The session option `destroy-unattached` is deliberately not set: tmux removing the session
/// itself crashes the server (seen on 3.4, 3.5 and 3.7 on Linux), so removal is left to
/// `board_release_script` and `board_sweep_script`.
pub fn board_attach_args(socket: Option<&str>, name: &str, active_pane: bool) -> Vec<String> {
    let mut args = vec!["-u".to_string()];
    args.extend(tmux_socket_args(socket));
    args.push("attach-session".to_string());
    args.push("-E".to_string());
    if active_pane {
        args.extend(["-f".to_string(), "active-pane".to_string()]);
    }
    args.extend(["-t".to_string(), format!("={name}")]);
    args
}

/// Remove the sessions made for the board terminal and for opening a session in a terminal
/// that nobody is attached to, left behind by a process that did not get to clean up. Only
/// those that are in a group with something else (they hold no windows of their own) and that
/// are not new: one made a moment ago by another connection is not attached *yet*.
pub fn board_sweep_script(socket: Option<&str>) -> String {
    let prefix = tmux_cmd_prefix(socket);
    format!(
        "now=$(date +%s); {prefix} list-sessions -F '#{{session_attached}} #{{session_group_size}} #{{session_created}} #{{session_name}}' 2>/dev/null | while read attached size created name; do case \"$name\" in {BOARD_SESSION_PREFIX}*|{OPEN_SESSION_PREFIX}*) if [ \"$attached\" = 0 ] && [ \"$size\" -gt 1 ] && [ $((now - created)) -gt 30 ]; then {prefix} kill-session -t \"=$name\"; fi;; esac; done; true"
    )
}

/// The command a terminal window runs to show the session `name`: `tmux attach` on it, in
/// control mode (`-CC`) for iTerm2, which draws tmux's windows as its own.
///
/// With `keep_last` (tmux 3.4 and later) the session goes when its last client leaves, so
/// closing the window leaves nothing behind. Earlier tmux has no such option, and the session
/// is left for the next sweep; `destroy-unattached on` is not a substitute, as it would end the
/// session before anything attached.
pub fn native_attach_line(
    socket: Option<&str>,
    name: &str,
    control: bool,
    keep_last: bool,
) -> String {
    let mut args = vec!["tmux".to_string(), "-u".to_string()];
    if control {
        args.push("-CC".to_string());
    }
    args.extend(tmux_socket_args(socket));
    args.extend(["attach-session", "-t"].map(str::to_string));
    args.push(format!("={name}"));
    if keep_last {
        args.extend([";", "set-option", "-t"].map(str::to_string));
        args.push(format!("={name}:"));
        args.extend(["destroy-unattached", "keep-last"].map(str::to_string));
    }
    crate::template::sh_join(&args)
}

/// A new iTerm2 window running `line`. A window of its own rather than a tab of the current
/// one: the person may be in another application, or have no window open at all.
fn iterm_attach_script(line: &str) -> String {
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

/// What `board_release_script` prints when it left the session alone.
pub const KEPT_MARKER: &str = "adjutant:kept";

/// Remove the session `name` once its client has gone, unless it is the only holder of the
/// windows: if the original session was killed meanwhile, the group has shrunk to this one and
/// killing it would take every window with it — that it was left is what `KEPT_MARKER` says.
pub fn board_release_script(socket: Option<&str>, name: &str) -> String {
    let prefix = tmux_cmd_prefix(socket);
    // `=name` alone is not a pane target, so the ones that want one end in a colon, which makes
    // it the session's current pane; `kill-session` takes a session and no colon.
    let target = sh_quote(&format!("={name}:"));
    let session = sh_quote(&format!("={name}"));
    format!(
        "size=$({prefix} display-message -p -t {target} '#{{session_group_size}}' 2>/dev/null); if [ \"${{size:-0}}\" -gt 1 ]; then {prefix} kill-session -t {session}; elif [ \"${{size:-0}}\" = 1 ]; then echo {KEPT_MARKER}; fi; true"
    )
}

/// The backend `spawn` opens a tab with under `terminal`, in the order `spawn` decides it: a
/// `terminal.spawn` template first, then the tmux preset, then the built-in iTerm2.
pub fn backend_name(terminal: &TerminalSettings) -> &'static str {
    if terminal.spawn.is_some() {
        "custom"
    } else if terminal.is_tmux() {
        "tmux"
    } else {
        "iterm2"
    }
}

/// Where this process is running, as the session record keeps it.
///
/// Read from inside the tab rather than from the settings, which only say where a *new* tab
/// would go: a session started under other settings, or in a tmux the settings do not name,
/// would otherwise be reported somewhere it is not. tmux says where a pane is through
/// `$TMUX` (whose first field is the server's socket) and `$TMUX_PANE`; outside tmux there
/// is nothing addressable to record beyond the backend.
///
/// So the backend here can differ from `backend_name`: a `terminal.spawn` template that
/// opens a tmux window records "tmux", because that is what a later attach has to talk to.
pub fn own_location(terminal: &TerminalSettings) -> SessionTerminal {
    location_with(
        run_shell,
        std::env::var("TMUX").ok().as_deref(),
        std::env::var("TMUX_PANE").ok().as_deref(),
        terminal,
    )
}

pub fn location_with(
    run: impl Fn(&str) -> Result<String, String>,
    tmux_env: Option<&str>,
    tmux_pane: Option<&str>,
    terminal: &TerminalSettings,
) -> SessionTerminal {
    let socket = tmux_env
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let pane = tmux_pane.map(str::trim).filter(|s| !s.is_empty());
    let (Some(socket), Some(pane)) = (socket, pane) else {
        return SessionTerminal {
            backend: backend_name(terminal).to_string(),
            socket: None,
            session: None,
            window: None,
            pane: None,
        };
    };
    // Asked once, now: a pane id stays the pane's for its life, and the window it sits in
    // is what the browser terminal will later attach to. `-u` for the same reason as
    // `tmux_window_home_script`: without a UTF-8 locale tmux turns the tabs into `_`.
    let cmd = format!(
        "{} display-message -p -t {} '#{{session_name}}\t#{{window_id}}\t#{{session_group}}'",
        tmux_cmd_prefix(Some(socket)).replacen("tmux", "tmux -u", 1),
        sh_quote(pane)
    );
    let (session, window) = match run(&cmd) {
        Ok(out) => {
            let mut fields = out.trim().split('\t');
            let (name, window, group) = (
                fields.next().unwrap_or(""),
                fields.next().unwrap_or(""),
                fields.next().unwrap_or(""),
            );
            // Started from inside a board connection's own session: the session the person
            // knows is the one it is grouped with, and the one that outlives the connection.
            let session = match is_own_session(name) && !group.is_empty() {
                true => group,
                false => name,
            };
            (
                Some(session.to_string()).filter(|s| !s.is_empty()),
                Some(window.to_string()).filter(|s| !s.is_empty()),
            )
        }
        Err(_) => (None, None),
    };
    SessionTerminal {
        backend: "tmux".to_string(),
        socket: Some(socket.to_string()),
        session,
        window,
        pane: Some(pane.to_string()),
    }
}

pub fn tmux_wake(
    socket: Option<&str>,
    pid: u32,
    line: Option<&str>,
    agent: Agent,
    dry_run: bool,
) -> Result<Performed, String> {
    let default_line = WORKER_WAKE_LINE;
    let wake_line = line.unwrap_or(default_line);
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        socket: socket.map(str::to_string),
        ..Default::default()
    };
    wake(&term, &Wake::default(), pid, "", wake_line, agent, dry_run)
}

pub fn tmux_focus(socket: Option<&str>, pid: u32, dry_run: bool) -> Result<Performed, String> {
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        socket: socket.map(str::to_string),
        ..Default::default()
    };
    focus(&term, pid, "", dry_run)
}

pub fn tmux_close(socket: Option<&str>, pid: u32, dry_run: bool) -> Result<Performed, String> {
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        socket: socket.map(str::to_string),
        ..Default::default()
    };
    close(&term, pid, "", dry_run)
}

pub fn tmux_spawn(
    socket: Option<&str>,
    session: Option<&str>,
    cwd: &str,
    title: &str,
    command: &[String],
    dry_run: bool,
) -> Result<Performed, String> {
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        socket: socket.map(str::to_string),
        session: session.map(str::to_string),
        ..Default::default()
    };
    let cmd_str = crate::template::sh_join(command);
    spawn(
        &term,
        &SpawnRequest {
            cwd,
            title,
            command: &cmd_str,
            title_command: None,
        },
        dry_run,
    )
}

#[cfg(test)]
mod tests;
