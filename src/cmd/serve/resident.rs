//! The resident server request handling.

use std::io::{BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::infra::http::{self, Request};
use crate::infra::ws;

use super::Server;
use super::assets::UI_HTML;
use super::auth::refuse;
use super::index::boards_json;
use super::routes::{is_page_path, route, terminal_route};
use crate::registry::{Address, address_of};

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

/// The resident server: its token and port, and the boards it has opened. A board is opened
/// on the first request for it and kept, for the same reason a dedicated one keeps its
/// `JulesWatch`: what it remembers between polls is a cache, and the records stay the answer.
pub(super) struct Resident {
    /// The state directory this resident and every board it opens read.
    pub(super) root: std::path::PathBuf,
    pub(super) token: String,
    pub(super) port: u16,
    /// Each open board with the address it was built from, so that one whose address has
    /// changed since is not served from the old context.
    pub(super) boards: Mutex<std::collections::HashMap<String, (Address, Arc<Server>)>>,
    /// The tmux version, asked once at start: the board terminal is offered only with one.
    pub(super) tmux: Option<(u32, u32)>,
    /// Open board terminals, over all boards.
    pub(super) terminals: Arc<AtomicUsize>,
    /// The poll that keeps the cards' pull requests up to date, over all boards.
    pub(super) pr_poll: Arc<crate::cmd::PrPoll>,
}

impl Resident {
    /// The board for `slug`, or `None` when the address book has no such board any more.
    ///
    /// The address is read on every call — a small file — and the open board is used only
    /// while it is the one the file still names: a checkout that moved, or was replaced under
    /// the same slug, is served from where it is now.
    pub(super) fn board(&self, slug: &str) -> Option<Arc<Server>> {
        // Bounded: a file rewritten again and again while this builds is not worth chasing.
        for _ in 0..3 {
            let Some(address) = address_of(&self.root, slug) else {
                self.boards.lock().ok()?.remove(slug);
                return None;
            };
            if let Some((built_from, open)) = self.boards.lock().ok()?.get(slug)
                && *built_from == address
            {
                return Some(Arc::clone(open));
            }
            // Outside the lock: resolving the checkout asks git, and every other board waits
            // on this map.
            //
            // Never `set_current_dir`: this process is threaded, and the checkout is named to
            // each call instead.
            let repo = crate::kernel::identity::resolve_in(
                Some(Path::new(&address.main)),
                Some(&address.nwo),
                address.hub.as_deref(),
            )
            .ok()
            .filter(|repo| repo.slug == slug)?;
            let ctx = crate::registry::context_at(repo, self.root.clone()).ok()?;
            let server = Arc::new(Server {
                ctx,
                token: self.token.clone(),
                port: self.port,
                resident: true,
                jules: Arc::default(),
                hub_titles: Arc::default(),
                last_lines: Arc::default(),
                tmux: self.tmux,
                terminals: Arc::clone(&self.terminals),
                pr_poll: Some(Arc::clone(&self.pr_poll)),
            });
            let mut boards = self.boards.lock().ok()?;
            // Asked again under the lock: what was built is only put in place while it is still
            // what the file says, so a slower build of an older address cannot replace a newer
            // one — and one that lost the race is thrown away and built again.
            if address_of(&self.root, slug).as_ref() != Some(&address) {
                continue;
            }
            match boards.get(slug) {
                Some((built_from, open)) if *built_from == address => {
                    return Some(Arc::clone(open));
                }
                _ => {
                    boards.insert(slug.to_string(), (address, Arc::clone(&server)));
                    return Some(server);
                }
            }
        }
        None
    }
}

pub(super) fn handle_resident(resident: &Resident, mut stream: TcpStream) -> std::io::Result<()> {
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
        Ok(id) => crate::cmd::board_terminal::serve(&server, &id, req, stream, reader),
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
            &Value::Array(boards_json(&resident.root, resident.port, &resident.token)).to_string(),
        ),
        _ => http::json(out, 404, &json!({ "error": "no such route" }).to_string()),
    }
}
