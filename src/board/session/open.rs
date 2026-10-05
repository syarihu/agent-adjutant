use std::sync::atomic::Ordering;

use crate::board::view::find_session;
use crate::board::{NEXT, Server, settings_now, target_of};
use crate::infra::template::{Sub, render, sh_join, sh_quote};
use crate::infra::terminal;

// ── open ─────────────────────────────────────────────────────────────

const NOT_SET: &str = "terminal.attach is not set: put the command that opens your terminal in the config's terminal.attach";

/// The terminal a session was opened in.
pub enum OpenedIn {
    Attach,
    ITerm2,
}

/// What opening a session in the person's terminal came to.
pub struct Opened {
    pub terminal: OpenedIn,
    pub session: String,
    pub window: String,
}

/// Open the session `id` in the person's terminal.
///
/// Through a session of its own in the group of the original, as the board terminal does
/// (`terminal::board_attach_prepare_script`): a plain attach to the shared session would move
/// every other client's current window to this one.
pub fn open(server: &Server, id: &str) -> Result<Opened, String> {
    let version = server.tmux.ok_or("tmux 3.1 or later is not available")?;
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    let (socket, window) =
        target_of(&session).ok_or("the session is not running in a tmux window")?;
    let socket = socket.as_deref();
    let attach = settings.terminal.attach.as_deref();
    if attach.is_none() && !terminal::iterm_available() {
        return Err(NOT_SET.to_string());
    }

    // Before anything is made: a window that is gone must not start a server or leave a
    // session behind.
    let home =
        terminal::run_shell(&terminal::tmux_window_home_script(socket, &window)).map_err(|e| {
            let lower = e.to_ascii_lowercase();
            match lower.contains("can't find")
                || lower.contains("no server running")
                || lower.contains("error connecting")
            {
                true => "the tmux window is gone".to_string(),
                false => e,
            }
        })?;
    let group = terminal::parse_window_home(&home).ok_or("the tmux window is gone")?;

    let _ = terminal::run_shell(&terminal::board_sweep_script(socket));
    let name = format!(
        "{}{}-{}",
        terminal::OPEN_SESSION_PREFIX,
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    );
    terminal::run_shell(&terminal::board_attach_prepare_script(
        socket, &group, &name, &window,
    ))?;

    let opened = match attach {
        Some(template) => {
            let command = render(
                template,
                &[
                    (
                        "socket",
                        Sub::Raw(&sh_join(&terminal::tmux_socket_args(socket))),
                    ),
                    ("session", Sub::Quoted(&name)),
                    ("window", Sub::Quoted(&window)),
                ],
            );
            terminal::run_shell(&command)
                .map(|_| OpenedIn::Attach)
                .map_err(|e| format!("terminal.attach failed: {e}"))
        }
        None => {
            // `-CC` is iTerm2's own way of drawing tmux's windows; any other terminal gets a
            // plain attach through its template.
            let line = terminal::native_attach_line(socket, &name, true, version >= (3, 4));
            terminal::iterm_attach(&line)
                .map(|_| OpenedIn::ITerm2)
                .map_err(|e| format!("iTerm2 could not open the session: {e}"))
        }
    };
    match opened {
        Ok(terminal) => Ok(Opened {
            terminal,
            session: name,
            window,
        }),
        Err(e) => {
            // Nothing attached to it, so nothing else is holding the group's windows.
            let _ = terminal::run_shell(&format!(
                "{} kill-session -t {}",
                terminal::tmux_cmd_prefix(socket),
                sh_quote(&format!("={name}"))
            ));
            Err(e)
        }
    }
}
