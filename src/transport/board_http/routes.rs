//! The request router and the routes' path parsing.

use std::io::{BufReader, Write};
use std::net::TcpStream;

use serde_json::json;

use crate::infra::http::{self, Request};

use super::assets::{UI_HTML, vendor_asset};
use super::auth::refuse;
use super::handlers::{
    act_on_hub, act_on_worktree, answer_gate, create_task, fetch_issue, focus_hub, nudge_hub,
    refresh_tasks, relay_findings, review_findings, start_parent_hub, update_task,
};
use super::sessions::{
    clean_up_session, git_of_session, link_session, open_session, restart_session, resume_session,
    start_session,
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

/// A route a board serves and what its path names. `Id` is a session's or a hub's id: the
/// segment as written (`&str`) until `decoded`, then the id it percent-decodes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Route<Id = String> {
    Page,
    State {
        sessions: bool,
        lines: bool,
    },
    /// A script or style the board terminal loads: its content type and body.
    Asset(&'static str, &'static str),
    // Task ids, worktree actions and gate ids are the raw segment, never decoded.
    TaskHistory(String),
    TaskFindings(String),
    RelayFindings(String),
    FetchIssue(String),
    UpdateTask(String),
    CreateTask,
    Refresh,
    StartSession,
    LinkSession(Id),
    SessionGit(Id),
    Session(Id, SessionAction),
    StartParentHub,
    Hub(Id, HubAction),
    NudgeHub,
    FocusHub,
    Worktree(String),
    AnswerGate(String),
    Terminal(Id),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionAction {
    Resume,
    Restart,
    Open,
    CleanUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HubAction {
    Start,
    Stop,
    Close,
    Reset,
    Restart,
}

impl<Id> Route<Id> {
    /// The routes a board a hub serves answers 404 for: reopening a session, opening a
    /// terminal and starting or stopping a hub reach outside the repository's own records, and
    /// a board a hub serves lives and dies with that hub. The terminal assets are only for the
    /// resident's terminal.
    pub(super) fn resident_only(&self) -> bool {
        matches!(
            self,
            Route::Session(..)
                | Route::StartParentHub
                | Route::Hub(..)
                | Route::Terminal(_)
                | Route::Asset(..)
        )
    }
}

impl Route {
    #[cfg(test)]
    pub(super) fn parse(req: &Request) -> Result<Option<Route>, String> {
        match Route::named(req)? {
            Some(route) => route.decoded().map(Some),
            None => Ok(None),
        }
    }
}

impl<'a> Route<&'a str> {
    /// The route a request names, its session or hub id still as written. Two steps, because a
    /// caller has to say which routes it serves between them: a bad encoding in the id of a
    /// route that is not served is a 404, not a 400. `Err` only for a path that ends in a task
    /// route's name without exactly one id: "no such task".
    pub(super) fn named(req: &'a Request) -> Result<Option<Self>, String> {
        let (method, path) = (req.method.as_str(), req.path.as_str());
        if let Some(rest) = path.strip_prefix("/api/tasks/") {
            return Self::task(method, rest, req.tail());
        }
        if let Some(rest) = path.strip_prefix("/api/sessions/") {
            return Ok(Self::session(method, rest));
        }
        if let Some(rest) = path.strip_prefix("/api/hubs/") {
            return Ok(Self::hub(method, rest));
        }
        Ok(Some(match (method, path) {
            // One document serves the page paths, and the page reads its own address to know
            // which view to draw.
            ("GET", "/" | "/index.html" | "/review") => Route::Page,
            ("GET", "/api/state") => Route::State {
                sessions: req.param("sessions") != Some("0"),
                lines: req.param("lines") == Some("1"),
            },
            ("GET", _) => match vendor_asset(path) {
                Some((kind, body)) => Route::Asset(kind, body),
                None => return Ok(None),
            },
            ("POST", "/api/tasks") => Route::CreateTask,
            ("POST", "/api/refresh") => Route::Refresh,
            ("POST", "/api/sessions") => Route::StartSession,
            ("POST", "/api/hubs") => Route::StartParentHub,
            ("POST", "/api/hub/next") => Route::NudgeHub,
            ("POST", "/api/hub/focus") => Route::FocusHub,
            ("POST", _) => match (
                path.strip_prefix("/api/worktrees/"),
                path.strip_prefix("/api/gates/"),
            ) {
                (Some(_), _) => Route::Worktree(req.tail().to_string()),
                (_, Some(_)) => Route::AnswerGate(req.tail().to_string()),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        }))
    }

    fn task(method: &str, rest: &str, last: &str) -> Result<Option<Self>, String> {
        let segments: Vec<&str> = rest.split('/').collect();
        let id = || match segments[..] {
            [id, _] if !id.is_empty() => Ok(id.to_string()),
            _ => Err("no such task".to_string()),
        };
        Ok(Some(match (method, last) {
            ("GET", "history") => Route::TaskHistory(id()?),
            ("GET", "findings") => Route::TaskFindings(id()?),
            ("POST", "relay") => Route::RelayFindings(id()?),
            ("POST", "issue") => Route::FetchIssue(id()?),
            ("POST", _) => Route::UpdateTask(last.to_string()),
            _ => return Ok(None),
        }))
    }

    /// `<id>/<action>` under `/api/sessions/`. The id is one segment, split on the raw path
    /// before decoding, so an encoded `/` is part of an id and a bare one is another path. The
    /// terminal takes any method, as the handshake never looked at it; whether the id names a
    /// session, and one that runs in tmux, is answered once the socket is open, where the page
    /// can be told why not.
    fn session(method: &str, rest: &'a str) -> Option<Self> {
        let (raw, action) = rest.split_once('/').filter(|(raw, _)| !raw.is_empty())?;
        Some(match (method, action) {
            ("GET", "git") => Route::SessionGit(raw),
            ("POST", "link") => Route::LinkSession(raw),
            ("POST", "resume") => Route::Session(raw, SessionAction::Resume),
            ("POST", "restart") => Route::Session(raw, SessionAction::Restart),
            ("POST", "open") => Route::Session(raw, SessionAction::Open),
            ("POST", "cleanup") => Route::Session(raw, SessionAction::CleanUp),
            (_, "terminal") => Route::Terminal(raw),
            _ => return None,
        })
    }

    /// `<id>/<action>` under `/api/hubs/`, split as `session` does: the page sends the id
    /// through `encodeURIComponent`, and a key may hold a `/`, a space or a letter that is not
    /// ASCII.
    fn hub(method: &str, rest: &'a str) -> Option<Self> {
        let (raw, action) = rest.split_once('/').filter(|(raw, _)| !raw.is_empty())?;
        let action = match (method, action) {
            ("POST", "start") => HubAction::Start,
            ("POST", "stop") => HubAction::Stop,
            ("POST", "close") => HubAction::Close,
            ("POST", "reset") => HubAction::Reset,
            ("POST", "restart") => HubAction::Restart,
            _ => return None,
        };
        Some(Route::Hub(raw, action))
    }

    /// The same route with its ids percent-decoded. An encoding that is not UTF-8 is an error
    /// for the caller to say, not a different route.
    pub(super) fn decoded(self) -> Result<Route, String> {
        Ok(match self {
            Route::LinkSession(raw) => Route::LinkSession(decode_segment(raw)?),
            Route::SessionGit(raw) => Route::SessionGit(decode_segment(raw)?),
            Route::Session(raw, action) => Route::Session(decode_segment(raw)?, action),
            Route::Hub(raw, action) => Route::Hub(decode_segment(raw)?, action),
            Route::Terminal(raw) => Route::Terminal(decode_segment(raw)?),
            Route::Page => Route::Page,
            Route::State { sessions, lines } => Route::State { sessions, lines },
            Route::Asset(kind, body) => Route::Asset(kind, body),
            Route::TaskHistory(id) => Route::TaskHistory(id),
            Route::TaskFindings(id) => Route::TaskFindings(id),
            Route::RelayFindings(id) => Route::RelayFindings(id),
            Route::FetchIssue(id) => Route::FetchIssue(id),
            Route::UpdateTask(id) => Route::UpdateTask(id),
            Route::CreateTask => Route::CreateTask,
            Route::Refresh => Route::Refresh,
            Route::StartSession => Route::StartSession,
            Route::StartParentHub => Route::StartParentHub,
            Route::NudgeHub => Route::NudgeHub,
            Route::FocusHub => Route::FocusHub,
            Route::Worktree(action) => Route::Worktree(action),
            Route::AnswerGate(id) => Route::AnswerGate(id),
        })
    }
}

pub(super) fn route(server: &Server, req: &Request, out: &mut impl Write) -> std::io::Result<()> {
    let named = match Route::named(req) {
        Ok(Some(route)) => route,
        Ok(None) => return no_such_route(out),
        Err(e) => return reply(out, Err::<(), _>(e)),
    };
    // A terminal is a WebSocket, answered by the resident before routing; here it is no route.
    if matches!(named, Route::Terminal(_)) || (named.resident_only() && !server.resident) {
        return no_such_route(out);
    }
    let route = match named.decoded() {
        Ok(route) => route,
        Err(e) => return reply(out, Err::<(), _>(e)),
    };
    match route {
        Route::Page => http::html(out, UI_HTML),
        Route::State { sessions, lines } => {
            let document = state(server, sessions, lines);
            match serde_json::to_string(&document) {
                Ok(body) => http::json(out, 200, &body),
                Err(e) => http::json(out, 500, &json!({ "error": e.to_string() }).to_string()),
            }
        }
        Route::Asset(kind, body) => http::respond(out, 200, kind, body.as_bytes()),
        Route::TaskHistory(id) => reply(out, Ok::<_, String>(task_history(server, &id))),
        Route::TaskFindings(id) => reply(out, review_findings(server, &id)),
        Route::RelayFindings(id) => reply(out, relay_findings(server, &id, &req.body)),
        Route::FetchIssue(id) => reply(out, fetch_issue(server, &id)),
        Route::UpdateTask(id) => reply(out, update_task(server, &id, &req.body)),
        Route::CreateTask => reply(out, create_task(server, &req.body)),
        Route::Refresh => reply(out, refresh_tasks(server)),
        Route::StartSession => reply(out, start_session(server, &req.body)),
        Route::LinkSession(id) => reply(out, link_session(server, &id, &req.body)),
        Route::SessionGit(id) => reply(out, git_of_session(server, &id)),
        Route::Session(id, action) => reply(
            out,
            match action {
                SessionAction::Resume => resume_session(server, &id, &req.body),
                SessionAction::Restart => restart_session(server, &id, &req.body),
                SessionAction::Open => open_session(server, &id),
                SessionAction::CleanUp => clean_up_session(server, &id, &req.body),
            },
        ),
        Route::StartParentHub => reply(out, start_parent_hub(server, &req.body)),
        Route::Hub(id, action) => reply(out, act_on_hub(server, &id, action, &req.body)),
        Route::NudgeHub => reply(out, nudge_hub(server)),
        Route::FocusHub => reply(out, focus_hub(server)),
        Route::Worktree(action) => reply(out, act_on_worktree(server, &action, &req.body)),
        Route::AnswerGate(id) => reply(out, answer_gate(server, &id, &req.body)),
        Route::Terminal(_) => no_such_route(out),
    }
}

pub(super) fn no_such_route(out: &mut impl Write) -> std::io::Result<()> {
    http::json(out, 404, &json!({ "error": "no such route" }).to_string())
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

/// `%XX` escapes in one path segment, and nothing else: unlike a query string, a `+` here is a
/// plus.
pub(super) fn decode_segment(raw: &str) -> Result<String, String> {
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
