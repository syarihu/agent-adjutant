//! The security boundary of the board.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

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

/// The shared secret, made once and kept.
///
/// Stored beside the rest of the state rather than handed out on each start: the URL is
/// meant to be a bookmark, and a token that changed every run would break it daily.
pub(super) fn token(root: &Path) -> Result<String, String> {
    if let Some(existing) = stored_token(root) {
        return Ok(existing);
    }
    let path = token_path(root);
    let token = random_hex();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    // Written in full to a file of our own first, then put in place with a link, which
    // fails if the file is already there: hubs of two repositories can start their boards at
    // the same moment on a fresh machine, and neither may ever see the other's token half
    // written. The one whose token was replaced would go on checking a secret that no URL
    // handed out any more carries. The loser reads the winner's.
    let staged = path.with_file_name(format!("dashboard-token.{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Readable by its owner alone: every other user on the machine can otherwise read the
    // file and post to the endpoints.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(&staged)
        .and_then(|mut file| file.write_all(format!("{token}\n").as_bytes()))
        .map_err(|e| format!("cannot write {}: {e}", staged.display()))?;
    let placed = match std::fs::hard_link(&staged, &path) {
        Ok(()) => Ok(token),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => match stored_token(root) {
            Some(theirs) => Ok(theirs),
            // An empty file is no token at all, and is replaced as it always was — by a
            // rename, so the replacement is whole and owner-only too. Read back afterwards,
            // since another start may be replacing it at the same time and the last one in
            // is the one every URL will carry.
            None => std::fs::rename(&staged, &path)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))
                .map(|()| stored_token(root).unwrap_or(token)),
        },
        Err(e) => Err(format!("cannot write {}: {e}", path.display())),
    };
    let _ = std::fs::remove_file(&staged);
    placed
}

fn token_path(root: &Path) -> PathBuf {
    root.join("dashboard-token")
}

/// The token already on disk, without making one: asking for a URL must not be what
/// creates the secret a board was never started with.
pub(super) fn stored_token(root: &Path) -> Option<String> {
    let existing = std::fs::read_to_string(token_path(root)).ok()?;
    let existing = existing.trim().to_string();
    (!existing.is_empty()).then_some(existing)
}

fn random_hex() -> String {
    // The operating system's entropy, not a seeded PRNG of our own: this value is what
    // stands in for a password.
    //
    // Read by the byte with `read_exact` rather than `fs::read`, which asks for the whole
    // file: `/dev/urandom` has no end, so that call never returns and the process sits
    // there eating memory before it has printed a word.
    let mut bytes = [0u8; 24];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_ok()
    {
        return bytes.iter().map(|b| format!("{b:02x}")).collect();
    }
    // Nothing on this machine can be called random. Refusing to start would be worse than
    // a weak token on a loopback socket, but it should be visible.
    eprintln!("adj serve: warning — no /dev/urandom; the token is only as good as the clock");
    format!(
        "{:x}{:x}",
        std::process::id(),
        crate::infra::clock::now_secs()
    )
}
