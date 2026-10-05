//! The security boundary of the board.

use crate::infra::http::{self, Request};
use crate::infra::ws;

// ── the security boundary ────────────────────────────────────────────

/// Why a request is being refused, or `None` to let it through.
///
/// Pure, and separated from the routing for that reason: this is the whole of what stands
/// between a page on the internet and an endpoint that approves a diff and opens a pull
/// request, so it is the part that gets tested exhaustively.
pub(super) fn refuse(token: &str, port: u16, req: &Request) -> Option<(u16, &'static str)> {
    if req.body.is_empty()
        && req
            .header("content-length")
            .and_then(|v| v.parse::<usize>().ok())
            .is_some_and(|len| len > http::MAX_BODY)
    {
        return Some((413, "body too large"));
    }

    // The token may travel in the URL (that is how the page is first opened) or in a
    // header (how the page's own calls send it, so it stays out of logs and referrers).
    let given = req
        .header("x-adjutant-token")
        .or_else(|| req.param("token"));
    if !given.is_some_and(|t| http::secret_eq(t, token)) {
        return Some((403, "bad or missing token"));
    }

    // A WebSocket handshake is a GET, so nothing above stops a page on another site from
    // opening one, and what it opens here is a terminal. A browser always sends `Origin` on
    // a handshake and a page cannot forge it, so it has to be ours and it has to be there.
    if ws::is_upgrade(&req.headers) {
        match req.header("origin") {
            Some(origin) if is_own_origin(origin, port) => {}
            Some(_) => return Some((403, "cross-origin request")),
            None => return Some((403, "no Origin header")),
        }
    }

    // A form on another site can POST here without reading the answer, and that is enough
    // to approve something. It cannot set a custom header cross-origin without a preflight
    // this server never grants, and it cannot forge `Origin` — so anything that changes
    // state has to prove both.
    if req.method != "GET" {
        if req.header("x-adjutant-token").is_none() {
            return Some((
                403,
                "state-changing requests must send the token as a header",
            ));
        }
        match req.header("origin") {
            Some(origin) if is_own_origin(origin, port) => {}
            Some(_) => return Some((403, "cross-origin request")),
            None => return Some((403, "no Origin header")),
        }
    }
    None
}

pub(super) fn is_own_origin(origin: &str, port: u16) -> bool {
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|host| origin == format!("http://{host}:{port}"))
}
