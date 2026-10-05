//! `adj serve` — the board, served to a browser on this machine.

use std::net::TcpListener;

mod assets;
mod auth;
mod daemon;
mod handlers;
mod resident;
mod routes;

use crate::board::Board;
pub(super) use crate::board::Server;
pub(super) use crate::board::settings_now;
pub(super) use crate::board::view::{board_session, find_session, git_state_of};
pub use crate::board::{DEFAULT_PORT, HubBoard, located, serve_for_hub};
use daemon::open_browser;
pub use daemon::{server_restart, server_start, server_status, server_stop};
pub(super) use handlers::hub_start_of;
pub(crate) use routes::handle as board_connection;

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
