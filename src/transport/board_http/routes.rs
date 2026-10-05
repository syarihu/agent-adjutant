//! The request router and the routes' path parsing.

use std::io::{BufReader, Write};
use std::net::TcpStream;

use serde_json::json;

use crate::board::hub::start_parent_hub;
use crate::board::session::start_request;
use crate::infra::http::{self, Request};

use super::assets::{UI_HTML, vendor_asset};
use super::auth::refuse;
use super::handlers::{
    act_on_hub, act_on_worktree, answer_gate, create_task, fetch_issue, focus_hub, nudge_hub,
    refresh_tasks, relay_findings, review_findings, update_task,
};
use super::sessions::{
    clean_up_session, git_of_session, open_session, restart_session, resume_session,
};
use crate::board::Server;
use crate::board::view::{state, task_history};

// ── routing ──────────────────────────────────────────────────────────

pub fn handle(server: &Server, mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let Some(req) = http::read_request(&mut reader)? else {
        return Ok(());
    };
    if let Some((status, why)) = refuse(&server.token, server.port, &req) {
        return http::json(&mut stream, status, &json!({ "error": why }).to_string());
    }
    route(server, &req, &mut stream)
}

/// The paths that are the page itself. One document serves them all, and the page reads its
/// own address to know which view to draw.
pub(super) fn is_page_path(path: &str) -> bool {
    matches!(path, "/" | "/index.html" | "/review")
}

pub(super) fn route(server: &Server, req: &Request, out: &mut impl Write) -> std::io::Result<()> {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", path) if is_page_path(path) => http::html(out, UI_HTML),
        ("GET", "/api/state") => {
            let document = state(
                server,
                req.param("sessions") != Some("0"),
                req.param("lines") == Some("1"),
            );
            match serde_json::to_string(&document) {
                Ok(body) => http::json(out, 200, &body),
                Err(e) => http::json(out, 500, &json!({ "error": e.to_string() }).to_string()),
            }
        }
        ("GET", path) if vendor_asset(path, server.resident).is_some() => {
            let (kind, body) = vendor_asset(path, server.resident).unwrap_or_default();
            http::respond(out, 200, kind, body.as_bytes())
        }
        ("GET", path) if path.starts_with("/api/tasks/") && path.ends_with("/history") => {
            reply(out, task_history(server, path))
        }
        ("GET", path) if path.starts_with("/api/tasks/") && path.ends_with("/findings") => {
            reply(out, review_findings(server, path))
        }
        ("GET", path) if session_route_for(path, "git").is_some() => {
            let result = session_route_for(path, "git")
                .unwrap_or_else(|| Err("no such route".to_string()))
                .and_then(|id| git_of_session(server, &id));
            reply(out, result)
        }
        ("POST", "/api/tasks") => reply(out, create_task(server, &req.body)),
        ("POST", path) if path.starts_with("/api/tasks/") && path.ends_with("/relay") => {
            reply(out, relay_findings(server, path, &req.body))
        }
        ("POST", path) if path.starts_with("/api/tasks/") && path.ends_with("/issue") => {
            reply(out, fetch_issue(server, path))
        }
        ("POST", path) if path.starts_with("/api/tasks/") => {
            reply(out, update_task(server, req.tail(), &req.body))
        }
        ("POST", "/api/refresh") => reply(out, refresh_tasks(server)),
        ("POST", "/api/sessions") => reply(out, start_request(server, &req.body)),
        ("POST", path) if session_route_for(path, "link").is_some() => {
            let result = session_route_for(path, "link")
                .unwrap_or_else(|| Err("no such route".to_string()))
                .and_then(|id| crate::board::session::link(server, &id, &req.body));
            reply(out, result)
        }
        // Only on the resident's boards, like the hub actions below: reopening a session,
        // opening a terminal and removing a worktree reach outside the repository's own
        // records, and a board a hub serves lives and dies with that hub.
        ("POST", path)
            if server.resident
                && session_route(path).is_some_and(|(_, action)| {
                    matches!(action, "resume" | "restart" | "open" | "cleanup")
                }) =>
        {
            let (id, action) = session_route(path).unwrap_or((Err("no such route".into()), ""));
            let result = id.and_then(|id| match action {
                "resume" => resume_session(server, &id, &req.body),
                "restart" => restart_session(server, &id, &req.body),
                "open" => open_session(server, &id),
                _ => clean_up_session(server, &id, &req.body),
            });
            reply(out, result)
        }
        ("POST", "/api/hubs") if server.resident => reply(out, start_parent_hub(server, &req.body)),
        // Only on the resident's boards: starting and stopping a hub reaches outside the
        // repository's own records, and a board a hub serves lives and dies with that hub.
        ("POST", path) if server.resident && hub_route(path).is_some() => {
            reply(out, act_on_hub(server, path, &req.body))
        }
        ("POST", "/api/hub/next") => reply(out, nudge_hub(server)),
        ("POST", "/api/hub/focus") => reply(out, focus_hub(server)),
        ("POST", path) if path.starts_with("/api/worktrees/") => {
            reply(out, act_on_worktree(server, req.tail(), &req.body))
        }
        ("POST", path) if path.starts_with("/api/gates/") => {
            reply(out, answer_gate(server, req.tail(), &req.body))
        }
        _ => http::json(out, 404, &json!({ "error": "no such route" }).to_string()),
    }
}

/// A command's answer, as the page sees it. An error is a 400 with the message in it rather
/// than a 500 with nothing: every failure reachable from here is something the person can
/// act on, and the page shows the text.
fn reply<T: serde::Serialize>(
    out: &mut impl Write,
    result: Result<T, String>,
) -> std::io::Result<()> {
    match result.and_then(|value| serde_json::to_string(&value).map_err(|e| e.to_string())) {
        Ok(body) => http::json(out, 200, &body),
        Err(e) => http::json(out, 400, &json!({ "error": e }).to_string()),
    }
}

/// `/api/hubs/<id>/<action>` as its id and action, for the five actions there are (start, stop,
/// close, reset and restart). The id is one path segment, percent-decoded — the page sends it through
/// `encodeURIComponent`, and a key may hold a `/`, a space or a letter that is not ASCII. The
/// raw segment is checked for a `/` first, so an encoded one names an id and a bare one is
/// another route. An encoding that is not UTF-8 is an error for the caller to say, not a
/// different route.
pub(super) fn hub_route(path: &str) -> Option<(Result<String, String>, &str)> {
    let (raw, action) = path.strip_prefix("/api/hubs/")?.split_once('/')?;
    (!raw.is_empty()
        && !raw.contains('/')
        && matches!(action, "start" | "stop" | "close" | "reset" | "restart"))
    .then(|| (decode_segment(raw), action))
}

/// `/api/sessions/<id>/terminal` as the session id, percent-decoded as `hub_route` does. Only
/// the path is looked at: whether the id names a session, and one that runs in tmux, is
/// answered once the socket is open, where the page can be told why not.
pub(super) fn terminal_route(path: &str) -> Option<Result<String, String>> {
    let raw = path
        .strip_prefix("/api/sessions/")?
        .strip_suffix("/terminal")?;
    (!raw.is_empty() && !raw.contains('/')).then(|| decode_segment(raw))
}

/// `/api/sessions/<id>/<action>` as the session id, percent-decoded as `terminal_route` does,
/// and the action, for the six there are besides the terminal (which is a WebSocket and
/// answered before routing). Which of them a board serves is for `route` to say.
pub(super) fn session_route(path: &str) -> Option<(Result<String, String>, &str)> {
    let (raw, action) = path.strip_prefix("/api/sessions/")?.split_once('/')?;
    (!raw.is_empty()
        && !raw.contains('/')
        && matches!(
            action,
            "link" | "git" | "resume" | "restart" | "open" | "cleanup"
        ))
    .then(|| (decode_segment(raw), action))
}

/// The session id in `path` when it is the route of `action` and no other.
pub(super) fn session_route_for(path: &str, action: &str) -> Option<Result<String, String>> {
    session_route(path)
        .filter(|(_, found)| *found == action)
        .map(|(id, _)| id)
}

/// `%XX` escapes in one path segment, and nothing else: unlike a query string, a `+` here is a
/// plus.
fn decode_segment(raw: &str) -> Result<String, String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let byte = bytes
            .get(i + 1..i + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .filter(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            .ok_or_else(|| format!("bad percent-encoding in the id: {raw}"))?;
        out.push(byte);
        i += 3;
    }
    String::from_utf8(out).map_err(|_| format!("the id is not valid UTF-8: {raw}"))
}

/// The task id in `/api/tasks/{id}/{what}`, when there is exactly one.
pub(super) fn task_id_in<'a>(path: &'a str, what: &str) -> Option<&'a str> {
    path.strip_prefix("/api/tasks/")
        .and_then(|rest| rest.strip_suffix(&format!("/{what}")))
        .filter(|id| !id.is_empty() && !id.contains('/'))
}
