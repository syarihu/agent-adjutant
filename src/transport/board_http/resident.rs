//! The resident server request handling.

use serde_json::json;
use std::io::{BufReader, Write};
use std::net::TcpStream;

use crate::infra::clock::{now_secs, utc_stamp};
use crate::infra::http::{self, Request};
use crate::infra::ws;

use super::assets::UI_HTML;
use super::auth::{page_rings, refuse};
use super::routes::{Route, decode_segment, no_such_route, route};
use crate::board::Resident;
use crate::board::review::RequestRefused;
use crate::board::view::{boards, others, work};
use crate::lifecycle::hub::TabOutcome;
use crate::lifecycle::review::AskError;
use crate::others::{Requested, SaveDraftsError, SyncError};

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

/// A WebSocket handshake: it may ask for the terminal of a session on a board this server serves,
/// or for the terminal of a PR's reader. Anything else is a 404, and so is that on a board a hub
/// or a dedicated `adj serve` serves, which never reach here.
pub(super) fn upgrade_resident(
    resident: &Resident,
    req: &Request,
    mut stream: TcpStream,
    reader: BufReader<TcpStream>,
) -> std::io::Result<()> {
    if req.path == "/api/others/terminal" {
        // Resolved from the record of the PR, so the page names a PR and nothing more.
        let Some(pr) = req.param("id").and_then(crate::others::pr_of) else {
            return http::json(
                &mut stream,
                400,
                &json!({ "error": "a pull request id is required" }).to_string(),
            );
        };
        return super::terminal::serve_reader(resident, &pr, req, stream, reader);
    }
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

pub(super) fn route_resident(
    resident: &Resident,
    req: &Request,
    out: &mut impl Write,
) -> std::io::Result<()> {
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
        ("GET", "/api/boards") => {
            // The request the page rings from: it says whether it may. A page that may not is not noted.
            if page_rings(req) {
                resident.waits.page_seen(now_secs());
            }
            http::json(
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
            )
        }
        ("GET", "/api/work") => match work_json(resident) {
            Some(json) => http::json(out, 200, &json),
            None => http::json(
                out,
                500,
                &json!({ "error": "could not build the work list" }).to_string(),
            ),
        },
        ("GET", "/api/others") => http::json(
            out,
            200,
            &serde_json::to_string(&others(&resident.root)).unwrap_or_default(),
        ),
        // Run on this connection's own thread, as the resident gives each one: a sync takes up
        // to its 30 seconds without holding any other request up.
        ("POST", "/api/others/sync") => match crate::others::sync(&resident.root) {
            Ok(synced) => http::json(
                out,
                200,
                &serde_json::to_string(&synced).unwrap_or_default(),
            ),
            Err(SyncError::Busy) => http::json(
                out,
                409,
                &json!({ "error": "a sync is already running", "busy": true }).to_string(),
            ),
            Err(SyncError::Failed(why)) => {
                http::json(out, 400, &json!({ "error": why }).to_string())
            }
        },
        ("POST", "/api/others/ai") => ask_for_read(resident, req, out),
        ("POST", "/api/others/hub/start") => start_review_hub(resident, req, out),
        ("GET", "/api/others/read") => stored_read(resident, req, out),
        ("GET", "/api/others/drafts") => stored_drafts(resident, req, out),
        ("POST", "/api/others/drafts") => save_drafts(resident, req, out),
        _ => no_such_route(out),
    }
}

/// `POST /api/others/ai`: queue a read-through of a PR, and start the readers next in line.
fn ask_for_read(resident: &Resident, req: &Request, out: &mut impl Write) -> std::io::Result<()> {
    let input: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
    let Some(id) = input.get("id").and_then(serde_json::Value::as_str) else {
        return http::json(
            out,
            400,
            &json!({ "error": "an id is required" }).to_string(),
        );
    };
    match crate::board::review::request(&resident.root, id) {
        Ok(asked) => {
            let (Requested::Queued(record) | Requested::Already(record)) = asked.requested;
            // `warning` is set when the queue could not be worked after the request was taken.
            http::json(
                out,
                200,
                &json!({ "record": record, "warning": asked.queue_error.as_deref().map(crate::transport::wording::queue_warning) }).to_string(),
            )
        }
        Err(RequestRefused::BadId) => http::json(
            out,
            400,
            &json!({ "error": format!("not a pull request: {id}") }).to_string(),
        ),
        Err(RequestRefused::Refused(why)) => {
            let (status, because) = match &why {
                AskError::HubStopped => (409, Some("hub-stopped")),
                AskError::NotStartable => (409, Some("not-startable")),
                AskError::NoRecord => (404, None),
                AskError::Done => (400, None),
                AskError::Failed(_) => (500, None),
            };
            http::json(
                out,
                status,
                &json!({ "error": crate::transport::wording::review_refusal(&why), "why": because })
                    .to_string(),
            )
        }
    }
}

/// `POST /api/others/hub/start`: open the code-review hub in a new window.
fn start_review_hub(
    resident: &Resident,
    req: &Request,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let input: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
    let how = match super::handlers::hub_start_of(&input) {
        Ok(how) => how,
        Err(e) => return http::json(out, 400, &json!({ "error": e }).to_string()),
    };
    match crate::board::review::start(&resident.root, how) {
        Ok(TabOutcome::Opened(done)) => http::json(
            out,
            200,
            &json!({ "started": true, "description": done.description }).to_string(),
        ),
        Ok(TabOutcome::AlreadyRunning(status)) => http::json(
            out,
            200,
            &json!({ "started": false, "running": true, "pid": status.pid }).to_string(),
        ),
        // The input was fine and the start itself failed: not the caller's mistake.
        Err(e) => http::json(out, 500, &json!({ "error": e }).to_string()),
    }
}

/// `GET /api/others/read?id=`: the read-through kept for a PR, as it was handed in.
fn stored_read(resident: &Resident, req: &Request, out: &mut impl Write) -> std::io::Result<()> {
    let Some(pr) = req.param("id").and_then(crate::others::pr_of) else {
        return http::json(
            out,
            400,
            &json!({ "error": "a pull request id is required" }).to_string(),
        );
    };
    match crate::others::read_through(&resident.root, &pr) {
        Ok(Some(read)) => http::json(out, 200, &read.to_string()),
        Ok(None) => http::json(out, 404, &json!({ "error": "no read-through" }).to_string()),
        Err(e) => http::json(out, 500, &json!({ "error": e }).to_string()),
    }
}

/// `GET /api/others/drafts?id=`: the drafts kept for a PR.
fn stored_drafts(resident: &Resident, req: &Request, out: &mut impl Write) -> std::io::Result<()> {
    let Some(pr) = req.param("id").and_then(crate::others::pr_of) else {
        return http::json(
            out,
            400,
            &json!({ "error": "a pull request id is required" }).to_string(),
        );
    };
    match crate::others::drafts(&resident.root, &pr) {
        Ok(Some(drafts)) => http::json(out, 200, &drafts.to_string()),
        Ok(None) => http::json(out, 404, &json!({ "error": "no drafts" }).to_string()),
        Err(e) => http::json(out, 500, &json!({ "error": e }).to_string()),
    }
}

/// `POST /api/others/drafts`: replace the drafts of a PR with `{"id": ..., "drafts": {...}}`.
/// Only the inner object is the input, so that what a GET answers (with its `id` and `savedAt`)
/// cannot be sent back as it is.
fn save_drafts(resident: &Resident, req: &Request, out: &mut impl Write) -> std::io::Result<()> {
    let body: serde_json::Value = match serde_json::from_slice(&req.body) {
        Ok(body) => body,
        Err(e) => {
            return http::json(
                out,
                400,
                &json!({ "error": format!("cannot read the request: {e}") }).to_string(),
            );
        }
    };
    let Some(pr) = body
        .get("id")
        .and_then(serde_json::Value::as_str)
        .and_then(crate::others::pr_of)
    else {
        return http::json(
            out,
            400,
            &json!({ "error": "a pull request id is required" }).to_string(),
        );
    };
    let Some(drafts) = body.get("drafts").filter(|d| d.is_object()) else {
        return http::json(
            out,
            400,
            &json!({ "error": "drafts must be an object" }).to_string(),
        );
    };
    match crate::others::save_drafts(&resident.root, &pr, drafts, &utc_stamp(now_secs())) {
        Ok(stored) => http::json(out, 200, &stored.to_string()),
        Err(SaveDraftsError::Invalid(problems)) => http::json(
            out,
            400,
            &json!({ "error": "the drafts are not valid", "problems": problems }).to_string(),
        ),
        Err(SaveDraftsError::NoRecord) => http::json(
            out,
            404,
            &json!({ "error": "no such pull request" }).to_string(),
        ),
        Err(SaveDraftsError::Failed(e)) => http::json(out, 500, &json!({ "error": e }).to_string()),
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
