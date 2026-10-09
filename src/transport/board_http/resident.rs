//! The resident server request handling.

use serde_json::json;
use std::io::{BufReader, Write};
use std::net::TcpStream;

use crate::infra::http::{self, Request};
use crate::infra::ws;

use super::assets::UI_HTML;
use super::auth::refuse;
use super::routes::{Route, decode_segment, no_such_route, route};
use crate::board::Resident;
use crate::board::view::{boards, work};

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
        let inner = Request {
            path: rest.to_string(),
            ..req.clone()
        };
        let Ok(Some(Route::Terminal(raw))) = Route::named(&inner) else {
            return None;
        };
        let id = decode_segment(raw);
        Some((resident.board(slug)?, id))
    });
    let Some((server, id)) = found else {
        return no_such_route(&mut stream);
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
    if matches!(Route::named(req), Ok(Some(Route::Page))) {
        return http::html(out, UI_HTML);
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/boards") => http::json(
            out,
            200,
            &serde_json::to_string(&{
                let mut boards = boards(&resident.root, resident.port, &resident.token);
                for board in &mut boards {
                    board.waits = resident.waits.notices(&board.slug);
                }
                boards
            })
            .unwrap_or_default(),
        ),
        ("GET", "/api/work") => match work_json(resident) {
            Some(json) => http::json(out, 200, &json),
            None => http::json(
                out,
                500,
                &json!({ "error": "could not build the work list" }).to_string(),
            ),
        },
        _ => no_such_route(out),
    }
}

/// How long a built `/api/work` document answers the next requests.
const WORK_CACHE: std::time::Duration = std::time::Duration::from_millis(1500);

/// The work document, from the resident's cache while it is fresh. Built outside the lock: it
/// reads every repository's board, and a request that finds it stale builds its own rather than
/// holding the others up.
fn work_json(resident: &Resident) -> Option<String> {
    if let Ok(cache) = resident.work_cache.lock()
        && let Some((built, json)) = cache.as_ref()
        && built.elapsed() < WORK_CACHE
    {
        return Some(json.clone());
    }
    // A document that cannot be written is an error, not an empty answer to keep.
    let json = serde_json::to_string(&work(resident)).ok()?;
    if let Ok(mut cache) = resident.work_cache.lock() {
        *cache = Some((std::time::Instant::now(), json.clone()));
    }
    Some(json)
}
