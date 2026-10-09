use super::*;

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

/// `hold` keeps the new window open when its command exits and prints the new pane's id, for a
/// caller that wants to read what the command left behind (see `tmux_watch_start`).
pub fn tmux_spawn_script(
    socket: Option<&str>,
    session: &str,
    cwd: &str,
    title: &str,
    command: &str,
    hold: bool,
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
    // A session made here starts from $HOME: the server keeps the directory it was started in
    // for good, and a window cannot be opened anywhere useful from one that has been removed.
    let ensure = format!(
        "{prefix} has-session -t {exact_q} 2>/dev/null || ( cd \"${{HOME:-/}}\" 2>/dev/null || cd /; {prefix} new-session -d -s {session_q} -n main -c \"$PWD\" )"
    );
    if !hold {
        return format!(
            "{ensure}; {prefix} new-window -d -t {next_q} -c {cwd_q} -n {title_q} {cmd_q}"
        );
    }
    // The window is opened on a placeholder and the real command is put in after
    // `remain-on-exit` is on: set after the real command had started, a hub that dies in a few
    // milliseconds (#182) would be gone before the option landed and leave nothing to read. By
    // pane id because `\; set-option` in the same call lands on the session's current pane, not
    // the new one. A failure of either step is the caller's, with no placeholder left behind.
    format!(
        "{ensure}; P=$({prefix} new-window -d -t {next_q} -c {cwd_q} -n {title_q} -P -F '#{{pane_id}}' 'sleep 60') || exit 1; {prefix} set-option -p -t \"$P\" remain-on-exit on 2>/dev/null; {prefix} respawn-pane -k -t \"$P\" -c {cwd_q} {cmd_q} || {{ {prefix} kill-pane -t \"$P\"; exit 1; }}; echo \"$P\""
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

/// The line, typed and not sent. The pause is the same one `tmux_wake_script` leaves before
/// Enter; here it is also what gives the agent time to draw the line before it is looked for.
pub(super) fn tmux_type_script(socket: Option<&str>, pane_id: &str, line: &str) -> String {
    let prefix = tmux_cmd_prefix(socket);
    format!(
        "{prefix} send-keys -l -t {} {} && sleep {WAKE_ENTER_DELAY}",
        sh_quote(pane_id),
        sh_quote(line)
    )
}

pub(super) fn tmux_enter_script(socket: Option<&str>, pane_id: &str) -> String {
    let prefix = tmux_cmd_prefix(socket);
    format!(
        "{prefix} send-keys -t {} Enter && echo {WOKE_MARKER}",
        sh_quote(pane_id)
    )
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

// ── watching a window that was just opened ───────────────────────────

/// What became of a pane opened by `tmux_spawn_script` with `hold`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartWatch {
    /// Still running when the watch ended.
    Alive,
    /// The command ended; what it printed last, so the caller can say why.
    Exited {
        status: Option<i32>,
        /// What it printed last; `None` when the screen could not be read, which is not the same
        /// as having printed nothing.
        screen: Option<String>,
    },
    /// The pane is gone, closed before it could be read.
    Gone,
}

/// How long a freshly started command is watched: a hub that cannot start dies within a
/// moment, and a window that is still up after this long is taken to be running.
const WATCH_POLLS: u32 = 12;
const WATCH_EVERY: Duration = Duration::from_millis(250);
/// How much of a dead pane's screen is reported.
const WATCH_SCREEN_LINES: usize = 10;

pub fn tmux_watch_start(socket: Option<&str>, pane: &str) -> Result<StartWatch, String> {
    tmux_watch_start_with(
        run_shell,
        std::thread::sleep,
        socket,
        pane,
        WATCH_POLLS,
        WATCH_EVERY,
    )
}

/// The same, with the runner and the pause handed in so a test need not wait.
pub fn tmux_watch_start_with(
    run: impl Fn(&str) -> Result<String, String>,
    pause: impl Fn(Duration),
    socket: Option<&str>,
    pane: &str,
    polls: u32,
    every: Duration,
) -> Result<StartWatch, String> {
    let prefix = tmux_cmd_prefix(socket);
    let pane_q = sh_quote(pane);
    let ask = format!(
        "{prefix} display-message -p -t {pane_q} '#{{pane_id}}\t#{{pane_dead}}\t#{{pane_dead_status}}'"
    );
    let unhold = format!("{prefix} set-option -p -t {pane_q} -u remain-on-exit");
    for nth in 0..polls {
        if nth > 0 {
            pause(every);
        }
        let answer = match run(&ask) {
            Ok(answer) => answer,
            Err(e) => {
                let lower = e.to_ascii_lowercase();
                if lower.contains("can't find pane") || lower.contains("no such pane") {
                    return Ok(StartWatch::Gone);
                }
                // Not read as the pane being fine: a tmux that cannot be asked is not an answer.
                // The window is let go of as it would be if it were running, whatever happens next.
                let _ = run(&unhold);
                return Err(e);
            }
        };
        let mut fields = answer.trim_end().split('\t');
        // tmux answers an empty line, status 0, for some panes that are gone.
        if fields.next() != Some(pane) {
            return Ok(StartWatch::Gone);
        }
        if fields.next() == Some("1") {
            let status = fields.next().and_then(|s| s.trim().parse().ok());
            let screen = run(&format!("{prefix} capture-pane -p -J -t {pane_q} -S -40"))
                .map(|out| last_lines(&out))
                .ok();
            // The window was kept for this look and has nothing more to show.
            let _ = run(&format!("{prefix} kill-pane -t {pane_q}"));
            return Ok(StartWatch::Exited { status, screen });
        }
    }
    // Still running: give back the window's normal way of closing with its command.
    let _ = run(&format!(
        "{prefix} set-option -p -t {pane_q} -u remain-on-exit"
    ));
    Ok(StartWatch::Alive)
}

fn last_lines(screen: &str) -> String {
    let lines: Vec<&str> = screen
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty() && !l.starts_with("Pane is dead"))
        .collect();
    lines[lines.len().saturating_sub(WATCH_SCREEN_LINES)..].join("\n")
}
