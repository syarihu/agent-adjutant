//! `adj serve` — the board, served to a browser on this machine.

use std::net::TcpListener;

use super::server::open_browser;
use crate::board::Board;
use crate::transport::board_http::handle;

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
    board.run(handle);
    Ok(())
}
