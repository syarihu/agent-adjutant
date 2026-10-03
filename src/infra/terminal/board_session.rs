use super::*;

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
pub(super) fn is_own_session(name: &str) -> bool {
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
    crate::infra::template::sh_join(&args)
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
