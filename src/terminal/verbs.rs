use super::*;

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
pub(super) const MAX_INLINE_COMMAND: usize = 900;

/// Put a long command line in a file and return the short one that runs it.
///
/// Left behind rather than self-deleting: the script is read by a shell in another process
/// at a time we do not get to know, and a file removed before that read is a tab that opens
/// on an error. Old ones are swept on the way past instead.
pub(super) fn stage_command(line: &str) -> Result<String, String> {
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
pub(super) fn close_with(
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

/// The line and its Enter go as two writes. `write text "<line>"` sends both in one burst,
/// and an agent's input box takes a long enough burst as a paste: the newline lands in the
/// box as text and nothing is submitted. Observed on Claude Code 2.1.282, where a line of
/// about 50 characters still submitted and one of 79 did not — and a paste threshold is the
/// agent's to move, so no line is short enough to count on.
pub(super) fn default_wake_command(tty: &str, line: &str) -> String {
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
pub(super) fn wake_with(
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
pub(super) fn wake_with_clock(
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

/// How long to wait for the agent to come back to its prompt, looking every so often. Long
/// enough for a turn that is just ending; a person who is answering a question is not waited
/// for, and falls back to being notified.
pub(super) const WAKE_READY_BUDGET: Duration = Duration::from_secs(5);
pub(super) const WAKE_READY_POLL: Duration = Duration::from_millis(500);

/// After typing: how many times to look for the line at the prompt, and how far apart. The
/// agent draws what it is sent asynchronously, so the first look can be too early.
pub(super) const WAKE_ECHO_LOOKS: u32 = 5;
const WAKE_ECHO_POLL: Duration = Duration::from_millis(250);

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
pub(super) fn lock_pane(
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
