//! `adj serve` — the board, served to a browser on this machine.
//!
//! The dashboard is not a second coordination system. Every button on it ends in something
//! this binary could already do: a task handed over becomes a `request` in the hub's inbox
//! and a poke on its tab, exactly as `adj send` would. What the server adds is a view of
//! state that until now could only be read one `adj` invocation at a time, and a place to
//! put the questions a worker used to have to ask into a tab nobody was watching.
//!
//! It holds one clock, and only in the resident server: the poll that keeps the cards' pull
//! requests up to date (`pr_poll`). A board served by itself, or by a hub, has none: there a
//! request arrives because a person clicked, and that is the only thing that moves. `/api/state`
//! never asks GitHub on either, since the page polls it every couple of seconds.

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

mod assets;
mod auth;
mod daemon;
mod handlers;
mod index;
mod registry;
mod resident;
mod routes;
mod sessions;
mod state;

use auth::token;
use daemon::open_browser;
pub use daemon::{resident_running, server_restart, server_start, server_status, server_stop};
pub(super) use handlers::hub_start_of;
pub(super) use registry::forget_board;
pub use registry::{board_json, dashboards_running, note_board, running};
use registry::{board_url, record, resident_url};
use routes::handle;
pub(super) use sessions::{board_session, find_session, git_state_of};
use state::LastLines;
pub(super) use state::settings_now;

pub const DEFAULT_PORT: u16 = 4577;

/// Everything a connection needs. Shared across threads, read-only after startup — the
/// state that changes lives on disk, where the hub and its workers can also reach it.
pub(super) struct Server {
    pub(super) ctx: super::Context,
    token: String,
    port: u16,
    /// Whether the resident server is the one answering, which serves this board at a path
    /// of its own. The page reads it from the state.
    pub(super) resident: bool,
    /// What Jules last said about each session a card follows. The one thing here that
    /// changes after startup, and it is a cache: the record on disk stays the answer.
    jules: Arc<super::JulesWatch>,
    /// The titles of the parent tasks the hubs are named after: a cache of the tracker's, kept
    /// on disk, and read from a thread of its own.
    hub_titles: Arc<super::HubTitles>,
    /// The last line each session's pane showed, for the pages that ask for it (`?lines=1`).
    /// A cache: the pane is the answer.
    last_lines: Arc<LastLines>,
    /// What tmux this machine has, when the board may open terminals on it: only the resident
    /// server serves one, and only where `tmux -V` answered when it started.
    pub(super) tmux: Option<(u32, u32)>,
    /// How many board terminals are open across every board, which is what is capped.
    pub(super) terminals: Arc<AtomicUsize>,
    /// The resident server's PR poll, whose health the page shows. `None` on a board that is
    /// served by itself: nothing polls there, and the page says nothing about it.
    pr_poll: Option<Arc<super::PrPoll>>,
}

pub fn serve(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    port: u16,
    open: bool,
) -> Result<(), String> {
    let ctx = super::context(repo_arg, hub_arg)?;
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
        format!(
            "cannot listen on 127.0.0.1:{port}: {e}\n\
             (a dashboard may already be running — try opening http://127.0.0.1:{port}/)"
        )
    })?;
    let board = Board::new(ctx, listener)?;
    let url = board.url();
    println!("adj serve: {} — {url}", board.server.ctx.repo.nwo);
    println!("The token is in the URL. Anything without it gets a 403.");
    if !board.recorded {
        eprintln!(
            "adj serve: another board is already serving this hub; it stays the one `adj gate open` and `adj config` point at."
        );
    }
    if open {
        open_browser(&url);
    }
    board.run();
    Ok(())
}

/// What serving the board of a hub from inside its MCP server came to.
pub enum HubBoard {
    /// This process is serving the board, at this URL.
    Serving(String),
    /// The resident server serves it, at this URL. Nothing was bound here.
    Resident(String),
    /// A board for the hub is already running: one somebody started by hand with `adj serve`
    /// is left to go on serving, rather than joined by a second.
    AlreadyRunning,
}

/// Serve the board of the hub `ctx` addresses from inside that hub's MCP server, and hand
/// back where it is. A live resident server (see `server_start`) serves every board already,
/// so it is asked first: the hub only tells it where the repository is and binds nothing.
///
/// The socket and the record are both in place before this returns, so a tool call that
/// asks for the URL straight after finds it; only the accept loop goes to a thread, and it
/// ends with the process — which is the point: the MCP server lives exactly as long as the
/// hub's session does.
///
/// Nothing here writes to stdout. In that process stdout carries JSON-RPC, and a stray line
/// on it breaks the protocol for the whole session.
pub fn serve_for_hub(ctx: super::Context) -> Result<HubBoard, String> {
    if let Some(url) = resident_url(&ctx.repo) {
        return Ok(HubBoard::Resident(url));
    }
    if dashboards_running(&ctx.repo.slug).is_some() {
        return Ok(HubBoard::AlreadyRunning);
    }
    let listener =
        bind_preferring(DEFAULT_PORT).map_err(|e| format!("cannot listen on 127.0.0.1: {e}"))?;
    let board = Board::new(ctx, listener)?;
    let url = board.url();
    std::thread::spawn(move || board.run());
    Ok(HubBoard::Serving(url))
}

/// `port` on the loopback address, or any free port when `port` is taken — a second hub of
/// the same repository, a hub of another one, or something that is not ours at all. Only
/// "in use" falls back: any other failure would fail on a free port too.
fn bind_preferring(port: u16) -> std::io::Result<TcpListener> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => TcpListener::bind(("127.0.0.1", 0)),
        bound => bound,
    }
}

/// A board with its socket bound and its record written, not yet answering.
struct Board {
    server: Arc<Server>,
    listener: TcpListener,
    recorded: bool,
}

impl Board {
    fn new(ctx: super::Context, listener: TcpListener) -> Result<Board, String> {
        let token = token()?;
        // Asked back rather than taken from the caller: port 0 is how a board gets a free
        // port, and the number it got is the only way to reach it.
        let port = listener
            .local_addr()
            .map(|a| a.port())
            .map_err(|e| format!("cannot read the board's port: {e}"))?;
        let recorded = record(&ctx.repo.slug, port)?;
        Ok(Board {
            server: Arc::new(Server {
                ctx,
                token,
                port,
                resident: false,
                jules: Arc::default(),
                hub_titles: Arc::default(),
                last_lines: Arc::default(),
                tmux: None,
                terminals: Arc::default(),
                pr_poll: None,
            }),
            listener,
            recorded,
        })
    }

    fn url(&self) -> String {
        board_url(self.server.port, &self.server.token)
    }

    fn run(self) {
        for stream in self.listener.incoming() {
            match stream {
                Ok(stream) => {
                    let server = Arc::clone(&self.server);
                    // A thread per connection, because this one is long-lived and a browser
                    // holds several at once: an accept loop that serves them one at a time
                    // deadlocks the moment a second tab is opened.
                    std::thread::spawn(move || {
                        if let Err(e) = handle(&server, stream) {
                            eprintln!("adj serve: connection error: {e}");
                        }
                    });
                }
                Err(e) => eprintln!("adj serve: accept error: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests;
