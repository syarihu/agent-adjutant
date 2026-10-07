use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use super::{DEFAULT_PORT, Server, bind_preferring, board_url, jobs, resident_url, token};
use crate::registry::{dashboards_running, record};

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
pub fn serve_for_hub(
    ctx: crate::registry::Context,
    handle: fn(&Server, TcpStream) -> std::io::Result<()>,
) -> Result<HubBoard, String> {
    if let Some(url) = resident_url(&ctx.state, &ctx.repo) {
        return Ok(HubBoard::Resident(url));
    }
    if dashboards_running(&ctx.state, &ctx.repo.slug).is_some() {
        return Ok(HubBoard::AlreadyRunning);
    }
    let listener =
        bind_preferring(DEFAULT_PORT).map_err(|e| format!("cannot listen on 127.0.0.1: {e}"))?;
    let board = Board::bind(ctx, listener)?;
    let url = board.url();
    std::thread::spawn(move || board.run(handle));
    Ok(HubBoard::Serving(url))
}

/// A board with its socket bound and its record written, not yet answering.
pub struct Board {
    pub server: Arc<Server>,
    listener: TcpListener,
    pub recorded: bool,
}

impl Board {
    pub fn bind(ctx: crate::registry::Context, listener: TcpListener) -> Result<Board, String> {
        let token = token(&ctx.state)?;
        // Asked back rather than taken from the caller: port 0 is how a board gets a free
        // port, and the number it got is the only way to reach it.
        let port = listener
            .local_addr()
            .map(|a| a.port())
            .map_err(|e| format!("cannot read the board's port: {e}"))?;
        let recorded = record(&ctx.state, &ctx.repo.slug, port)?;
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
                waits: Arc::default(),
            }),
            listener,
            recorded,
        })
    }

    pub fn url(&self) -> String {
        board_url(self.server.port, &self.server.token)
    }

    pub fn run(self, handle: fn(&Server, TcpStream) -> std::io::Result<()>) {
        // The board's one clock: it ends with the process, as the accept loop below does.
        let ctx = self.server.ctx.clone();
        std::thread::spawn(move || jobs::sweep_gates::run(move || vec![ctx.clone()]));
        let server = Arc::clone(&self.server);
        std::thread::spawn(move || {
            let waits = Arc::clone(&server.waits);
            waits.run(&server.ctx.state.clone(), move || {
                (vec![Arc::clone(&server)], true)
            });
        });
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
