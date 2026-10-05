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

//! `adj serve` — the board, served to a browser on this machine.

use std::net::TcpListener;

mod assets;
mod auth;
mod daemon;
mod handlers;
mod index;
mod resident;
mod routes;
mod sessions;
mod state;

use crate::board::Board;
pub(super) use crate::board::Server;
pub(super) use crate::board::settings_now;
pub use crate::board::{DEFAULT_PORT, HubBoard, located, serve_for_hub};
use daemon::open_browser;
pub use daemon::{server_restart, server_start, server_status, server_stop};
pub(super) use handlers::hub_start_of;
pub(crate) use routes::handle as board_connection;
pub(super) use sessions::{board_session, find_session, git_state_of};

pub fn serve(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    port: u16,
    open: bool,
) -> Result<(), String> {
    let ctx = crate::registry::context(repo_arg, hub_arg)?;
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
        format!(
            "cannot listen on 127.0.0.1:{port}: {e}\n\
             (a dashboard may already be running — try opening http://127.0.0.1:{port}/)"
        )
    })?;
    let board = Board::bind(ctx, listener)?;
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
    board.run(routes::handle);
    Ok(())
}

#[cfg(test)]
mod tests;
