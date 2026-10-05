//! The resident server request handling.

use serde_json::json;
use std::io::{BufReader, Write};
use std::net::TcpStream;

use crate::infra::http::{self, Request};
use crate::infra::ws;

use super::assets::UI_HTML;
use super::auth::refuse;
use super::routes::{is_page_path, route, terminal_route};
use crate::board::Resident;
use crate::board::view::boards;

/// `/b/<slug>/rest` as its slug and the path the board itself sees. A slug is what
/// `identity::slug_for` makes — lowercase letters, digits and `-` — and anything else is not a
/// board, so nothing that reaches the address book or the file system is ever a stranger's
/// string.
pub(super) fn split_board_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/b/")?;
    let (slug, tail) = match rest.find('/') {
        Some(at) => rest.split_at(at),
        None => (rest, "/"),
    };
    (!slug.is_empty()
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
    .then_some((slug, tail))
}

pub fn handle_resident(resident: &Resident, mut stream: TcpStream) -> std::io::Result<()> {
    // A connection that opens and says nothing must not hold a thread for ever: this process
    // is meant to run for days, and a browser opens speculative connections all the time.
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let Some(req) = http::read_request(&mut reader)? else {
        return Ok(());
    };
    if let Some((status, why)) = refuse(&resident.token, resident.port, &req) {
        return http::json(&mut stream, status, &json!({ "error": why }).to_string());
    }
    if ws::is_upgrade(&req.headers) {
        return upgrade_resident(resident, &req, stream, reader);
    }
    route_resident(resident, &req, &mut stream)
}

/// A WebSocket handshake: there is exactly one thing it may ask for, the terminal of a session
/// on a board this server serves. Anything else is a 404, and so is that on a board a hub or a
/// dedicated `adj serve` serves, which never reach here.
fn upgrade_resident(
    resident: &Resident,
    req: &Request,
    mut stream: TcpStream,
    reader: BufReader<TcpStream>,
) -> std::io::Result<()> {
    let found = split_board_path(&req.path).and_then(|(slug, rest)| {
        let id = terminal_route(rest)?;
        Some((resident.board(slug)?, id))
    });
    let Some((server, id)) = found else {
        return http::json(
            &mut stream,
            404,
            &json!({ "error": "no such route" }).to_string(),
        );
    };
    match id {
        Ok(id) => super::terminal::serve(&server, &id, req, stream, reader),
        Err(e) => http::json(&mut stream, 400, &json!({ "error": e }).to_string()),
    }
}

fn route_resident(resident: &Resident, req: &Request, out: &mut impl Write) -> std::io::Result<()> {
    if let Some((slug, rest)) = split_board_path(&req.path) {
        let Some(server) = resident.board(slug) else {
            return http::json(out, 404, &json!({ "error": "no such board" }).to_string());
        };
        let inner = Request {
            path: rest.to_string(),
            ..req.clone()
        };
        return route(&server, &inner, out);
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", path) if is_page_path(path) => http::html(out, UI_HTML),
        ("GET", "/api/boards") => http::json(
            out,
            200,
            &serde_json::to_string(&boards(&resident.root, resident.port, &resident.token))
                .unwrap_or_default(),
        ),
        _ => http::json(out, 404, &json!({ "error": "no such route" }).to_string()),
    }
}
