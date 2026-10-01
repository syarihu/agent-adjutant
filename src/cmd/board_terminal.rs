//! The terminal a board opens on a session that runs in tmux, over a WebSocket.
//!
//! A pseudo-terminal runs `tmux attach` and this relays its bytes to the browser and the
//! browser's back. It is the same terminal a person would get by attaching from a shell, so
//! nothing about the session changes: the agent is not restarted, and closing the page only
//! detaches. Other clients are left alone by attaching through a session of its own in the
//! group of the original (see `terminal::board_attach_prepare_script`).
//!
//! What the page names is a session id from the board's own state; the socket and the window
//! come from the record, never from the request.

use super::serve::Server;
use crate::http::Request;
use std::io::{BufReader, Write};
use std::net::TcpStream;

/// Board terminals open at once, over every board: each is a process, a thread pair and a tmux
/// client.
#[cfg(any(unix, test))]
const MAX_TERMINALS: usize = 8;

/// Close codes the page is told, beside the standard ones. In the 4000 range, which is for
/// applications.
#[cfg(any(unix, test))]
const CLOSE_NO_SESSION: u16 = 4404;
#[cfg(any(unix, test))]
const CLOSE_TMUX_FAILED: u16 = 4500;

/// A tmux window id: `@` and digits, the only thing a record's `window` is allowed to be.
pub(super) fn is_window_id(window: &str) -> bool {
    window
        .strip_prefix('@')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The socket and window of `session`, if it is one a terminal can be opened on: it runs in
/// tmux, was recorded with a window, and is running. Shared with the board's action that opens
/// the session in the person's own terminal.
pub(super) fn target_of(session: &crate::session::Session) -> Option<(Option<String>, String)> {
    let terminal = &session.terminal;
    let window = terminal.window.as_deref().filter(|w| is_window_id(w))?;
    (terminal.backend == "tmux" && session.present)
        .then(|| (terminal.socket.clone(), window.to_string()))
}

#[cfg(unix)]
pub(super) use imp::serve;

#[cfg(not(unix))]
pub(super) fn serve(
    _server: &Server,
    _id: &str,
    _req: &Request,
    mut stream: TcpStream,
    _reader: BufReader<TcpStream>,
) -> std::io::Result<()> {
    crate::http::json(&mut stream, 404, r#"{"error":"no such route"}"#)
}

#[cfg(unix)]
mod imp {
    use super::*;
    use crate::cmd::serve::{board_session, settings_now};
    use crate::pty;
    use crate::terminal;
    use crate::ws;
    use std::io::Read;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Numbers the sessions this process makes, so that two connections never share a name.
    static NEXT: AtomicUsize = AtomicUsize::new(1);

    /// A slot among `MAX_TERMINALS`, given back when this goes out of scope.
    struct Slot(Arc<AtomicUsize>);

    impl Slot {
        fn take(open: &Arc<AtomicUsize>) -> Option<Slot> {
            open.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < MAX_TERMINALS).then_some(n + 1)
            })
            .ok()
            .map(|_| Slot(Arc::clone(open)))
        }
    }

    impl Drop for Slot {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// What the socket is written through: the connection's thread answers pings and the
    /// reader thread sends output, and a frame must not be cut into by the other's.
    type Wire = Arc<Mutex<TcpStream>>;

    fn send(wire: &Wire, frame: &[u8]) -> std::io::Result<()> {
        let mut stream = wire.lock().unwrap_or_else(|e| e.into_inner());
        stream.write_all(frame)?;
        stream.flush()
    }

    pub(in crate::cmd) fn serve(
        server: &Server,
        id: &str,
        req: &Request,
        mut stream: TcpStream,
        reader: BufReader<TcpStream>,
    ) -> std::io::Result<()> {
        let accept = match ws::accept_key(&req.headers) {
            Ok(accept) => accept,
            Err(why) => {
                return crate::http::json(
                    &mut stream,
                    400,
                    &serde_json::json!({ "error": why }).to_string(),
                );
            }
        };
        let Some(_slot) = Slot::take(&server.terminals) else {
            return crate::http::json(
                &mut stream,
                503,
                &serde_json::json!({ "error": "too many open terminals" }).to_string(),
            );
        };
        // From here on the socket carries frames, for as long as the person keeps the terminal
        // open — the 10 s a request gets to say something does not apply.
        stream.set_read_timeout(Some(KEEPALIVE))?;
        stream.set_write_timeout(Some(KEEPALIVE))?;
        stream.set_nodelay(true)?;
        stream.write_all(ws::handshake_response(&accept).as_bytes())?;
        let wire: Wire = Arc::new(Mutex::new(stream));

        let (cols, rows) = (size_param(req, "cols", 80), size_param(req, "rows", 24));
        match open(server, id, cols, rows) {
            Ok(terminal) => relay(terminal, wire, reader),
            Err((code, why)) => {
                let _ = send(&wire, &ws::encode_close(code, &why));
                Ok(())
            }
        }
    }

    /// Silence this long is answered with a ping, and three in a row end the connection: a pong
    /// can queue behind a large write of output, so one or two misses do not mean the page is gone.
    const KEEPALIVE: Duration = Duration::from_secs(30);

    fn size_param(req: &Request, name: &str, default: u16) -> u16 {
        req.param(name)
            .and_then(|v| v.parse::<u64>().ok())
            .map(clamp_size)
            .unwrap_or(default)
    }

    /// One to a thousand: a terminal has at least a cell, and a number the page picked is not
    /// allowed to size a buffer in tmux.
    pub(super) fn clamp_size(n: u64) -> u16 {
        n.clamp(1, 1000) as u16
    }

    /// `(cols, rows)` out of a control message from the page, clamped, or `None` for any other.
    pub(super) fn parse_control(text: &str) -> Option<(u16, u16)> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        if value.get("type")?.as_str()? != "resize" {
            return None;
        }
        Some((
            clamp_size(value.get("cols")?.as_u64()?),
            clamp_size(value.get("rows")?.as_u64()?),
        ))
    }

    /// The tmux session a board connection was resolved to, and what it runs in.
    ///
    /// The fields drop in order, so the client is taken down before the session made for it is
    /// given back — on any path, including an early return or a panic.
    struct Attached {
        pty: pty::Pty,
        _release: Release,
        /// The window the terminal was opened on, which it ends with.
        window: String,
    }

    /// Gives the session `name` back when dropped.
    struct Release {
        socket: Option<String>,
        name: String,
    }

    impl Drop for Release {
        fn drop(&mut self) {
            release(self.socket.as_deref(), &self.name);
        }
    }

    /// Resolve `id` to a live tmux window and attach to it, or say why not as a close code.
    fn open(server: &Server, id: &str, cols: u16, rows: u16) -> Result<Attached, (u16, String)> {
        // The same rule the page is shown (`boardTerminal.available`): no usable tmux, no terminal.
        let version = server.tmux.ok_or((
            CLOSE_NO_SESSION,
            "tmux 3.1 or later is not available".to_string(),
        ))?;
        let settings = settings_now(server);
        let (socket, window) = board_session(server, &settings, id)
            .and_then(|session| target_of(&session))
            .ok_or((
                CLOSE_NO_SESSION,
                "no such tmux session on this board".to_string(),
            ))?;
        let socket = socket.as_deref();

        // Before anything is made: a window that is gone must not start a server or leave a
        // session behind.
        let home = terminal::run_shell(&terminal::tmux_window_home_script(socket, &window))
            .map_err(|e| {
                let lower = e.to_ascii_lowercase();
                match lower.contains("can't find")
                    || lower.contains("no server running")
                    || lower.contains("error connecting")
                {
                    true => (CLOSE_NO_SESSION, "the tmux window is gone".to_string()),
                    false => (CLOSE_TMUX_FAILED, e),
                }
            })?;
        let group = terminal::parse_window_home(&home)
            .ok_or((CLOSE_NO_SESSION, "the tmux window is gone".to_string()))?;

        let _ = terminal::run_shell(&terminal::board_sweep_script(socket));
        let name = format!(
            "{}{}-{}",
            terminal::BOARD_SESSION_PREFIX,
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        );
        // Before the session exists, so that nothing after this can leave it behind: whatever
        // happens from here, dropping the guard gives it back (a session that was never made
        // is left as it is).
        let release = Release {
            socket: socket.map(str::to_string),
            name: name.clone(),
        };
        terminal::run_shell(&terminal::board_attach_prepare_script(
            socket, &group, &name, &window,
        ))
        .map_err(|e| (CLOSE_TMUX_FAILED, e))?;

        let mut command = Command::new("tmux");
        command
            .args(terminal::board_attach_args(
                socket,
                &name,
                version >= (3, 3),
            ))
            .env("TERM", "xterm-256color")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        let pty = pty::spawn(command, cols, rows)
            .map_err(|e| (CLOSE_TMUX_FAILED, format!("cannot start tmux: {e}")))?;
        Ok(Attached {
            pty,
            _release: release,
            window,
        })
    }

    /// Give the session back once its client is gone, unless it is all that is left of the
    /// original's windows.
    fn release(socket: Option<&str>, name: &str) {
        match terminal::run_shell(&terminal::board_release_script(socket, name)) {
            Ok(said) if said.contains(terminal::KEPT_MARKER) => eprintln!(
                "adj server: kept tmux session {name}: the original session is gone, so it holds the windows now"
            ),
            _ => {}
        }
    }

    /// How often the target window is looked for.
    const WATCH_EVERY: Duration = Duration::from_secs(1);

    /// Detach the client of the session `name` once `window` is no longer one of its windows,
    /// or return when `stop` is set. A tmux that does not answer is not taken to mean the window
    /// is gone, and a detach that fails is tried again on the next look rather than given up,
    /// since until it succeeds the page shows whichever window tmux moved on to. That is at most
    /// `WATCH_EVERY` in the common case.
    fn watch_window(stop: &AtomicBool, socket: Option<&str>, name: &str, window: &str) {
        let step = Duration::from_millis(100);
        'watching: loop {
            let mut waited = Duration::ZERO;
            while waited < WATCH_EVERY {
                if stop.load(Ordering::SeqCst) {
                    break 'watching;
                }
                std::thread::sleep(step);
                waited += step;
            }
            if let Ok(windows) = terminal::run_shell(&terminal::board_windows_script(socket, name))
                && !windows.lines().any(|w| w.trim() == window)
                && terminal::run_shell(&terminal::board_detach_script(socket, name)).is_ok()
            {
                break;
            }
        }
    }

    /// Carry bytes both ways until either side is done, then take everything down.
    fn relay(
        attached: Attached,
        wire: Wire,
        mut reader: BufReader<TcpStream>,
    ) -> std::io::Result<()> {
        // Bound in this order on purpose: bindings drop in reverse, so on any early return the
        // client (which is dropped by shutting it down) goes before the session made for it is
        // given back.
        let Attached {
            _release,
            mut pty,
            window,
        } = attached;
        let mut master = pty.reader()?;
        // Set once the connection's side has sent its own close frame, so that the client going
        // away afterwards is not announced a second time.
        let closing = Arc::new(AtomicBool::new(false));
        let pump = {
            let (wire, closing) = (Arc::clone(&wire), Arc::clone(&closing));
            std::thread::spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                // EOF is how a hung-up terminal ends on some systems and EIO on others.
                while let Ok(n) = master.read(&mut buf) {
                    if n == 0 || send(&wire, &ws::encode(ws::OP_BINARY, &buf[..n])).is_err() {
                        break;
                    }
                }
                if !closing.load(Ordering::SeqCst) {
                    let _ = send(&wire, &ws::encode_close(ws::CLOSE_NORMAL, "detached"));
                }
                if let Ok(stream) = wire.lock() {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                }
            })
        };

        // The client is shown one window. If it goes (the agent exited), tmux would move the
        // client on to another window of the group and what is typed next would reach another
        // agent — so the terminal ends instead. Polled rather than hooked: see
        // `terminal::board_windows_script`.
        let stop = Arc::new(AtomicBool::new(false));
        let watch = {
            let stop = Arc::clone(&stop);
            let socket = _release.socket.clone();
            let name = _release.name.clone();
            std::thread::spawn(move || watch_window(&stop, socket.as_deref(), &name, &window))
        };

        let mut decoder = ws::Decoder::new();
        let mut buf = [0u8; 16 * 1024];
        let mut silent = 0;
        let mut told = false;
        'connection: loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    silent = 0;
                    decoder.push(&buf[..n]);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    silent += 1;
                    if silent >= 3 || send(&wire, &ws::encode(ws::OP_PING, b"")).is_err() {
                        break;
                    }
                    continue;
                }
                Err(_) => break,
            }
            loop {
                match decoder.next() {
                    Ok(None) => break,
                    Ok(Some(ws::Message::Binary(bytes))) => {
                        if pty.writer().write_all(&bytes).is_err() {
                            break 'connection;
                        }
                    }
                    Ok(Some(ws::Message::Text(text))) => {
                        if let Some((cols, rows)) = parse_control(&text) {
                            let _ = pty.resize(cols, rows);
                        }
                    }
                    Ok(Some(ws::Message::Ping(payload))) => {
                        let _ = send(&wire, &ws::encode(ws::OP_PONG, &payload));
                    }
                    Ok(Some(ws::Message::Pong(_))) => {}
                    Ok(Some(ws::Message::Close { code, .. })) => {
                        let code = code.unwrap_or(ws::CLOSE_NORMAL);
                        let _ = send(&wire, &ws::encode_close(code, ""));
                        told = true;
                        break 'connection;
                    }
                    Err(e) => {
                        let _ = send(&wire, &ws::encode_close(e.close_code(), e.reason()));
                        told = true;
                        break 'connection;
                    }
                }
            }
        }

        // The one place this ends, however it got here: the client goes, then the thread that
        // reads it, and the session made for it and the slot are given back as this returns.
        closing.store(told, Ordering::SeqCst);
        stop.store(true, Ordering::SeqCst);
        pty.shutdown(Duration::from_secs(1));
        let _ = pump.join();
        let _ = watch.join();
        if let Ok(stream) = wire.lock() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::imp::*;
    use super::*;
    use crate::session::{Session, SessionTerminal};

    fn session(id: &str, backend: &str, window: Option<&str>, present: bool) -> Session {
        Session {
            id: id.to_string(),
            kind: "worker".to_string(),
            agent: "claude".to_string(),
            terminal: SessionTerminal {
                backend: backend.to_string(),
                socket: Some("adj-test".to_string()),
                session: Some("work".to_string()),
                window: window.map(str::to_string),
                pane: None,
            },
            hub: None,
            key: None,
            worktree: "/tmp/w".to_string(),
            branch: None,
            task: None,
            title: None,
            conversation: None,
            present,
            stale: false,
            pid: None,
            started_at: None,
            phase: None,
            phase_at: None,
            phases: Vec::new(),
            last_activity_at: None,
            attached: None,
            waiting: None,
        }
    }

    #[test]
    fn only_a_running_tmux_session_with_a_window_id_can_be_opened() {
        let sessions = [
            session("worker-a", "tmux", Some("@3"), true),
            session("worker-b", "iterm2", Some("@3"), true),
            session("worker-c", "tmux", None, true),
            session("worker-d", "tmux", Some("main:1"), true),
            session("worker-e", "tmux", Some("@3"), false),
        ];
        assert_eq!(
            target_of(&sessions[0]),
            Some((Some("adj-test".to_string()), "@3".to_string()))
        );
        for other in &sessions[1..] {
            assert_eq!(target_of(other), None, "{}", other.id);
        }
    }

    #[test]
    fn window_ids_are_an_at_sign_and_digits() {
        assert!(is_window_id("@0"));
        assert!(is_window_id("@123"));
        for bad in ["", "@", "3", "@a", "@1 ", "@1;ls", "=x:@1", "@-1"] {
            assert!(!is_window_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_resize_is_clamped_and_anything_else_is_ignored() {
        assert_eq!(
            parse_control(r#"{"type":"resize","cols":120,"rows":40}"#),
            Some((120, 40))
        );
        assert_eq!(
            parse_control(r#"{"type":"resize","cols":0,"rows":99999}"#),
            Some((1, 1000))
        );
        for other in [
            r#"{"type":"focus"}"#,
            r#"{"type":"resize","cols":"x","rows":1}"#,
            r#"{"type":"resize","cols":-3,"rows":1}"#,
            r#"{"type":"resize","cols":10}"#,
            "resize",
            "",
        ] {
            assert_eq!(parse_control(other), None, "{other:?}");
        }
    }

    #[test]
    fn the_cap_and_the_close_codes_are_the_ones_the_page_reads() {
        assert_eq!(MAX_TERMINALS, 8);
        assert_eq!(CLOSE_NO_SESSION, 4404);
        assert_eq!(CLOSE_TMUX_FAILED, 4500);
    }
}
