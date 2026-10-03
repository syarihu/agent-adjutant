//! The board terminal: a tmux session of a worker, opened in the browser over a WebSocket.
//!
//! The tmux here is a server of its own (`-L <unique name>`), so nothing touches the developer's
//! sessions, and the resident server is started on a free port with a scratch config and state
//! directory. Without tmux the tests that need it are skipped, as `tmux.rs` does.

mod common;

use common::*;
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// How long to wait for anything the server or tmux has to start processes for. Under load a
/// single spawn can take seconds and the first frame sits behind a chain of them (git, ps, sh,
/// tmux, tmux attach); a passing test returns as soon as its condition holds, so this only
/// bounds a real hang.
const PATIENCE: Duration = Duration::from_secs(60);

const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
const ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

fn config(tmux: &IsolatedTmux) -> String {
    serde_json::json!({
        "notification": "true",
        "terminal": { "preset": "tmux", "session": tmux.session, "socket": tmux.socket },
        "repos": {
            "acme/widget": {
                "taskSource": "github",
                "issueRepo": "acme/widget",
                "issueKeys": { "acme/widget": "WID" },
                "ide": "code"
            }
        }
    })
    .to_string()
}

/// A tmux name no other test, or copy of this binary, shares: `IsolatedTmux` derives its socket
/// from the clock alone, which can tie on parallel starts.
fn unique(name: &str) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// What the isolated tmux holds: a session showing `main`, and a window running `cat` that
/// has printed a marker, which is the one the board is to open.
struct Layout {
    socket_path: String,
    session: String,
    main_window: String,
    target_window: String,
}

impl IsolatedTmux {
    fn out(&self, args: &[&str]) -> String {
        let out = self.tmux_cmd(args);
        assert!(
            out.status.success(),
            "tmux {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn lay_out(&self) -> Layout {
        self.out(&[
            "new-session",
            "-d",
            "-s",
            &self.session,
            "-x",
            "200",
            "-y",
            "50",
            "-n",
            "main",
            "sleep",
            "600",
        ]);
        let target_window = self.out(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{window_id}",
            "-t",
            &self.session,
            "-n",
            "target",
            "sh",
            "-c",
            "echo ADJ_MARK; cat",
        ]);
        let main_window = self.out(&["display-message", "-p", "-t", &self.session, "#{window_id}"]);
        let socket_path = self.out(&["display-message", "-p", "#{socket_path}"]);
        Layout {
            socket_path,
            session: self.session.clone(),
            main_window,
            target_window,
        }
    }

    fn current_window(&self, session: &str) -> String {
        self.out(&[
            "display-message",
            "-p",
            "-t",
            &format!("={session}:"),
            "#{window_id}",
        ])
    }

    fn board_sessions(&self) -> Vec<String> {
        self.out(&["list-sessions", "-F", "#{session_name}"])
            .lines()
            .filter(|name| name.starts_with("adjboard-"))
            .map(str::to_string)
            .collect()
    }
}

/// Wait for `check` to hold, for as long as tmux and the server can reasonably take.
fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !check() {
        assert!(Instant::now() < deadline, "never: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The main checkout's worker, recorded as running in `layout`'s target window. The pid is the
/// test's own, which is alive and started when the record says.
fn forge_worker(fixture: &Fixture, layout: &Layout) {
    let pid = std::process::id();
    let record = fixture.repo.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": pid,
            "title": "WID-1",
            "psStarted": ps_started(pid),
            "terminal": {
                "backend": "tmux",
                "socket": layout.socket_path,
                "session": layout.session,
                "window": layout.target_window,
                "pane": "%1",
            },
        })
        .to_string(),
    )
    .unwrap();
}

// ── a WebSocket client, by hand ──────────────────────────────────────

/// Fill `buf`, retrying while the socket's short read timeout runs out, until `deadline`. Not
/// `read_exact` in a loop: that does not say how much it consumed when it fails, so a retry
/// could resume in the middle of a frame. The server closing the connection is a failure too,
/// as it was when `read_exact` was used here.
fn read_fully(stream: &mut TcpStream, buf: &mut [u8], deadline: Instant, what: &str) {
    let started = Instant::now();
    let mut filled = 0;
    while filled < buf.len() {
        // Checked before every read, not only after a timeout: a trickle of bytes never times
        // out, and would otherwise carry the wait past the deadline.
        assert!(
            Instant::now() < deadline,
            "no {what} before the deadline (this read began {:?} ago)",
            started.elapsed()
        );
        match stream.read(&mut buf[filled..]) {
            Ok(0) => panic!("the connection closed while waiting for {what}"),
            Ok(n) => filled += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(e) => panic!("reading {what}: {e}"),
        }
    }
}

/// The status line and headers of the answer, read up to the blank line and no further: what
/// follows may already be frames.
fn read_head(stream: &mut TcpStream) -> String {
    let deadline = Instant::now() + PATIENCE;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        read_fully(stream, &mut byte, deadline, "the handshake answer");
        head.push(byte[0]);
    }
    String::from_utf8(head).unwrap()
}

fn status_of(head: &str) -> u16 {
    head.split_whitespace().nth(1).unwrap().parse().unwrap()
}

/// Send a handshake and read the answer.
fn handshake(port: u16, target: &str, origin: Option<&str>) -> (String, TcpStream) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let origin = origin
        .map(|o| format!("Origin: {o}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {KEY}\r\n\
         {origin}\r\n"
    )
    .unwrap();
    let head = read_head(&mut stream);
    (head, stream)
}

fn terminal_path(resident: &Resident, id: &str, size: &str) -> String {
    format!(
        "/b/{SLUG}/api/sessions/{id}/terminal?token={}&{size}",
        resident.token
    )
}

fn own_origin(resident: &Resident) -> String {
    format!("http://127.0.0.1:{}", resident.port)
}

/// A frame from the browser: masked, as they have to be.
fn client_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mask = [0x12, 0x34, 0x56, 0x78];
    assert!(payload.len() < 126);
    let mut out = vec![0x80 | opcode, 0x80 | payload.len() as u8];
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// One frame from the server, as `(opcode, payload)`.
fn read_frame(stream: &mut TcpStream, deadline: Instant) -> (u8, Vec<u8>) {
    let what = "a frame from the server";
    let mut head = [0u8; 2];
    read_fully(stream, &mut head, deadline, what);
    assert_eq!(head[1] & 0x80, 0, "server frames are not masked");
    let len = match head[1] & 0x7f {
        126 => {
            let mut two = [0u8; 2];
            read_fully(stream, &mut two, deadline, what);
            usize::from(u16::from_be_bytes(two))
        }
        127 => {
            let mut eight = [0u8; 8];
            read_fully(stream, &mut eight, deadline, what);
            u64::from_be_bytes(eight) as usize
        }
        short => usize::from(short),
    };
    let mut payload = vec![0u8; len];
    read_fully(stream, &mut payload, deadline, what);
    (head[0] & 0x0f, payload)
}

/// Answer a ping, which the server sends after 30 s of silence and holds against the client
/// after three unanswered: a test that waits long enough has to stay a client it keeps.
fn pong(stream: &mut TcpStream, opcode: u8, payload: &[u8]) {
    if opcode == 0x9 {
        stream.write_all(&client_frame(0xA, payload)).unwrap();
    }
}

/// Read until the server's close frame and return its payload. Output already on its way comes
/// first, and is skipped.
fn read_close(stream: &mut TcpStream) -> Vec<u8> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        assert!(
            Instant::now() < deadline,
            "the server never closed, though it kept sending"
        );
        let (opcode, payload) = read_frame(stream, deadline);
        if opcode == 0x8 {
            return payload;
        }
        pong(stream, opcode, &payload);
    }
}

/// Read output until `wanted` has appeared in it.
fn read_until(stream: &mut TcpStream, seen: &mut String, wanted: &str) {
    let deadline = Instant::now() + PATIENCE;
    while !seen.contains(wanted) {
        assert!(
            Instant::now() < deadline,
            "never saw {wanted:?} in {seen:?}"
        );
        let (opcode, payload) = read_frame(stream, deadline);
        match opcode {
            0x2 => seen.push_str(&String::from_utf8_lossy(&payload)),
            0x9 => pong(stream, opcode, &payload),
            0xA => {}
            other => {
                panic!("unexpected frame {other:#x} while waiting for {wanted:?}: {payload:?}")
            }
        }
    }
}

// ── the tests ────────────────────────────────────────────────────────

#[test]
fn a_worker_s_tmux_window_is_opened_in_the_browser_without_disturbing_the_session() {
    let Some(tmux) = IsolatedTmux::new(&unique("board")) else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let layout = tmux.lay_out();
    let fixture = Fixture::new(&config(&tmux));
    forge_worker(&fixture, &layout);
    let resident = Resident::start(&fixture);

    // The board says the terminal can be opened at all.
    let (status, state) = resident.get(&format!("/b/{SLUG}/api/state"));
    assert_eq!(status, 200, "{state}");
    let state: serde_json::Value = serde_json::from_str(&state).unwrap();
    assert_eq!(state["boardTerminal"]["available"], true);
    let worker = state["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "worker-main")
        .unwrap_or_else(|| panic!("no worker-main in {state}"));
    assert_eq!(worker["present"], true);
    assert_eq!(worker["terminal"]["window"], layout.target_window.as_str());

    let path = terminal_path(&resident, "worker-main", "cols=100&rows=30");
    let (head, mut ws) = handshake(resident.port, &path, Some(&own_origin(&resident)));
    assert_eq!(status_of(&head), 101, "{head}");
    assert!(
        head.contains(&format!("Sec-WebSocket-Accept: {ACCEPT}")),
        "{head}"
    );
    assert!(!head.contains("Sec-WebSocket-Extensions"), "{head}");

    // What the window has on screen arrives as terminal bytes.
    let mut seen = String::new();
    read_until(&mut ws, &mut seen, "ADJ_MARK");

    // The terminal opened at the size asked for in the URL, and follows a resize.
    let sizes = || tmux.out(&["list-clients", "-F", "#{client_width}x#{client_height}"]);
    eventually("the client to have the size from the URL", || {
        sizes().lines().any(|l| l == "100x30")
    });
    ws.write_all(&client_frame(
        0x1,
        br#"{"type":"resize","cols":120,"rows":40}"#,
    ))
    .unwrap();
    eventually("the client to follow the resize", || {
        sizes().lines().any(|l| l == "120x40")
    });

    // Keys go to the window: `cat` gets them, and the terminal shows them.
    ws.write_all(&client_frame(0x2, b"hello\r")).unwrap();
    read_until(&mut ws, &mut seen, "hello");

    // The person in the original session is not moved by any of this: their current window is
    // as it was and the board's client is not one of theirs.
    assert_eq!(tmux.current_window(&layout.session), layout.main_window);
    let theirs = tmux.out(&[
        "list-clients",
        "-t",
        &format!("={}", layout.session),
        "-F",
        "#{client_session}",
    ]);
    assert!(
        theirs.is_empty(),
        "the board attached to the original session: {theirs}"
    );
    let made = tmux.board_sessions();
    assert_eq!(made.len(), 1, "{made:?}");
    assert_eq!(tmux.current_window(&made[0]), layout.target_window);

    // Closing the page detaches and takes the session made for it with it. The agent's window
    // is untouched.
    ws.write_all(&client_frame(0x8, &1000u16.to_be_bytes()))
        .unwrap();
    let echoed = read_close(&mut ws);
    assert_eq!(&echoed[..2], &1000u16.to_be_bytes(), "the close is echoed");
    drop(ws);
    eventually("the board session to be gone", || {
        tmux.board_sessions().is_empty()
    });
    let windows = tmux.out(&["list-windows", "-a", "-F", "#{window_id}"]);
    assert!(
        windows.lines().any(|w| w == layout.target_window),
        "{windows}"
    );
    assert!(
        windows.lines().any(|w| w == layout.main_window),
        "{windows}"
    );
    assert_eq!(tmux.current_window(&layout.session), layout.main_window);
}

#[test]
fn opening_a_terminal_asks_about_its_own_session_and_no_other() {
    let Some(tmux) = IsolatedTmux::new(&unique("board-one")) else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let layout = tmux.lay_out();
    let fixture = Fixture::new(&config(&tmux));
    let pid = std::process::id();
    // Three worktrees, each with a record; only the first runs in the window the board opens.
    for (name, worker_pid) in [("spy-target", pid), ("spy-other-a", 1), ("spy-other-b", 2)] {
        let worktree = fixture.repo.parent().unwrap().join(name);
        let out = Command::new("git")
            .hermetic()
            .args(["worktree", "add", "-q", "-b", name])
            .arg(&worktree)
            .current_dir(&fixture.repo)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let mut record = serde_json::json!({
            "pid": worker_pid,
            "psStarted": ps_started(worker_pid),
            "title": name,
        });
        if worker_pid == pid {
            record["terminal"] = serde_json::json!({
                "backend": "tmux",
                "socket": layout.socket_path,
                "session": layout.session,
                "window": layout.target_window,
                "pane": "%1",
            });
        }
        std::fs::create_dir_all(worktree.join(".claude")).unwrap();
        std::fs::write(
            worktree.join(".claude").join("adjutant-worker.json"),
            record.to_string(),
        )
        .unwrap();
    }
    let spy = Spy::new(fixture._dir.path());
    let resident = Resident::start_with(&fixture, &[("PATH", &spy.path())]);
    spy.clear();

    let path = terminal_path(&resident, "worker-spy-target", "cols=100&rows=30");
    let (head, mut ws) = handshake(resident.port, &path, Some(&own_origin(&resident)));
    assert_eq!(status_of(&head), 101, "{head}");
    let mut seen = String::new();
    read_until(&mut ws, &mut seen, "ADJ_MARK");

    let calls = spy.calls();
    let asked = |needle: &str| calls.iter().filter(|c| c.contains(needle)).count();
    assert!(
        !calls.iter().any(|c| c.contains("spy-other")),
        "another worktree was asked about: {calls:?}"
    );
    // The branch comes out of the worktree listing, not from a `git branch` of its own, and
    // the listing is read once for the session, besides the one that locates the repository.
    assert_eq!(asked("branch --show-current"), 0, "{calls:?}");
    assert_eq!(asked("worktree list"), 2, "{calls:?}");
    let ps: Vec<&String> = calls.iter().filter(|c| c.starts_with("ps ")).collect();
    assert!(!ps.is_empty(), "{calls:?}");
    assert!(
        ps.iter().all(|c| c.ends_with(&format!("-p {pid}"))),
        "{calls:?}"
    );
    // The target's own window is looked at, and only its server is listed.
    assert_eq!(asked("list-panes"), 1, "{calls:?}");
    assert!(asked("display-message") >= 1, "{calls:?}");
    assert_eq!(asked("new-session"), 1, "{calls:?}");
    assert_eq!(asked("attach-session"), 1, "{calls:?}");
}

#[test]
fn a_handshake_is_refused_unless_it_comes_from_the_board_itself() {
    let Some(tmux) = IsolatedTmux::new(&unique("board-auth")) else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let layout = tmux.lay_out();
    let fixture = Fixture::new(&config(&tmux));
    forge_worker(&fixture, &layout);
    let resident = Resident::start(&fixture);
    let path = terminal_path(&resident, "worker-main", "cols=80&rows=24");

    let (head, _) = handshake(resident.port, &path, None);
    assert_eq!(status_of(&head), 403, "no Origin: {head}");
    let (head, _) = handshake(resident.port, &path, Some("http://evil.example"));
    assert_eq!(status_of(&head), 403, "foreign Origin: {head}");
    let (head, _) = handshake(
        resident.port,
        &path.replace(&resident.token, "guess"),
        Some(&own_origin(&resident)),
    );
    assert_eq!(status_of(&head), 403, "bad token: {head}");
    assert!(tmux.board_sessions().is_empty());
}

#[test]
fn a_session_that_is_not_in_tmux_is_closed_with_4404() {
    let Some(tmux) = IsolatedTmux::new(&unique("board-none")) else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let layout = tmux.lay_out();
    let fixture = Fixture::new(&config(&tmux));
    forge_worker(&fixture, &layout);
    let resident = Resident::start(&fixture);

    // A session the board does not have, and a hub that was never recorded in a window.
    for id in ["worker-nowhere", "hub"] {
        let path = terminal_path(&resident, id, "cols=80&rows=24");
        let (head, mut ws) = handshake(resident.port, &path, Some(&own_origin(&resident)));
        assert_eq!(status_of(&head), 101, "{id}: {head}");
        // The close comes first: nothing was attached, so there is no output to skip, and the
        // server closes long before it would ping.
        let (opcode, payload) = read_frame(&mut ws, Instant::now() + PATIENCE);
        assert_eq!(opcode, 0x8, "{id}");
        assert_eq!(u16::from_be_bytes([payload[0], payload[1]]), 4404, "{id}");
        assert!(!payload[2..].is_empty(), "{id}: a reason is given");
    }
    assert!(tmux.board_sessions().is_empty());
}

#[test]
fn the_terminal_is_not_a_route_on_a_dedicated_board() {
    let fixture = Fixture::new(QUIET);
    let mut board = fixture
        .command(["serve", "--port", "0", "--no-open"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut said = String::new();
    std::io::BufReader::new(board.stdout.as_mut().unwrap())
        .read_line(&mut said)
        .unwrap();
    let url = said.split(" — ").nth(1).unwrap().trim().to_string();
    let (host, query) = url
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap();
    let port: u16 = host.rsplit(':').next().unwrap().parse().unwrap();
    let token = query.split("token=").nth(1).unwrap();

    let path = format!("/api/sessions/hub/terminal?token={token}&cols=80&rows=24");
    let (head, _) = handshake(port, &path, Some(&format!("http://127.0.0.1:{port}")));
    let (assets, _) = get(port, token, "/vendor/xterm.js");
    let (_, state) = get(port, token, "/api/state");
    board.kill().unwrap();
    board.wait().unwrap();

    assert_eq!(status_of(&head), 404, "{head}");
    assert_eq!(assets, 404);
    let state: serde_json::Value = serde_json::from_str(&state).unwrap();
    assert_eq!(state["boardTerminal"]["available"], false);
}

#[test]
fn the_resident_serves_the_terminal_library_with_its_license() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);

    let (status, js) = resident.get(&format!("/b/{SLUG}/vendor/xterm.js"));
    assert_eq!(status, 200);
    assert!(
        js.starts_with("/*! xterm.js - MIT License"),
        "{}",
        &js[..80]
    );
    assert!(js.contains("Permission is hereby granted"));
    assert!(js.contains("FitAddon") && js.contains("Unicode11Addon"));
    let (status, css) = resident.get(&format!("/b/{SLUG}/vendor/xterm.css"));
    assert_eq!(status, 200);
    assert!(css.contains(".xterm"));
    // The token applies to it like to everything else.
    let (status, _) = get(resident.port, "", &format!("/b/{SLUG}/vendor/xterm.js"));
    assert_eq!(status, 403);
}

/// An isolated tmux, a worker recorded in one of its windows and a resident server, with a
/// terminal opened on it and its first output read.
struct Open {
    ws: TcpStream,
    // Before the tmux, so that the server goes first (fields drop in order).
    _resident: Resident,
    _fixture: Fixture,
    tmux: IsolatedTmux,
    layout: Layout,
}

fn open_terminal(name: &str) -> Option<Open> {
    let tmux = IsolatedTmux::new(&unique(name))?;
    let layout = tmux.lay_out();
    let fixture = Fixture::new(&config(&tmux));
    forge_worker(&fixture, &layout);
    let resident = Resident::start(&fixture);
    let path = terminal_path(&resident, "worker-main", "cols=100&rows=30");
    let (head, mut ws) = handshake(resident.port, &path, Some(&own_origin(&resident)));
    assert_eq!(status_of(&head), 101, "{head}");
    read_until(&mut ws, &mut String::new(), "ADJ_MARK");
    assert_eq!(tmux.board_sessions().len(), 1);
    Some(Open {
        tmux,
        layout,
        ws,
        _resident: resident,
        _fixture: fixture,
    })
}

fn assert_target_alive(tmux: &IsolatedTmux, layout: &Layout) {
    let windows = tmux.out(&["list-windows", "-a", "-F", "#{window_id}"]);
    assert!(
        windows.lines().any(|w| w == layout.target_window),
        "{windows}"
    );
}

#[test]
fn a_page_that_vanishes_without_a_close_frame_leaves_nothing_behind() {
    let Some(open) = open_terminal("board-drop") else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    drop(open.ws);
    eventually("the board session to be gone", || {
        open.tmux.board_sessions().is_empty()
    });
    eventually("the board client to be gone", || {
        open.tmux.out(&["list-clients"]).is_empty()
    });
    assert_target_alive(&open.tmux, &open.layout);
    assert_eq!(
        open.tmux.current_window(&open.layout.session),
        open.layout.main_window
    );
}

#[test]
fn a_client_detached_from_tmux_closes_the_page_with_detached() {
    let Some(mut open) = open_terminal("board-detach") else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let made = open.tmux.board_sessions();
    open.tmux.out(&["detach-client", "-s", &made[0]]);
    let closed = read_close(&mut open.ws);
    assert_eq!(&closed[..2], &1000u16.to_be_bytes());
    assert_eq!(&closed[2..], b"detached");
    eventually("the board session to be gone", || {
        open.tmux.board_sessions().is_empty()
    });
    assert_target_alive(&open.tmux, &open.layout);
}

#[test]
fn a_closed_target_window_ends_the_terminal_instead_of_showing_another_agent() {
    let Some(mut open) = open_terminal("board-unlink") else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    // The agent exits and its window goes: the client must not move on to the next window.
    open.tmux
        .out(&["kill-window", "-t", &open.layout.target_window]);
    let closed = read_close(&mut open.ws);
    assert_eq!(&closed[..2], &1000u16.to_be_bytes());
    assert_eq!(&closed[2..], b"detached");
    eventually("the board session to be gone", || {
        open.tmux.board_sessions().is_empty()
    });
    let windows = open.tmux.out(&["list-windows", "-a", "-F", "#{window_id}"]);
    assert!(
        windows.lines().any(|w| w == open.layout.main_window),
        "{windows}"
    );
    assert!(
        !windows.lines().any(|w| w == open.layout.target_window),
        "{windows}"
    );
}

#[test]
fn switching_windows_inside_the_board_keeps_the_terminal_open() {
    let Some(mut open) = open_terminal("board-switch") else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let made = open.tmux.board_sessions();
    // The person moves to another window and back: only the target *disappearing* ends it.
    open.tmux.out(&[
        "select-window",
        "-t",
        &format!("={}:{}", made[0], open.layout.main_window),
    ]);
    open.tmux.out(&[
        "select-window",
        "-t",
        &format!("={}:{}", made[0], open.layout.target_window),
    ]);
    open.ws.write_all(&client_frame(0x2, b"still\r")).unwrap();
    read_until(&mut open.ws, &mut String::new(), "still");
    assert_eq!(open.tmux.board_sessions().len(), 1);
}
