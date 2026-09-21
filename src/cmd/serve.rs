//! `adj serve` — the board, served to a browser on this machine.
//!
//! The dashboard is not a second coordination system. Every button on it ends in something
//! this binary could already do: a task handed over becomes a `request` in the hub's inbox
//! and a poke on its tab, exactly as `adj send` would. What the server adds is a view of
//! state that until now could only be read one `adj` invocation at a time, and a place to
//! put the questions a worker used to have to ask into a tab nobody was watching.
//!
//! It holds no clock. Nothing here polls a tracker or wakes on a timer: a request arrives
//! because a person clicked, and that is the only thing that moves.

use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::http::{self, Request};
use crate::messaging;
use crate::task;

/// The page. One file, no build step, no network fetches — it is read from the binary and
/// runs from there.
const UI_HTML: &str = include_str!("../ui.html");

pub const DEFAULT_PORT: u16 = 4577;

/// Everything a connection needs. Shared across threads, read-only after startup — the
/// state that changes lives on disk, where the hub and its workers can also reach it.
struct Server {
    ctx: super::Context,
    token: String,
    port: u16,
}

pub fn serve(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    port: u16,
    open: bool,
) -> Result<(), String> {
    let ctx = super::context(repo_arg, hub_arg)?;
    let token = token()?;

    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
        format!(
            "cannot listen on 127.0.0.1:{port}: {e}\n\
             (a dashboard may already be running — try opening http://127.0.0.1:{port}/)"
        )
    })?;
    // Asked back rather than echoed: `--port 0` is how a second one gets a free port, and
    // the number it got is the only way to reach it.
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let url = format!("http://127.0.0.1:{port}/?token={token}");

    println!("adj serve: {} — {url}", ctx.repo.nwo);
    println!("The token is in the URL. Anything without it gets a 403.");
    if open {
        open_browser(&url);
    }

    let server = Arc::new(Server { ctx, token, port });
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let server = Arc::clone(&server);
                // A thread per connection, because this one is long-lived and a browser
                // holds several at once: an accept loop that serves them one at a time
                // deadlocks the moment a second tab is opened.
                std::thread::spawn(move || {
                    if let Err(e) = handle(&server, stream) {
                        eprintln!("adj serve: connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("adj serve: accept error: {e}"),
        }
    }
    Ok(())
}

// ── the security boundary ────────────────────────────────────────────

/// Why a request is being refused, or `None` to let it through.
///
/// Pure, and separated from the routing for that reason: this is the whole of what stands
/// between a page on the internet and an endpoint that approves a diff and opens a pull
/// request, so it is the part that gets tested exhaustively.
fn refuse(token: &str, port: u16, req: &Request) -> Option<(u16, &'static str)> {
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

fn is_own_origin(origin: &str, port: u16) -> bool {
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|host| origin == format!("http://{host}:{port}"))
}

/// The shared secret, made once and kept.
///
/// Stored beside the rest of the state rather than handed out on each start: the URL is
/// meant to be a bookmark, and a token that changed every run would break it daily.
fn token() -> Result<String, String> {
    let path = messaging::state_dir().join("dashboard-token");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim().to_string();
        if !existing.is_empty() {
            return Ok(existing);
        }
    }
    let token = random_hex();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, format!("{token}\n"))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    // Readable by its owner alone: every other user on the machine can otherwise read the
    // file and post to the endpoints.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(token)
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
    format!("{:x}{:x}", std::process::id(), messaging::now_secs())
}

// ── routing ──────────────────────────────────────────────────────────

fn handle(server: &Server, mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let Some(req) = http::read_request(&mut reader)? else {
        return Ok(());
    };
    if let Some((status, why)) = refuse(&server.token, server.port, &req) {
        return http::json(&mut stream, status, &json!({ "error": why }).to_string());
    }
    route(server, &req, &mut stream)
}

fn route(server: &Server, req: &Request, out: &mut impl Write) -> std::io::Result<()> {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/" | "/index.html") => http::html(out, UI_HTML),
        ("GET", "/api/state") => http::json(out, 200, &state(server).to_string()),
        ("POST", "/api/tasks") => reply(out, create_task(server, &req.body)),
        ("POST", path) if path.starts_with("/api/tasks/") => {
            reply(out, update_task(server, req.tail(), &req.body))
        }
        _ => http::json(out, 404, &json!({ "error": "no such route" }).to_string()),
    }
}

/// A command's answer, as the page sees it. An error is a 400 with the message in it rather
/// than a 500 with nothing: every failure reachable from here is something the person can
/// act on, and the page shows the text.
fn reply(out: &mut impl Write, result: Result<Value, String>) -> std::io::Result<()> {
    match result {
        Ok(value) => http::json(out, 200, &value.to_string()),
        Err(e) => http::json(out, 400, &json!({ "error": e }).to_string()),
    }
}

// ── what the board reads ─────────────────────────────────────────────

fn state(server: &Server) -> Value {
    let repo = &server.ctx.repo;
    let hub = messaging::hub_status(&repo.slug, &repo.hub_name);
    let tasks: Vec<Value> = task::list(&super::task::dir(&server.ctx))
        .iter()
        .filter_map(|t| serde_json::to_value(t).ok())
        .collect();

    let workers: Vec<Value> = worktrees(&repo.main)
        .into_iter()
        .filter(|path| Path::new(path) != Path::new(&repo.main))
        .map(|path| {
            let status = messaging::worker_status(Path::new(&path));
            json!({
                "worktree": path,
                "name": Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()),
                "branch": branch_of(&path),
                "present": status.present,
                "stale": status.stale,
                "title": status.title,
            })
        })
        .collect();

    let pending: Vec<Value> = messaging::list(&repo.slug)
        .iter()
        .map(|entry| {
            json!({
                "name": entry.name,
                "subject": entry.subject,
                "from": entry.from,
                "kind": entry.kind,
                "worktree": entry.worktree,
            })
        })
        .collect();

    json!({
        "repo": repo.nwo,
        "main": repo.main,
        "hubName": repo.hub_name,
        "hub": {
            "present": hub.present,
            "stale": hub.stale,
            "pid": hub.pid,
            "startedAt": hub.started_at,
        },
        "tasks": tasks,
        "workers": workers,
        "pending": pending,
        // Slice 1 knows about no gates. The key is here so the page can be written once
        // against the shape it will have.
        "gates": [],
    })
}

/// The worktrees of this checkout, main one included, as absolute paths.
fn worktrees(main: &str) -> Vec<String> {
    let output = std::process::Command::new("git")
        .args(["-C", main, "worktree", "list", "--porcelain"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(str::to_string)
        .collect()
}

fn branch_of(worktree: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-C", worktree, "branch", "--show-current"])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

// ── the two things the board can change ──────────────────────────────

fn create_task(server: &Server, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let (task, handed) = super::task::create(&server.ctx, &input)?;
    Ok(json!({ "task": task, "handed": handed_json(handed) }))
}

fn update_task(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let (task, handed) = super::task::update(&server.ctx, id, &input)?;
    Ok(json!({ "task": task, "handed": handed_json(handed) }))
}

/// What the page is told about the hand-over: whether the hub was there, and whether its
/// tab was poked. Both matter to the person — a hub that is down is not an error, it just
/// means the task waits.
fn handed_json(handed: Option<super::Delivered>) -> Value {
    match handed {
        Some(d) => json!({
            "present": d.delivery.present,
            "woken": d.woken,
        }),
        None => Value::Null,
    }
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    let _ = std::process::Command::new(opener).arg(url).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, path: &str, headers: &[(&str, &str)]) -> Request {
        Request {
            method: method.to_string(),
            path: path.to_string(),
            query: match path.split_once("?token=") {
                Some((_, token)) => vec![("token".to_string(), token.to_string())],
                None => Vec::new(),
            },
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: Vec::new(),
        }
    }

    const TOKEN: &str = "s3cret";
    const PORT: u16 = 4577;

    #[test]
    fn the_page_opens_with_the_token_in_the_url() {
        let req = request("GET", "/?token=s3cret", &[]);
        assert_eq!(refuse(TOKEN, PORT, &req), None);
    }

    #[test]
    fn no_token_is_refused() {
        let req = request("GET", "/", &[]);
        assert_eq!(
            refuse(TOKEN, PORT, &req),
            Some((403, "bad or missing token"))
        );
    }

    #[test]
    fn a_wrong_token_is_refused() {
        let req = request("GET", "/?token=guess", &[]);
        assert_eq!(
            refuse(TOKEN, PORT, &req),
            Some((403, "bad or missing token"))
        );
    }

    /// The page's own calls carry the token in a header, which is also the half of the
    /// CSRF defence a cross-site form cannot reproduce.
    #[test]
    fn a_post_from_our_own_page_is_allowed() {
        let req = request(
            "POST",
            "/api/tasks",
            &[
                ("X-Adjutant-Token", TOKEN),
                ("Origin", "http://127.0.0.1:4577"),
            ],
        );
        assert_eq!(refuse(TOKEN, PORT, &req), None);
    }

    /// The attack this exists for: a page on another site knows the token (it leaked
    /// through a log, a screenshot, a shell history) and submits a form. It cannot set the
    /// header, so it dies here even holding the secret.
    #[test]
    fn a_post_carrying_the_token_only_in_the_url_is_refused() {
        let req = request(
            "POST",
            "/api/tasks?token=s3cret",
            &[("Origin", "https://evil.example")],
        );
        assert_eq!(
            refuse(TOKEN, PORT, &req),
            Some((
                403,
                "state-changing requests must send the token as a header"
            ))
        );
    }

    #[test]
    fn a_post_from_another_origin_is_refused() {
        let req = request(
            "POST",
            "/api/tasks",
            &[
                ("X-Adjutant-Token", TOKEN),
                ("Origin", "https://evil.example"),
            ],
        );
        assert_eq!(
            refuse(TOKEN, PORT, &req),
            Some((403, "cross-origin request"))
        );
    }

    /// A non-browser client (curl, a script) sends no `Origin`. It is refused for the
    /// state-changing routes rather than trusted: there is no way to tell it from a
    /// browser that stripped the header.
    #[test]
    fn a_post_with_no_origin_is_refused() {
        let req = request("POST", "/api/tasks", &[("X-Adjutant-Token", TOKEN)]);
        assert_eq!(refuse(TOKEN, PORT, &req), Some((403, "no Origin header")));
    }

    #[test]
    fn localhost_and_the_loopback_address_are_the_same_origin() {
        for origin in [
            "http://127.0.0.1:4577",
            "http://localhost:4577",
            "http://[::1]:4577",
        ] {
            assert!(is_own_origin(origin, PORT), "{origin}");
        }
        assert!(!is_own_origin("http://127.0.0.1:4578", PORT));
        assert!(!is_own_origin("https://127.0.0.1:4577", PORT));
        assert!(!is_own_origin("http://127.0.0.1:4577.evil.example", PORT));
    }

    /// Refused before the body is read, so a caller cannot make this process allocate a
    /// gigabyte by saying it is about to send one.
    #[test]
    fn an_oversized_body_is_refused_before_the_token_is_even_checked() {
        let huge = (http::MAX_BODY + 1).to_string();
        let req = request("POST", "/api/tasks", &[("Content-Length", &huge)]);
        assert_eq!(refuse(TOKEN, PORT, &req), Some((413, "body too large")));
    }
}
