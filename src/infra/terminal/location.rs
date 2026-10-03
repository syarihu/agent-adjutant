use super::*;
use serde::{Deserialize, Serialize};

/// Where the session runs: its terminal backend, and for tmux the socket, session and window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTerminal {
    /// Terminal backend name: "tmux", "iterm2" (the built-in one), or "custom" for a
    /// `terminal.spawn` template.
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

/// The backend `spawn` opens a tab with under `terminal`, in the order `spawn` decides it: a
/// `terminal.spawn` template first, then the tmux preset, then the built-in iTerm2.
pub fn backend_name(terminal: &TerminalSettings) -> &'static str {
    if terminal.spawn.is_some() {
        "custom"
    } else {
        Backend::of(terminal).name()
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
