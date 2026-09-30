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

use std::io::{BufReader, IsTerminal, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::gate;
use crate::http::{self, Request};
use crate::messaging;
use crate::runner;
use crate::session;
use crate::task;
use crate::ws;

/// The page. One file, no build step, no network fetches — it is read from the binary and
/// runs from there. The source is kept in pieces under `src/ui/` only so it can be read; they
/// are joined here in order, so the browser still gets a single page and every request it
/// makes still carries the token. The scripts share one global scope, so their order matters.
const UI_HTML: &str = concat!(
    include_str!("../ui/page-head.html"),
    include_str!("../ui/tokens.css"),
    include_str!("../ui/components.css"),
    include_str!("../ui/shell.css"),
    include_str!("../ui/board.css"),
    include_str!("../ui/review.css"),
    include_str!("../ui/task-view.css"),
    include_str!("../ui/console-and-dialog.css"),
    include_str!("../ui/terminal.css"),
    include_str!("../ui/page-body.html"),
    include_str!("../ui/core.js"),
    include_str!("../ui/terminal.js"),
    include_str!("../ui/board.js"),
    include_str!("../ui/actions.js"),
    include_str!("../ui/review.js"),
    include_str!("../ui/task-view.js"),
    include_str!("../ui/main.js"),
    include_str!("../ui/page-end.html"),
);

/// The terminal the board opens on a tmux session, served only by the resident server and only
/// when a page asks for it: xterm.js and the two addons it is used with, as one script. The
/// license notice comes first, as the licenses ask for it to travel with the code (the files
/// themselves are documented in `src/ui/vendor/xterm/README.md`).
const XTERM_JS: &str = concat!(
    "/*! xterm.js - MIT License\n",
    include_str!("../ui/vendor/xterm/LICENSE"),
    "\n@xterm/addon-fit and @xterm/addon-unicode11: Copyright (c) 2019, The xterm.js authors\n",
    "(https://github.com/xtermjs/xterm.js), under the same license.\n*/\n",
    include_str!("../ui/vendor/xterm/xterm.js"),
    "\n",
    include_str!("../ui/vendor/xterm/addon-fit.js"),
    "\n",
    include_str!("../ui/vendor/xterm/addon-unicode11.js"),
);

const XTERM_CSS: &str = concat!(
    "/*! xterm.js - MIT License\n",
    include_str!("../ui/vendor/xterm/LICENSE"),
    "*/\n",
    include_str!("../ui/vendor/xterm/xterm.css"),
);

pub const DEFAULT_PORT: u16 = 4577;

/// Everything a connection needs. Shared across threads, read-only after startup — the
/// state that changes lives on disk, where the hub and its workers can also reach it.
pub(super) struct Server {
    pub(super) ctx: super::Context,
    token: String,
    port: u16,
    /// Whether the resident server is the one answering, which serves this board at a path
    /// of its own. The page reads it from the state.
    pub(super) resident: bool,
    /// What Jules last said about each session a card follows. The one thing here that
    /// changes after startup, and it is a cache: the record on disk stays the answer.
    jules: Arc<super::JulesWatch>,
    /// What tmux this machine has, when the board may open terminals on it: only the resident
    /// server serves one, and only where `tmux -V` answered when it started.
    pub(super) tmux: Option<(u32, u32)>,
    /// How many board terminals are open across every board, which is what is capped.
    pub(super) terminals: Arc<AtomicUsize>,
}

pub fn serve(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    port: u16,
    open: bool,
) -> Result<(), String> {
    let ctx = super::context(repo_arg, hub_arg)?;
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
        format!(
            "cannot listen on 127.0.0.1:{port}: {e}\n\
             (a dashboard may already be running — try opening http://127.0.0.1:{port}/)"
        )
    })?;
    let board = Board::new(ctx, listener)?;
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
    board.run();
    Ok(())
}

/// What serving the board of a hub from inside its MCP server came to.
pub enum HubBoard {
    /// This process is serving the board, at this URL.
    Serving(String),
    /// The resident server serves it, at this URL. Nothing was bound here.
    Resident(String),
    /// A board for the hub is already running: one somebody started by hand with `adj serve`
    /// is left to go on serving, rather than joined by a second.
    AlreadyRunning,
}

/// Serve the board of the hub `ctx` addresses from inside that hub's MCP server, and hand
/// back where it is. A live resident server (see `server_start`) serves every board already,
/// so it is asked first: the hub only tells it where the repository is and binds nothing.
///
/// The socket and the record are both in place before this returns, so a tool call that
/// asks for the URL straight after finds it; only the accept loop goes to a thread, and it
/// ends with the process — which is the point: the MCP server lives exactly as long as the
/// hub's session does.
///
/// Nothing here writes to stdout. In that process stdout carries JSON-RPC, and a stray line
/// on it breaks the protocol for the whole session.
pub fn serve_for_hub(ctx: super::Context) -> Result<HubBoard, String> {
    if let Some(url) = resident_url(&ctx.repo) {
        return Ok(HubBoard::Resident(url));
    }
    if dashboards_running(&ctx.repo.slug).is_some() {
        return Ok(HubBoard::AlreadyRunning);
    }
    let listener =
        bind_preferring(DEFAULT_PORT).map_err(|e| format!("cannot listen on 127.0.0.1: {e}"))?;
    let board = Board::new(ctx, listener)?;
    let url = board.url();
    std::thread::spawn(move || board.run());
    Ok(HubBoard::Serving(url))
}

/// `port` on the loopback address, or any free port when `port` is taken — a second hub of
/// the same repository, a hub of another one, or something that is not ours at all. Only
/// "in use" falls back: any other failure would fail on a free port too.
fn bind_preferring(port: u16) -> std::io::Result<TcpListener> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => TcpListener::bind(("127.0.0.1", 0)),
        bound => bound,
    }
}

/// A board with its socket bound and its record written, not yet answering.
struct Board {
    server: Arc<Server>,
    listener: TcpListener,
    recorded: bool,
}

impl Board {
    fn new(ctx: super::Context, listener: TcpListener) -> Result<Board, String> {
        let token = token()?;
        // Asked back rather than taken from the caller: port 0 is how a board gets a free
        // port, and the number it got is the only way to reach it.
        let port = listener
            .local_addr()
            .map(|a| a.port())
            .map_err(|e| format!("cannot read the board's port: {e}"))?;
        let recorded = record(&ctx.repo.slug, port)?;
        Ok(Board {
            server: Arc::new(Server {
                ctx,
                token,
                port,
                resident: false,
                jules: Arc::default(),
                tmux: None,
                terminals: Arc::default(),
            }),
            listener,
            recorded,
        })
    }

    fn url(&self) -> String {
        board_url(self.server.port, &self.server.token)
    }

    fn run(self) {
        for stream in self.listener.incoming() {
            match stream {
                Ok(stream) => {
                    let server = Arc::clone(&self.server);
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
    }
}

fn board_url(port: u16, token: &str) -> String {
    format!("http://127.0.0.1:{port}/?token={token}")
}

/// The same board as the resident server serves it: under a path of its own.
fn resident_board_url(port: u16, slug: &str, token: &str) -> String {
    format!("http://127.0.0.1:{port}/b/{slug}/?token={token}")
}

/// Where a board for a hub is being served from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Served {
    Resident(u16),
    Dedicated(u16),
}

/// The resident server when there is one, the hub's own board otherwise. The resident wins
/// because it is the one that outlives the hub, and a URL that named the hub's board would
/// stop working when the hub did.
fn prefer(resident: Option<u16>, dedicated: Option<u16>) -> Option<Served> {
    resident
        .map(Served::Resident)
        .or(dedicated.map(Served::Dedicated))
}

fn served(repo: &crate::repo::RepoInfo) -> Option<Served> {
    let resident = live_resident().map(|(_, port)| port);
    if resident.is_some() {
        // Told where the repository is, so that the board this answers with can be opened.
        note_board(repo);
    }
    prefer(resident, dashboards_running(&repo.slug))
}

/// The board serving `repo`'s hub — its URL and whether the resident server is the one — or
/// `None`. Whoever started it, the token is the one every board on this machine shares.
fn located(repo: &crate::repo::RepoInfo) -> Option<(String, bool)> {
    let token = stored_token()?;
    match served(repo)? {
        Served::Resident(port) => Some((resident_board_url(port, &repo.slug, &token), true)),
        Served::Dedicated(port) => Some((board_url(port, &token), false)),
    }
}

/// `board` as `adj config` and `adjutant_config` report it: where it is, and whether the
/// resident server serves it. `null` when nothing does.
pub fn board_json(repo: &crate::repo::RepoInfo) -> Value {
    match located(repo) {
        Some((url, resident)) => json!({ "url": url, "resident": resident }),
        None => Value::Null,
    }
}

/// Where the resident server serves `repo`'s board, when a resident is live.
fn resident_url(repo: &crate::repo::RepoInfo) -> Option<String> {
    let (_, port) = live_resident()?;
    let token = stored_token()?;
    note_board(repo);
    Some(resident_board_url(port, &repo.slug, &token))
}

// ── is anybody serving? ──────────────────────────────────────────────

fn record_path(slug: &str) -> PathBuf {
    messaging::state_dir()
        .join("dashboards")
        .join(format!("{slug}.json"))
}

fn live_record(slug: &str) -> Option<(u32, u16)> {
    live_at(&record_path(slug))
}

/// The pid and port `path` records, when the process is still the one that wrote them.
fn live_at(path: &Path) -> Option<(u32, u16)> {
    let record: Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())?;
    let pid = record.get("pid").and_then(Value::as_u64)? as u32;
    let started = record.get("psStarted").and_then(Value::as_str);
    if messaging::ps_started(pid).as_deref() != started {
        return None;
    }
    let port = record
        .get("port")
        .and_then(Value::as_u64)
        .map(|p| p as u16)?;
    Some((pid, port))
}

/// The port a live dashboard is on for `repo`'s hub, the resident server's or the hub's own,
/// or `None`.
///
/// This is what `adj gate open` asks before it hands the ball over: a gate written with
/// nobody serving is a message into a directory no one opens, and an agent that waited on
/// one would wait for ever. Anchored on the recorded process start time like every other
/// record here, so a crashed server leaves a file that reads as absent rather than as a
/// dashboard that is about to answer.
pub fn running(repo: &crate::repo::RepoInfo) -> Option<u16> {
    match served(repo)? {
        Served::Resident(port) | Served::Dedicated(port) => Some(port),
    }
}

/// The port of a board of its own for `slug`, one started by a hub or by `adj serve`. The
/// resident server is not asked: this is what a hub's MCP server checks before it binds one.
pub fn dashboards_running(slug: &str) -> Option<u16> {
    live_record(slug).map(|(_, port)| port)
}

/// Record this board as the one serving the hub. Returns `Ok(true)` if it wrote the record,
/// and `Ok(false)` if another live board holds it and nothing was written.
/// A second board (for instance one started in a worktree to check a UI change) must not
/// take the record from a live board, because once it stops the record would name a dead
/// process and the live board would read as absent.
fn record(slug: &str, port: u16) -> Result<bool, String> {
    if live_record(slug).is_some_and(|(pid, _)| pid != std::process::id()) {
        return Ok(false);
    }
    let path = record_path(slug);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let pid = std::process::id();
    let record = json!({ "pid": pid, "port": port, "psStarted": messaging::ps_started(pid) });
    std::fs::write(&path, format!("{record:#}\n"))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(true)
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
    if let Some(existing) = stored_token() {
        return Ok(existing);
    }
    let path = token_path();
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
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => match stored_token() {
            Some(theirs) => Ok(theirs),
            // An empty file is no token at all, and is replaced as it always was — by a
            // rename, so the replacement is whole and owner-only too. Read back afterwards,
            // since another start may be replacing it at the same time and the last one in
            // is the one every URL will carry.
            None => std::fs::rename(&staged, &path)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))
                .map(|()| stored_token().unwrap_or(token)),
        },
        Err(e) => Err(format!("cannot write {}: {e}", path.display())),
    };
    let _ = std::fs::remove_file(&staged);
    placed
}

fn token_path() -> PathBuf {
    messaging::state_dir().join("dashboard-token")
}

/// The token already on disk, without making one: asking for a URL must not be what
/// creates the secret a board was never started with.
fn stored_token() -> Option<String> {
    let existing = std::fs::read_to_string(token_path()).ok()?;
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
    format!("{:x}{:x}", std::process::id(), messaging::now_secs())
}

// ── the resident server ──────────────────────────────────────────────
//
// One process per state directory that serves every repository's board, each under
// `/b/<slug>/`, whether or not a hub is running. It finds the repositories through an
// address book — `boards/<slug>.json`, written by whoever learns where a repository is —
// and builds each board's context from that on first use.

fn server_lock_path() -> PathBuf {
    messaging::state_dir().join("server.lock")
}

fn server_record_path() -> PathBuf {
    messaging::state_dir().join("server.json")
}

fn server_log_path() -> PathBuf {
    messaging::state_dir().join("server.log")
}

fn boards_dir() -> PathBuf {
    messaging::state_dir().join("boards")
}

/// The pid and port of the resident server, when one is running. Anchored on the recorded
/// process start time like every other record here, so a killed server leaves a file that
/// reads as absent.
fn live_resident() -> Option<(u32, u16)> {
    live_at(&server_record_path())
}

/// Whether a resident server is running.
pub fn resident_running() -> bool {
    live_resident().is_some()
}

/// Write `text` to `path` whole or not at all: to a file of our own beside it, then a rename.
/// A reader — `adj server status`, a hub asking where the board is — never sees half a record.
fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let staged = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&staged, text)
        .and_then(|()| std::fs::rename(&staged, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            format!("cannot write {}: {e}", path.display())
        })
}

/// Tell the resident server where `repo` is, so that it can serve its board. Skipped when the
/// entry is already what it would write, and a failure is not one for the caller: the address
/// is only ever a convenience for a server that may not be running.
pub fn note_board(repo: &crate::repo::RepoInfo) {
    let entry = json!({ "main": repo.main, "nwo": repo.nwo, "hub": repo.hub });
    let path = boards_dir().join(format!("{}.json", repo.slug));
    if messaging::read_json(&path).as_ref() == Some(&entry) {
        return;
    }
    let _ = write_whole(&path, &format!("{entry:#}\n"));
}

/// Take `slug` out of the address book, so a closed hub is not offered a board any more.
pub(super) fn forget_board(slug: &str) -> Result<(), String> {
    let path = boards_dir().join(format!("{slug}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
    }
}

/// The pieces of the address book entry `slug` names, when they still describe that board:
/// the checkout is there and the repository and hub still come to the same slug.
#[derive(Clone, PartialEq, Eq)]
struct Address {
    slug: String,
    main: String,
    nwo: String,
    hub: Option<String>,
}

fn address_of(slug: &str) -> Option<Address> {
    let entry = messaging::read_json(&boards_dir().join(format!("{slug}.json")))?;
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    let address = Address {
        slug: slug.to_string(),
        main: text("main")?,
        nwo: text("nwo")?,
        hub: text("hub").filter(|hub| !hub.is_empty()),
    };
    (Path::new(&address.main).is_dir()
        && crate::repo::slug_for(&address.nwo, address.hub.as_deref()) == slug)
        .then_some(address)
}

/// Every board the address book names, by slug.
fn addresses() -> Vec<Address> {
    let mut slugs: Vec<String> = std::fs::read_dir(boards_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .collect();
    slugs.sort();
    slugs.iter().filter_map(|slug| address_of(slug)).collect()
}

/// The board list as `/api/boards` and `adj server status` give it.
fn boards_json(port: u16, token: &str) -> Vec<Value> {
    addresses()
        .into_iter()
        .map(|a| {
            let present = crate::repo::hub_name(&a.nwo, a.hub.as_deref())
                .map(|name| messaging::hub_status(&a.slug, &name).present)
                .unwrap_or(false);
            json!({
                "slug": a.slug,
                "nwo": a.nwo,
                "hub": a.hub,
                "url": resident_board_url(port, &a.slug, token),
                "hubPresent": present,
            })
        })
        .collect()
}

/// The repository this process stands in, if it stands in one. Whether it is one is not the
/// business of the commands that ask.
fn checkout_here() -> Option<crate::repo::RepoInfo> {
    super::resolve(None, None).ok()
}

/// Seed the address book from where this process stands and from the hub records already on
/// disk, so that the boards of hubs started before the resident are there from the first
/// request.
fn seed_boards() {
    if let Some(repo) = checkout_here() {
        note_board(&repo);
    }
    let Ok(entries) = std::fs::read_dir(messaging::state_dir().join("hubs")) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        let Some(slug) = path
            .extension()
            .filter(|ext| *ext == "json")
            .and_then(|_| path.file_stem())
            .and_then(|stem| stem.to_str())
        else {
            continue;
        };
        let Some(record) = messaging::read_json(&path) else {
            continue;
        };
        let Some(cwd) = record.get("cwd").and_then(Value::as_str) else {
            continue;
        };
        let hub = record.get("hub").and_then(Value::as_str);
        if let Ok(repo) = crate::repo::resolve_in(Some(Path::new(cwd)), None, hub)
            && repo.slug == slug
        {
            note_board(&repo);
        }
    }
}

/// `/b/<slug>/rest` as its slug and the path the board itself sees. A slug is what
/// `repo::slug_for` makes — lowercase letters, digits and `-` — and anything else is not a
/// board, so nothing that reaches the address book or the file system is ever a stranger's
/// string.
fn split_board_path(path: &str) -> Option<(&str, &str)> {
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
struct Resident {
    token: String,
    port: u16,
    /// Each open board with the address it was built from, so that one whose address has
    /// changed since is not served from the old context.
    boards: Mutex<std::collections::HashMap<String, (Address, Arc<Server>)>>,
    /// The tmux version, asked once at start: the board terminal is offered only with one.
    tmux: Option<(u32, u32)>,
    /// Open board terminals, over all boards.
    terminals: Arc<AtomicUsize>,
}

impl Resident {
    /// The board for `slug`, or `None` when the address book has no such board any more.
    ///
    /// The address is read on every call — a small file — and the open board is used only
    /// while it is the one the file still names: a checkout that moved, or was replaced under
    /// the same slug, is served from where it is now.
    fn board(&self, slug: &str) -> Option<Arc<Server>> {
        // Bounded: a file rewritten again and again while this builds is not worth chasing.
        for _ in 0..3 {
            let Some(address) = address_of(slug) else {
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
            let repo = crate::repo::resolve_in(
                Some(Path::new(&address.main)),
                Some(&address.nwo),
                address.hub.as_deref(),
            )
            .ok()
            .filter(|repo| repo.slug == slug)?;
            let ctx = super::context_of(repo).ok()?;
            let server = Arc::new(Server {
                ctx,
                token: self.token.clone(),
                port: self.port,
                resident: true,
                jules: Arc::default(),
                tmux: self.tmux,
                terminals: Arc::clone(&self.terminals),
            });
            let mut boards = self.boards.lock().ok()?;
            // Asked again under the lock: what was built is only put in place while it is still
            // what the file says, so a slower build of an older address cannot replace a newer
            // one — and one that lost the race is thrown away and built again.
            if address_of(slug).as_ref() != Some(&address) {
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

fn handle_resident(resident: &Resident, mut stream: TcpStream) -> std::io::Result<()> {
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
        Ok(id) => super::board_terminal::serve(&server, &id, req, stream, reader),
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
        ("GET", "/" | "/index.html") => http::html(out, &index_page(resident)),
        ("GET", "/api/boards") => http::json(
            out,
            200,
            &Value::Array(boards_json(resident.port, &resident.token)).to_string(),
        ),
        _ => http::json(out, 404, &json!({ "error": "no such route" }).to_string()),
    }
}

fn html_escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The list of boards, for `/`. Plain HTML with no script: it is a list of links.
fn index_page(resident: &Resident) -> String {
    let boards = boards_json(resident.port, &resident.token);
    let items: String = boards
        .iter()
        .map(|b| {
            let slug = b["slug"].as_str().unwrap_or_default();
            let nwo = b["nwo"].as_str().unwrap_or_default();
            let name = match b["hub"].as_str() {
                Some(key) => format!("{nwo}（{key}）"),
                None => nwo.to_string(),
            };
            let state = if b["hubPresent"].as_bool().unwrap_or(false) {
                "hub 稼働中"
            } else {
                "hub 停止中"
            };
            format!(
                "<li><a href=\"/b/{slug}/?token={}\">{}</a> <span class=\"state\">{state}</span></li>\n",
                resident.token,
                html_escaped(&name)
            )
        })
        .collect();
    let body = if items.is_empty() {
        "<p>まだボードがありません。リポジトリで adj server start か adj hub を実行してください。</p>"
            .to_string()
    } else {
        format!("<ul>\n{items}</ul>")
    };
    format!(
        "<!DOCTYPE html>\n<html lang=\"ja\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>adj ボード一覧</title>\n\
         <style>body{{font-family:system-ui,sans-serif;margin:2rem auto;max-width:40rem;padding:0 1rem;line-height:1.7}}\
         li{{margin:.4rem 0}}.state{{color:#666;font-size:.85em;margin-left:.5em}}</style>\n\
         </head>\n<body>\n<h1>ボード</h1>\n{body}\n</body>\n</html>\n"
    )
}

/// A relative `ADJUTANT_STATE_DIR`, made absolute against where this was started, so that
/// the resident and the process that started it — which stand in different places once the
/// resident is detached — read the same directory. The resident never changes directory.
fn anchor_state_dir() {
    let Ok(value) = std::env::var(messaging::STATE_DIR_ENV) else {
        return;
    };
    let path = Path::new(&value);
    if value.is_empty() || value == "~" || value.starts_with("~/") || path.is_absolute() {
        return;
    }
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    if let Ok(absolute) = std::path::absolute(cwd.join(path)) {
        // SAFETY: called from `server_start` before it starts a thread.
        unsafe { std::env::set_var(messaging::STATE_DIR_ENV, absolute) };
    }
}

/// `adj server start`. Detached unless `foreground`, which is what a service manager and the
/// tests run.
pub fn server_start(port: u16, foreground: bool, open: bool) -> Result<i32, String> {
    anchor_state_dir();
    if foreground {
        return serve_resident(port, open);
    }
    let here = checkout_here();
    if let Some((pid, port)) = live_resident() {
        if let Some(repo) = &here {
            note_board(repo);
        }
        let shown = shown_url(port, here.as_ref())?;
        println!("adj server: already running (pid {pid}) — {shown}");
        return Ok(0);
    }
    let child = spawn_resident(port)?;
    let port = wait_for_resident(child)?;
    let index = resident_index_url(port)?;
    println!("adj server: serving on {index}");
    if let Some(repo) = &here {
        note_board(repo);
        println!(
            "adj server: {} — {}",
            repo.nwo,
            shown_url(port, here.as_ref())?
        );
    }
    if open {
        open_browser(&shown_url(port, here.as_ref())?);
    }
    Ok(0)
}

fn resident_index_url(port: u16) -> Result<String, String> {
    Ok(board_url(port, &token()?))
}

/// The URL worth showing: the board of the checkout this stands in, else the index.
fn shown_url(port: u16, here: Option<&crate::repo::RepoInfo>) -> Result<String, String> {
    match here {
        Some(repo) => Ok(resident_board_url(port, &repo.slug, &token()?)),
        None => resident_index_url(port),
    }
}

/// `path` opened for appending, readable by its owner alone. The log is written by a server
/// that holds a secret, and a file another user on the machine can read is a way to it — so a
/// log an earlier version made with looser permissions is tightened too.
fn private_log(path: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("cannot restrict {}: {e}", path.display()))?;
    Ok(file)
}

/// The resident, started in a process group of its own so that closing the terminal it was
/// started from does not take it along, with what it says written to `server.log`.
fn spawn_resident(port: u16) -> Result<std::process::Child, String> {
    use std::os::unix::process::CommandExt;

    let log = server_log_path();
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let open_log = || private_log(&log);
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this binary: {e}"))?;
    let mut command = std::process::Command::new(exe);
    command
        .args(["server", "start", "--foreground", "--no-open", "--port"])
        .arg(port.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(open_log()?)
        .stderr(open_log()?)
        // What the agent is started without, for the same reason: this one answers for every
        // repository and must not inherit the identity of the hub it was started from.
        .env_remove(messaging::HUB_SESSION_ENV)
        .env_remove(messaging::HUB_SERVE_ENV)
        .env_remove(messaging::HUB_ENV)
        .process_group(0);
    for name in crate::repo::REPOSITORY_LOCATION_ENV {
        command.env_remove(name);
    }
    command
        .spawn()
        .map_err(|e| format!("cannot start adj server: {e}"))
}

/// Wait for the resident to say where it is — it writes `server.json` once it is listening.
fn wait_for_resident(mut child: std::process::Child) -> Result<u16, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some((_, port)) = live_resident() {
            return Ok(port);
        }
        // A child that has exited is not yet a failure: it exits when another resident holds
        // the lock, and that one may be a moment from writing its record. Asked for its status
        // either way, which is also what reaps it.
        let _ = child.try_wait();
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "adj server did not start; see {}",
                server_log_path().display()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// The resident itself: holds `server.lock` for as long as it runs, binds, says where it is,
/// and answers.
fn serve_resident(port: u16, open: bool) -> Result<i32, String> {
    let lock_path = server_lock_path();
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    // Held for the process's lifetime and released by the system when it ends, however it
    // ends — the same lock `messaging::take_over` takes, for the same reason.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("cannot open {}: {e}", lock_path.display()))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(match live_resident() {
                Some((pid, _)) => format!("another adj server is running (pid {pid})"),
                None => "another adj server is running".to_string(),
            });
        }
        Err(std::fs::TryLockError::Error(e)) => {
            return Err(format!("cannot lock {}: {e}", lock_path.display()));
        }
    }
    let listener =
        bind_preferring(port).map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))?;
    let bound = listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| format!("cannot read the server's port: {e}"))?;
    if port != 0 && bound != port {
        eprintln!("adj server: 127.0.0.1:{port} is taken; serving on {bound} instead");
    }
    let token = token()?;
    let pid = std::process::id();
    let record = json!({
        "pid": pid,
        "psStarted": messaging::ps_started(pid),
        "port": bound,
        "startedAt": messaging::utc_stamp(messaging::now_secs()),
        "version": env!("CARGO_PKG_VERSION"),
    });
    write_whole(&server_record_path(), &format!("{record:#}\n"))?;
    seed_boards();
    let index = board_url(bound, &token);
    // The token goes to a terminal and nowhere else: detached, or under a service manager,
    // stdout is a log file, and a log is kept, attached to bug reports and read by others. The
    // process that started it prints the whole URL to the person who asked.
    if std::io::stdout().is_terminal() {
        println!("adj server: serving on {index}");
    } else {
        println!("adj server: serving on http://127.0.0.1:{bound}/");
    }
    if open {
        open_browser(&index);
    }
    let resident = Arc::new(Resident {
        token,
        port: bound,
        boards: Mutex::default(),
        tmux: board_terminal_tmux(),
        terminals: Arc::default(),
    });
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let resident = Arc::clone(&resident);
                // A thread per connection, for the reason `Board::run` gives.
                std::thread::spawn(move || {
                    if let Err(e) = handle_resident(&resident, stream) {
                        eprintln!("adj server: connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("adj server: accept error: {e}"),
        }
    }
    drop(lock);
    Ok(0)
}

/// What `server.json` names, whether or not that process is still there.
fn recorded_resident() -> Option<(u32, Option<String>)> {
    let record = messaging::read_json(&server_record_path())?;
    let pid = record.get("pid").and_then(Value::as_u64)? as u32;
    let started = record
        .get("psStarted")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((pid, started))
}

/// Whether `record` still names the process `pid` started at `started`. The rule for removing
/// `server.json`: a supervisor that restarts the server may already have written the next
/// record, and that one is not ours to remove.
fn names_resident(record: Option<&(u32, Option<String>)>, pid: u32, started: Option<&str>) -> bool {
    record.is_some_and(|(p, s)| *p == pid && s.as_deref() == started)
}

fn forget_resident(pid: u32, started: Option<&str>) {
    if names_resident(recorded_resident().as_ref(), pid, started) {
        let _ = std::fs::remove_file(server_record_path());
    }
}

/// `adj server stop`. Only the server: a hub is a session of its own and goes on running.
pub fn server_stop() -> Result<i32, String> {
    let named = recorded_resident();
    let Some((pid, _)) = live_resident() else {
        if let Some((pid, started)) = &named {
            forget_resident(*pid, started.as_deref());
        }
        println!("adj server is not running");
        return Ok(0);
    };
    let started = named.and_then(|(_, s)| s);
    let status = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .map_err(|e| format!("cannot run kill: {e}"))?;
    if !status.success() {
        return Err(format!("cannot stop adj server (pid {pid})"));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    // The process this record named, not whatever the record names by now: a supervisor may
    // already have started the next one.
    let still_there = || match &started {
        Some(started) => messaging::ps_started(pid).as_deref() == Some(started.as_str()),
        None => live_resident().is_some_and(|(p, _)| p == pid),
    };
    while still_there() {
        if std::time::Instant::now() >= deadline {
            return Err(format!("adj server (pid {pid}) did not stop within 5s"));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    forget_resident(pid, started.as_deref());
    println!("stopped adj server (pid {pid})");
    Ok(0)
}

/// `adj server status`. Exit 1 when there is no resident, so a script can ask.
pub fn server_status(as_json: bool) -> Result<i32, String> {
    let Some((pid, port)) = live_resident() else {
        if as_json {
            println!("{}", json!({ "running": false }));
        } else {
            println!("adj server is not running");
        }
        return Ok(1);
    };
    let token = stored_token().ok_or("the dashboard token is missing")?;
    let index = board_url(port, &token);
    let boards = boards_json(port, &token);
    if as_json {
        let out = json!({
            "running": true,
            "pid": pid,
            "port": port,
            "url": index,
            "boards": boards,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(0);
    }
    println!("adj server: running (pid {pid}) on port {port}");
    println!("index: {index}");
    for board in &boards {
        let hub = board["hub"].as_str().unwrap_or("-");
        let state = if board["hubPresent"].as_bool().unwrap_or(false) {
            "running"
        } else {
            "stopped"
        };
        println!(
            "{} (hub {hub}, {state}) — {}",
            board["nwo"].as_str().unwrap_or_default(),
            board["url"].as_str().unwrap_or_default()
        );
    }
    Ok(0)
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
        ("POST", "/api/sessions") => reply(out, super::session::start_request(server, &req.body)),
        ("POST", path) if link_route(path).is_some() => {
            let result = link_route(path)
                .unwrap_or_else(|| Err("no such route".to_string()))
                .and_then(|id| super::session::link(server, &id, &req.body));
            reply(out, result)
        }
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
fn reply(out: &mut impl Write, result: Result<Value, String>) -> std::io::Result<()> {
    match result {
        Ok(value) => http::json(out, 200, &value.to_string()),
        Err(e) => http::json(out, 400, &json!({ "error": e }).to_string()),
    }
}

// ── what the board reads ─────────────────────────────────────────────

/// Where a session runs: what its record says it was started in, or — for a record written
/// before it said so, and for a hub, whose record does not — the settings and a live look
/// through tmux for the pid.
fn session_terminal(
    record: Option<&Value>,
    terminal_settings: &crate::config::TerminalSettings,
    tmux_panes: &[crate::terminal::TmuxPane],
    pid: Option<u32>,
) -> session::SessionTerminal {
    if let Some(recorded) = record
        .and_then(|r| r.get("terminal"))
        .and_then(|t| serde_json::from_value::<session::SessionTerminal>(t.clone()).ok())
    {
        return recorded;
    }
    let tmux = terminal_settings.spawn.is_none() && terminal_settings.is_tmux();
    let pane = pid
        .filter(|_| tmux)
        .and_then(|p| crate::terminal::find_matching_pane(tmux_panes, Some(p), None));
    session::SessionTerminal {
        backend: crate::terminal::backend_name(terminal_settings).to_string(),
        socket: terminal_settings
            .tmux_socket()
            .filter(|_| tmux)
            .map(str::to_string),
        session: tmux.then(|| terminal_settings.tmux_session().to_string()),
        window: pane.map(|p| p.window_id.clone()),
        pane: pane.map(|p| p.pane_id.clone()),
    }
}

/// The `hubs[]` id of the hub a worker names by `key`: the repository's own hub when it
/// names none, and one made from the key when no hub of that key was found.
fn parent_hub_id(
    repo: &crate::repo::RepoInfo,
    hubs: &[session::RepoHub],
    key: Option<&str>,
) -> String {
    match key.map(str::trim).filter(|s| !s.is_empty()) {
        Some(key) => {
            let slug = crate::repo::slug_for(&repo.nwo, Some(key));
            hubs.iter()
                .find(|h| h.slug == slug)
                .map(|h| h.id.clone())
                .unwrap_or_else(|| format!("hub-{key}"))
        }
        None => "hub".to_string(),
    }
}

/// The board ids of the workers in `paths`, in order: `worker-<name>` for the worktree's own
/// name, and `worker-<name>-<digest of the path>` when another of them has that name. A
/// worktree called `main` keeps `worker-main` unless the main checkout's own session
/// (`main_listed`) is on the board and has it. A name that is not shared keeps the id it always
/// had, so nothing that already holds one is told a new one.
fn worker_session_ids(paths: &[String], main_listed: bool) -> Vec<String> {
    let name_of = |path: &str| {
        Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let names: Vec<String> = paths.iter().map(|p| name_of(p)).collect();
    paths
        .iter()
        .zip(&names)
        .map(|(path, name)| {
            let shared = names.iter().filter(|other| *other == name).count() > 1
                || (main_listed && name == "main");
            match shared {
                true => format!("worker-{name}-{}", crate::repo::short_digest(path)),
                false => format!("worker-{name}"),
            }
        })
        .collect()
}

fn state(server: &Server) -> Value {
    // Before the gates are read: a gate whose worker has moved on is closed here rather than
    // by a timer, since nothing in the server polls on one.
    let _ = super::gate::close_resumed(&server.ctx);
    let repo = &server.ctx.repo;
    let hub = messaging::hub_status(&repo.slug, &repo.hub_name);
    let tasks = with_records(
        task::list(&super::task::dir(&server.ctx)),
        gate::list(&super::gate::records_dir(&server.ctx)),
        gate::list_of_kind(&super::gate::answered_dir(&server.ctx), gate::Kind::Plan),
    );

    let now = messaging::now_secs();
    let settings = settings_now(server);
    // After the records are joined, from the same values the page gets: a card shows the last
    // answer about its session, and an old answer is asked again behind the page's back.
    let tasks: Vec<Value> = tasks
        .into_iter()
        .map(|mut t| {
            if let Some(seen) = server.jules.look(&server.ctx, &settings.jules_key, &t) {
                t["jules"] = seen;
            }
            t
        })
        .collect();
    let shown: std::collections::HashSet<String> = tasks
        .iter()
        .filter_map(|t| t["jules"]["session"].as_str().map(str::to_string))
        .collect();
    server.jules.keep_only(&shown);
    // Counted as `adj work` counts, main checkout included, though it is not listed below.
    let mut busy = usize::from(messaging::holds_worker_slot(Path::new(&repo.main), now));
    // The board shows what it can; `adj work` is the one that refuses on a failed listing.
    let linked_paths = crate::repo::linked_worktrees(&repo.main).unwrap_or_default();
    let mut workers_data = Vec::with_capacity(linked_paths.len());
    let mut workers: Vec<Value> = Vec::with_capacity(linked_paths.len());
    for path in &linked_paths {
        let status = messaging::worker_status(Path::new(path));
        // A present worker holds a slot without asking `ps` again; the rest are asked
        // the way `adj work` asks, so the header and the refusal cannot disagree.
        if status.present || messaging::holds_worker_slot(Path::new(path), now) {
            busy += 1;
        }
        let branch = branch_of(path);
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        let task = messaging::worker_task(Path::new(path));
        workers.push(json!({
            "worktree": path,
            "name": name,
            "branch": branch.clone(),
            "present": status.present,
            "stale": status.stale,
            "title": status.title,
            // The task this worker reports for, which is what the card joins on: a worker
            // with none is a session that has no card until it is linked.
            "task": task,
            "phase": status.phase,
            "phaseAt": status.phase_at,
        }));
        workers_data.push((status, branch));
    }

    let hubs = messaging::all_repo_hubs(repo);

    let sessions = sessions_of(server, &settings, &hubs, &linked_paths, workers_data);

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
        "hubs": hubs,
        // Whether the resident server serves this board, which is also what tells the page
        // it lives under a path of its own.
        "resident": server.resident,
        // Whether the board may start a hub: only where the settings mean a tmux window.
        "hubStart": { "available": super::hub_startable(&settings.terminal) },
        // Whether the board can open a terminal on a session that runs in tmux: the resident
        // server, on a machine that has tmux. Which sessions is for the page to read from
        // `sessions[].terminal` and `present`.
        "boardTerminal": { "available": server.resident && server.tmux.is_some() },
        "sessions": sessions,
        "tasks": tasks,
        "workers": workers,
        // The slot count `adj work` decides by, counted the same way — a worker still
        // starting up holds one — so the header and the refusal cannot disagree.
        "workerSlots": {
            "busy": busy,
            "max": settings.max_workers,
        },
        // Minutes in one phase before a card is flagged. `0` = never.
        "stuckAfterMinutes": settings.stuck_after_minutes,
        // Whether the IDE buttons can do anything, and where to set it when they cannot. Read
        // on every poll, so an `ide` written into the config shows up without a restart.
        "ideConfigured": crate::ide::configured(settings.ide.as_deref()),
        "configPath": crate::config::config_path().to_string_lossy(),
        "now": now,
        "pending": pending,
        "gates": gate::list(&super::gate::dir(&server.ctx)),
    })
}

/// The sessions this board lists, hubs first and then the workers of `linked_paths` (with what
/// was already asked of each), as the page reads them and as a board terminal resolves an id.
fn sessions_of(
    server: &Server,
    settings: &crate::config::Settings,
    hubs: &[session::RepoHub],
    linked_paths: &[String],
    workers_data: Vec<(messaging::WorkerStatus, Option<String>)>,
) -> Vec<session::Session> {
    let repo = &server.ctx.repo;
    let terminal_settings = &settings.terminal;
    let tmux_panes = if terminal_settings.spawn.is_none() && terminal_settings.is_tmux() {
        crate::terminal::list_tmux_panes(terminal_settings.tmux_socket()).unwrap_or_default()
    } else {
        Vec::new()
    };

    let hub_agent = runner::agent_from_runner(
        settings
            .hub_runner
            .as_deref()
            .unwrap_or(runner::DEFAULT_HUB_RUNNER),
    );
    let worker_agent = runner::agent_from_runner(
        settings
            .agent_runner
            .as_deref()
            .unwrap_or(runner::DEFAULT_AGENT_RUNNER),
    );

    let main_branch = branch_of(&repo.main);
    let mut sessions: Vec<session::Session> = Vec::new();

    // 1. Hub sessions from hubs
    for h in hubs {
        let record = messaging::read_json(&messaging::hub_record_path(&h.slug));
        let terminal =
            session_terminal(record.as_ref(), terminal_settings, &tmux_panes, h.state.pid);

        sessions.push(session::Session {
            id: h.id.clone(),
            conversation: messaging::hub_session(&h.slug).map(|s| s.session_id),
            kind: "hub".to_string(),
            agent: hub_agent.clone(),
            terminal,
            hub: None,
            key: h.key.clone(),
            worktree: repo.main.clone(),
            branch: main_branch.clone(),
            task: None,
            title: Some(h.name.clone()),
            present: h.state.present,
            stale: h.state.stale,
            pid: h.state.pid,
            started_at: h.state.started_at.clone(),
            phase: None,
            phase_at: None,
        });
    }

    // 2. Worker sessions from linked worktrees
    // Whether the main checkout is listed below as `worker-main`, which a worktree of that
    // name would otherwise collide with.
    let main_listed = messaging::read_json(&messaging::worker_record_path(Path::new(&repo.main)))
        .is_some()
        || messaging::worker_session(Path::new(&repo.main)).is_some();
    let worker_ids = worker_session_ids(linked_paths, main_listed);
    for ((path, id), (status, branch)) in linked_paths.iter().zip(worker_ids).zip(workers_data) {
        let wt_path = Path::new(path);
        let record_json = messaging::read_json(&messaging::worker_record_path(wt_path));
        let saved_session = messaging::worker_session(wt_path);
        let parent_hub = parent_hub_id(repo, hubs, messaging::worker_hub_key(wt_path).as_deref());
        let started_at = record_json
            .as_ref()
            .and_then(|r| r.get("startedAt"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let conversation = saved_session.as_ref().map(|s| s.session_id.clone());

        let terminal = session_terminal(
            record_json.as_ref(),
            terminal_settings,
            &tmux_panes,
            status.pid,
        );

        let saved_title = saved_session.and_then(|s| s.title);
        let task_id = messaging::worker_task(wt_path);

        let title = status.title.or(saved_title);

        sessions.push(session::Session {
            id,
            conversation,
            kind: "worker".to_string(),
            agent: worker_agent.clone(),
            terminal,
            hub: Some(parent_hub),
            key: None,
            worktree: path.clone(),
            branch,
            task: task_id,
            title,
            present: status.present,
            stale: status.stale,
            pid: status.pid,
            started_at,
            phase: status.phase,
            phase_at: status.phase_at,
        });
    }

    // Also check worker in main checkout if one exists
    let main_record_path = messaging::worker_record_path(Path::new(&repo.main));
    if let Some(record_json) = messaging::read_json(&main_record_path) {
        let status = messaging::worker_status(Path::new(&repo.main));
        let parent_hub = parent_hub_id(
            repo,
            hubs,
            messaging::worker_hub_key(Path::new(&repo.main)).as_deref(),
        );
        let started_at = record_json
            .get("startedAt")
            .and_then(Value::as_str)
            .map(str::to_string);

        let terminal = session_terminal(
            Some(&record_json),
            terminal_settings,
            &tmux_panes,
            status.pid,
        );

        let task_id = record_json
            .get("task")
            .and_then(Value::as_str)
            .map(str::to_string);

        sessions.push(session::Session {
            id: "worker-main".to_string(),
            conversation: messaging::worker_session(Path::new(&repo.main)).map(|s| s.session_id),
            kind: "worker".to_string(),
            agent: worker_agent.clone(),
            terminal,
            hub: Some(parent_hub),
            key: None,
            worktree: repo.main.clone(),
            branch: main_branch.clone(),
            task: task_id,
            title: status.title,
            present: status.present,
            stale: status.stale,
            pid: status.pid,
            started_at,
            phase: status.phase,
            phase_at: status.phase_at,
        });
    } else if let Some(saved) = messaging::worker_session(Path::new(&repo.main)) {
        let parent_hub = parent_hub_id(repo, hubs, saved.hub.as_deref());
        let terminal = session_terminal(None, terminal_settings, &tmux_panes, None);

        sessions.push(session::Session {
            id: "worker-main".to_string(),
            conversation: Some(saved.session_id.clone()),
            kind: "worker".to_string(),
            agent: worker_agent.clone(),
            terminal,
            hub: Some(parent_hub),
            key: None,
            worktree: repo.main.clone(),
            branch: main_branch,
            task: saved.task,
            title: saved.title,
            present: false,
            stale: false,
            pid: None,
            started_at: None,
            phase: None,
            phase_at: None,
        });
    }
    sessions
}

/// The sessions of this board, asked for on their own: what the board terminal resolves a
/// session id against, so that the page never names a socket or a window.
pub(super) fn board_sessions(
    server: &Server,
    settings: &crate::config::Settings,
) -> Vec<session::Session> {
    let repo = &server.ctx.repo;
    let hubs = messaging::all_repo_hubs(repo);
    let linked_paths = crate::repo::linked_worktrees(&repo.main).unwrap_or_default();
    let workers_data = linked_paths
        .iter()
        .map(|path| (messaging::worker_status(Path::new(path)), branch_of(path)))
        .collect();
    sessions_of(server, settings, &hubs, &linked_paths, workers_data)
}

/// The tasks as the board reads them, each live one with what its worker recorded without
/// stopping (`records`, oldest first) and the plan a person approved (`approvedPlan`, whose
/// `answeredAt` is when).
///
/// Joined here rather than written onto the task record: a record belongs to the gate
/// directory, and a copy on the task would be a second place for it that can disagree.
/// A finished task gets neither — nobody reads its card for them, and the archive only grows.
fn with_records(
    tasks: Vec<task::Task>,
    records: Vec<gate::Gate>,
    answered: Vec<gate::Gate>,
) -> Vec<Value> {
    tasks
        .into_iter()
        .filter_map(|t| {
            let live = !matches!(t.status, task::Status::Done | task::Status::Cancelled);
            let mut value = serde_json::to_value(&t).ok()?;
            if live {
                let mine = |g: &&gate::Gate| g.task.as_deref() == Some(t.id.as_str());
                let records: Vec<&gate::Gate> = records.iter().filter(mine).collect();
                // The latest, because a plan sent back with `changes` is opened again, and the
                // one that was approved last is the one being worked to.
                let plan = answered
                    .iter()
                    .filter(mine)
                    .filter(|g| g.kind == gate::Kind::Plan)
                    .filter(|g| matches!(g.decision.as_deref(), Some("approve" | "choice")))
                    .max_by(|a, b| a.answered_at.cmp(&b.answered_at));
                value["records"] = json!(records);
                value["approvedPlan"] = json!(plan);
            }
            Some(value)
        })
        .collect()
}

/// Everything one task's gates left behind, for its full view: the gates a person answered
/// (`answered`) and the records its worker kept (`records`), each oldest first.
///
/// Asked for by the page when it opens the view rather than joined into `/api/state`: the
/// archive only grows, and reading all of it on every poll would cost more each day. The
/// records are here as well as on the task in `/api/state` because a finished task's are not
/// there, and a review is meant to stay readable after the work is done.
fn task_history(server: &Server, path: &str) -> Result<Value, String> {
    let id = history_id(path).ok_or("no such task")?;
    Ok(history_of(
        id,
        gate::list(&super::gate::answered_dir(&server.ctx)),
        gate::list(&super::gate::records_dir(&server.ctx)),
    ))
}

/// The task id in `/api/tasks/{id}/history`, when there is exactly one.
fn history_id(path: &str) -> Option<&str> {
    path.strip_prefix("/api/tasks/")
        .and_then(|rest| rest.strip_suffix("/history"))
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

fn history_of(id: &str, answered: Vec<gate::Gate>, records: Vec<gate::Gate>) -> Value {
    let mine = |g: &gate::Gate| g.task.as_deref() == Some(id);
    json!({
        "answered": answered.into_iter().filter(mine).collect::<Vec<_>>(),
        "records": records.into_iter().filter(mine).collect::<Vec<_>>(),
    })
}

/// The settings as `adj work` would read them now. Resolved on every poll rather than taken
/// from the ones the server started with, because `adj work` reads the config each time it
/// runs, and a limit changed under a running board would otherwise show one number while
/// dispatches are refused by another.
pub(super) fn settings_now(server: &Server) -> crate::config::Settings {
    crate::config::resolve_config(&server.ctx.repo.nwo)
        .map(|resolved| resolved.settings)
        .unwrap_or_else(|_| server.ctx.settings.clone())
}

fn branch_of(worktree: &str) -> Option<String> {
    let output = crate::repo::git(&["-C", worktree, "branch", "--show-current"], None).ok()?;
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

/// The three buttons a card has for the worker behind it: raise its tab, open its worktree in
/// the editor, close its tab. All through the same templates the commands use.
///
/// Only a worktree of this checkout is acted on. The path comes from the page, and these run
/// commands — `ide` a template of the person's own choosing — so a path the board did not
/// list is refused rather than handed on.
fn act_on_worktree(server: &Server, action: &str, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let worktree = input
        .get("worktree")
        .and_then(Value::as_str)
        .ok_or("a worktree is required")?;
    let known = crate::repo::linked_worktrees(&server.ctx.repo.main)?;
    if !known.iter().any(|w| w == worktree) {
        return Err(format!("not a worktree of this repository: {worktree}"));
    }
    let path = Path::new(worktree);
    let settings = settings_now(server);
    match action {
        "focus" => {
            let done = super::focus_worker(&settings, path, false)?;
            Ok(json!({
                "present": done.is_some(),
                "ran": done.as_ref().is_some_and(|d| d.ran),
            }))
        }
        "ide" => {
            let command = crate::ide::open_command(settings.ide.as_deref(), worktree)
                .ok_or("ide is not set: put your editor command in the config's ide key")?;
            crate::terminal::run_shell(&command)?;
            Ok(json!({ "ran": true }))
        }
        "close" => {
            let closed = super::close(Some(&server.ctx.repo.nwo), worktree, true, false)?;
            Ok(json!({ "closed": closed }))
        }
        other => Err(format!("no such action: {other}")),
    }
}

/// Raise the hub's tab: 「タブで話す」 on a gate the hub opened, which sits in the main
/// checkout where there is no worker to raise.
fn focus_hub(server: &Server) -> Result<Value, String> {
    let repo = &server.ctx.repo;
    let status = messaging::hub_status(&repo.slug, &repo.hub_name);
    let Some(pid) = status.pid.filter(|_| status.present) else {
        return Ok(json!({ "present": false, "ran": false }));
    };
    let settings = settings_now(server);
    let done = crate::terminal::focus(&settings.terminal, pid, &repo.hub_name, false)?;
    Ok(json!({ "present": true, "ran": done.ran }))
}

/// `/api/hubs/<id>/<action>` as its id and action, for the two actions there are. The id is
/// one path segment, percent-decoded — the page sends it through `encodeURIComponent`, and a
/// key may hold a `/`, a space or a letter that is not ASCII. The raw segment is checked for a
/// `/` first, so an encoded one names an id and a bare one is another route. An encoding that
/// is not UTF-8 is an error for the caller to say, not a different route.
fn hub_route(path: &str) -> Option<(Result<String, String>, &str)> {
    let (raw, action) = path.strip_prefix("/api/hubs/")?.split_once('/')?;
    (!raw.is_empty() && !raw.contains('/') && matches!(action, "start" | "stop" | "close"))
        .then(|| (decode_segment(raw), action))
}

/// `/api/sessions/<id>/terminal` as the session id, percent-decoded as `hub_route` does. Only
/// the path is looked at: whether the id names a session, and one that runs in tmux, is
/// answered once the socket is open, where the page can be told why not.
fn terminal_route(path: &str) -> Option<Result<String, String>> {
    let raw = path
        .strip_prefix("/api/sessions/")?
        .strip_suffix("/terminal")?;
    (!raw.is_empty() && !raw.contains('/')).then(|| decode_segment(raw))
}

/// `/api/sessions/<id>/link` as the session id, percent-decoded as `terminal_route` does.
fn link_route(path: &str) -> Option<Result<String, String>> {
    let raw = path.strip_prefix("/api/sessions/")?.strip_suffix("/link")?;
    (!raw.is_empty() && !raw.contains('/')).then(|| decode_segment(raw))
}

/// The scripts and styles the board terminal loads, as `(content type, body)`. Only the
/// resident server has them to give, and a page fetches them only when it opens a terminal.
fn vendor_asset(path: &str, resident: bool) -> Option<(&'static str, &'static str)> {
    match path {
        "/vendor/xterm.js" if resident => Some(("text/javascript; charset=utf-8", XTERM_JS)),
        "/vendor/xterm.css" if resident => Some(("text/css; charset=utf-8", XTERM_CSS)),
        _ => None,
    }
}

/// The tmux version when the board terminal can be offered at all: a unix machine with tmux 3.1
/// or later. Older tmux has neither `window-size latest` nor the hook that ends the terminal with
/// its window, so the board would follow tmux on to another agent's window when the target closes.
fn board_terminal_tmux() -> Option<(u32, u32)> {
    #[cfg(unix)]
    {
        crate::terminal::tmux_version().filter(|&v| v >= (3, 1))
    }
    #[cfg(not(unix))]
    {
        None
    }
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

/// Start, stop or close one of the repository's hubs from the board. `id` is the `hubs[].id` the
/// page was given, so the page can only name a hub this repository was found to have.
fn act_on_hub(server: &Server, path: &str, body: &[u8]) -> Result<Value, String> {
    let (id, action) = hub_route(path).ok_or("no such route")?;
    let id = id?;
    let id = id.as_str();
    let input: Value = match body.is_empty() {
        true => json!({}),
        false => serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?,
    };
    let repo = &server.ctx.repo;
    let hub = messaging::all_repo_hubs(repo)
        .into_iter()
        .find(|h| h.id == id)
        .ok_or_else(|| format!("no such hub: {id}"))?;
    let settings = settings_now(server);
    match action {
        "start" => {
            let start = match input.get("start").and_then(Value::as_str).unwrap_or("auto") {
                "auto" => super::HubStart::Auto,
                "resume" => super::HubStart::Resume,
                "new" => super::HubStart::New,
                other => return Err(format!("no such start: {other}")),
            };
            if hub.parent && hub.key.is_none() {
                return Err(
                    "the key of this hub is not known; start it with adj hub --hub <key>"
                        .to_string(),
                );
            }
            let ctx = super::Context {
                repo: repo.clone().addressed(hub.key.as_deref())?,
                resolved: server.ctx.resolved.clone(),
                settings,
            };
            match super::start_hub(&ctx, start)? {
                super::TabOutcome::Opened(done) => {
                    Ok(json!({ "started": true, "description": done.description }))
                }
                super::TabOutcome::AlreadyRunning(status) => {
                    Ok(json!({ "alreadyRunning": true, "pid": status.pid }))
                }
            }
        }
        "stop" | "close" => {
            let closing = action == "close";
            if closing {
                super::closable_check(repo, &hub)?;
            }
            // Addressed by the slug the hub was listed under: a hub whose key cannot be told
            // can still be stopped, and nothing here needs the key for it.
            let mut stopping = repo.clone();
            stopping.slug = hub.slug.clone();
            stopping.hub_name = hub.name.clone();
            let ctx = super::Context {
                repo: stopping,
                resolved: server.ctx.resolved.clone(),
                settings,
            };
            // A hub that will not stop is not closed: nothing is forgotten until it is gone.
            let was_running = super::stop_hub(&ctx)?;
            if closing {
                // `stop_hub` cleared a record naming the process it stopped; what is left
                // names none, unless a hub registered in the meantime, which stays.
                if !messaging::unregister_hub_if_unnamed(&hub.slug)? {
                    return Err(format!("{} changed while it was being closed", hub.name));
                }
                forget_board(&hub.slug)?;
                Ok(json!({ "closed": true, "wasRunning": was_running, "unread": hub.inbox_count }))
            } else {
                Ok(json!({ "stopped": true, "wasRunning": was_running }))
            }
        }
        other => Err(format!("no such action: {other}")),
    }
}

/// The task id in `/api/tasks/{id}/{what}`, when there is exactly one.
fn task_id_in<'a>(path: &'a str, what: &str) -> Option<&'a str> {
    path.strip_prefix("/api/tasks/")
        .and_then(|rest| rest.strip_suffix(&format!("/{what}")))
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

/// The review bots' comments on a Jules task's PR, for the side sheet to choose from. Asked
/// for when a person opens the list, not on every poll: it is a round trip to GitHub.
fn review_findings(server: &Server, path: &str) -> Result<Value, String> {
    let id = task_id_in(path, "findings").ok_or("no such task")?;
    Ok(json!({ "findings": super::jules_findings(&server.ctx, id)? }))
}

/// Post the chosen comments to the PR for Jules, in the name `gh` is signed in as.
fn relay_findings(server: &Server, path: &str, body: &[u8]) -> Result<Value, String> {
    let id = task_id_in(path, "relay").ok_or("no such task")?;
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let chosen = input
        .get("comments")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .map(super::JulesChosen::read)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    super::jules_relay(
        &server.ctx,
        id,
        &chosen,
        input.get("note").and_then(Value::as_str),
    )
}

/// The board's 「再取得」: read the task's issue again, on a click and never on a poll.
fn fetch_issue(server: &Server, path: &str) -> Result<Value, String> {
    let id = task_id_in(path, "issue").ok_or("no such task")?;
    let task = super::task::fetch_issue(&server.ctx, id)?;
    Ok(json!({ "task": task }))
}

/// The board's 「PR を確認」: the same pass as `adj task refresh`, whose answer the page shows
/// in its log before it redraws.
fn refresh_tasks(server: &Server) -> Result<Value, String> {
    let checked = super::task::refresh(&server.ctx)?;
    Ok(super::task::refresh_json(&checked))
}

fn nudge_hub(server: &Server) -> Result<Value, String> {
    let handed = super::task::nudge(&server.ctx)?;
    Ok(json!({ "handed": handed_json(Some(handed)) }))
}

fn answer_gate(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let decision = input
        .get("decision")
        .and_then(Value::as_str)
        .ok_or("a decision is required")?;
    if decision == "close" || decision == "dismiss" {
        let gate = super::gate::close(
            &server.ctx,
            id,
            input.get("comment").and_then(Value::as_str),
            false,
        )?;
        return Ok(json!({ "gate": gate, "closed": true }));
    }
    let (gate, told) = super::gate::answer(
        &server.ctx,
        id,
        decision,
        input.get("choice").and_then(Value::as_str),
        input.get("comment").and_then(Value::as_str),
    )?;
    Ok(json!({ "gate": gate, "present": told.present, "woken": told.woken }))
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

    #[test]
    fn forgetting_a_board_removes_only_that_slug() {
        let _sandbox = crate::testing::Sandbox::empty();
        std::fs::create_dir_all(boards_dir()).unwrap();
        for slug in ["acme-widget-a", "acme-widget-b"] {
            std::fs::write(boards_dir().join(format!("{slug}.json")), "{}").unwrap();
        }
        forget_board("acme-widget-a").unwrap();
        assert!(!boards_dir().join("acme-widget-a.json").exists());
        assert!(boards_dir().join("acme-widget-b.json").exists());
        // Nothing to forget is not an error.
        forget_board("acme-widget-a").unwrap();
    }

    #[test]
    fn the_page_pieces_join_into_one_document() {
        // A piece left out or put out of order shows here rather than as a blank page.
        assert!(UI_HTML.starts_with("<!DOCTYPE html>"));
        assert!(UI_HTML.trim_end().ends_with("</html>"));
        for tag in [
            "<style>",
            "</style>",
            "<script>",
            "</script>",
            "<body>",
            "</body>",
        ] {
            assert_eq!(UI_HTML.matches(tag).count(), 1, "{tag}");
        }
        let at = |tag: &str| UI_HTML.find(tag).unwrap();
        assert!(at("<style>") < at("</style>"));
        assert!(at("</style>") < at("<body>"));
        assert!(at("<script>") < at("</script>"));
        assert!(at("</script>") < at("</body>"));
    }

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

    #[test]
    fn a_board_path_names_one_slug() {
        assert_eq!(
            split_board_path("/b/acme-x-1/api/state"),
            Some(("acme-x-1", "/api/state"))
        );
        assert_eq!(split_board_path("/b/acme-x-1"), Some(("acme-x-1", "/")));
        assert_eq!(split_board_path("/b/acme-x-1/"), Some(("acme-x-1", "/")));
        for refused in [
            "/b//x",
            "/b/../x",
            "/b/ACME/",
            "/b/",
            "/api/state",
            "/board/x",
        ] {
            assert_eq!(split_board_path(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_board_url_carries_its_path() {
        assert_eq!(
            resident_board_url(4577, "acme-x-1", "tok"),
            "http://127.0.0.1:4577/b/acme-x-1/?token=tok"
        );
        assert_eq!(board_url(4577, "tok"), "http://127.0.0.1:4577/?token=tok");
    }

    #[test]
    fn the_resident_is_preferred_over_a_dedicated_board() {
        assert_eq!(prefer(Some(1), Some(2)), Some(Served::Resident(1)));
        assert_eq!(prefer(Some(1), None), Some(Served::Resident(1)));
        assert_eq!(prefer(None, Some(2)), Some(Served::Dedicated(2)));
        assert_eq!(prefer(None, None), None);
    }

    #[test]
    fn a_post_to_a_board_path_needs_the_same_origin() {
        let path = "/b/acme-x-1/api/tasks";
        let with = |origin: Option<&str>| {
            let mut headers = vec![("X-Adjutant-Token", "t")];
            headers.extend(origin.map(|o| ("Origin", o)));
            refuse("t", 4577, &request("POST", path, &headers))
        };
        assert_eq!(with(Some("http://127.0.0.1:4577")), None);
        assert_eq!(with(Some("http://localhost:4577")), None);
        assert_eq!(
            with(Some("http://127.0.0.1:9999")),
            Some((403, "cross-origin request"))
        );
        assert_eq!(
            with(Some("https://example.com")),
            Some((403, "cross-origin request"))
        );
        assert_eq!(with(None), Some((403, "no Origin header")));
    }

    #[test]
    fn worker_ids_are_unique_within_a_board() {
        let paths: Vec<String> = ["/w/a/app", "/w/b/app", "/w/c/main", "/w/d/solo"]
            .iter()
            .map(|p| p.to_string())
            .collect();
        let ids = worker_session_ids(&paths, true);
        // A name nobody else has keeps the id it always had.
        assert_eq!(ids[3], "worker-solo");
        // The rest gain a digest of their path, so two of one name are two ids, and
        // `worker-main` stays the main checkout's.
        assert!(ids[0].starts_with("worker-app-") && ids[1].starts_with("worker-app-"));
        assert!(ids[2].starts_with("worker-main-"));
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "{ids:?}");
        assert_eq!(ids, worker_session_ids(&paths, true));
        assert_eq!(ids[0].len(), "worker-app-".len() + 8);
    }

    #[test]
    fn a_worktree_called_main_keeps_its_id_unless_the_main_checkout_has_it() {
        let paths = vec!["/w/c/main".to_string(), "/w/d/solo".to_string()];
        assert_eq!(
            worker_session_ids(&paths, false),
            ["worker-main", "worker-solo"]
        );
        assert!(worker_session_ids(&paths, true)[0].starts_with("worker-main-"));
    }

    #[test]
    fn a_record_is_removed_only_while_it_names_the_stopped_server() {
        let old = (7, Some("Mon Jan  1 00:00:00 2024".to_string()));
        assert!(names_resident(
            Some(&old),
            7,
            Some("Mon Jan  1 00:00:00 2024")
        ));
        // A supervisor's restart has written another pid, or the same pid started later.
        assert!(!names_resident(
            Some(&old),
            8,
            Some("Mon Jan  1 00:00:00 2024")
        ));
        assert!(!names_resident(Some(&old), 7, Some("later")));
        assert!(!names_resident(None, 7, None));
    }

    #[test]
    fn the_server_log_is_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.log");
        drop(private_log(&path).unwrap());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // A log an older version made with the default mask is tightened, not trusted.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        drop(private_log(&path).unwrap());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn a_hub_route_names_an_id_and_an_action() {
        fn route(path: &str) -> Option<(String, &str)> {
            hub_route(path).map(|(id, action)| (id.unwrap(), action))
        }
        assert_eq!(route("/api/hubs/hub/start"), Some(("hub".into(), "start")));
        assert_eq!(
            route("/api/hubs/hub-wid-957/stop"),
            Some(("hub-wid-957".into(), "stop"))
        );
        // As `encodeURIComponent` sends them.
        assert_eq!(
            route("/api/hubs/hub-foo%2Fbar/start"),
            Some(("hub-foo/bar".into(), "start"))
        );
        assert_eq!(
            route("/api/hubs/hub-%E8%A6%AA%20%E3%82%AD%E3%83%BC/stop"),
            Some(("hub-親 キー".into(), "stop"))
        );
        assert_eq!(
            route("/api/hubs/hub-wid-957/close"),
            Some(("hub-wid-957".into(), "close"))
        );
        assert_eq!(route("/api/hubs/a+b/start"), Some(("a+b".into(), "start")));
        // An encoding that is wrong is the caller's mistake to be told, not another route.
        for bad in [
            "/api/hubs/hub-%zz/start",
            "/api/hubs/hub-%2/start",
            "/api/hubs/%FF/start",
        ] {
            assert!(hub_route(bad).is_some_and(|(id, _)| id.is_err()), "{bad}");
        }
        for refused in [
            "/api/hubs/hub/restart",
            "/api/hubs//start",
            "/api/hubs/a/b/start",
            "/api/hubs/hub",
            "/api/tasks/hub/start",
        ] {
            assert!(hub_route(refused).is_none(), "{refused}");
        }
    }

    #[test]
    fn a_worktree_s_branch_is_its_own_whatever_git_dir_names() {
        let sandbox = crate::testing::Sandbox::empty();
        let here = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        crate::testing::init_repo(here.path(), "mine");
        crate::testing::init_repo(other.path(), "theirs");

        let _var = crate::testing::EnvVar::set(&sandbox, "GIT_DIR", other.path().join(".git"));
        assert_eq!(
            branch_of(&here.path().to_string_lossy()).as_deref(),
            Some("mine")
        );
    }

    fn a_task(id: &str, status: task::Status) -> task::Task {
        task::Task {
            id: id.to_string(),
            kind: task::Kind::Start,
            title: id.to_string(),
            body: String::new(),
            issue_url: None,
            done_when: task::DoneWhen::Pr,
            stop_at: task::StopAt::Plan,
            executor: task::Executor::Worker,
            base: None,
            parent: None,
            worktree_name: None,
            auto_start: true,
            order: 0,
            status,
            worktree: None,
            issue: None,
            pr: None,
            jules_session: None,
            jules_by: None,
            relayed: Vec::new(),
            announced: Vec::new(),
            relay_rounds: 0,
            note: None,
            instruction: None,
            gate_answered_at: None,
            issue_snapshot: None,
            created_at: "20260922T000000Z".to_string(),
            updated_at: "20260922T000000Z".to_string(),
        }
    }

    fn a_gate(id: &str, kind: gate::Kind, task: &str) -> gate::Gate {
        serde_json::from_value(json!({
            "id": id,
            "kind": kind,
            "worktree": "/tmp/wt",
            "task": task,
            "title": id,
            "openedAt": "20260922T010000Z",
        }))
        .unwrap()
    }

    #[test]
    fn a_live_task_carries_its_records_and_the_plan_approved_last() {
        let mut record = a_gate("r1", gate::Kind::Diff, "t1");
        record.wait = false;
        let others = a_gate("r2", gate::Kind::Verify, "t2");
        let answered = |id: &str, decision: &str, at: &str| {
            let mut g = a_gate(id, gate::Kind::Plan, "t1");
            g.decision = Some(decision.to_string());
            g.answered_at = Some(at.to_string());
            g
        };
        let tasks = with_records(
            vec![a_task("t1", task::Status::Dispatched)],
            vec![record, others],
            vec![
                answered("p-old", "approve", "20260922T020000Z"),
                answered("p-new", "approve", "20260922T040000Z"),
                // Sent back later still: not what was approved.
                answered("p-sent-back", "changes", "20260922T050000Z"),
            ],
        );
        let ids: Vec<&str> = tasks[0]["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["r1"]);
        assert_eq!(tasks[0]["approvedPlan"]["id"], "p-new");
        assert_eq!(tasks[0]["approvedPlan"]["answeredAt"], "20260922T040000Z");
    }

    #[test]
    fn a_task_with_no_approved_plan_says_so_and_a_finished_one_carries_nothing() {
        let tasks = with_records(
            vec![
                a_task("t1", task::Status::Queued),
                a_task("t2", task::Status::Done),
            ],
            vec![a_gate("r2", gate::Kind::Diff, "t2")],
            Vec::new(),
        );
        assert_eq!(tasks[0]["records"], json!([]));
        assert!(tasks[0]["approvedPlan"].is_null());
        assert!(tasks[1].get("records").is_none(), "{}", tasks[1]);
    }

    #[test]
    fn a_task_s_history_is_its_own_answered_gates_and_records_of_every_kind() {
        let mut diff = a_gate("20260922T010000Z-diff", gate::Kind::Diff, "t1");
        diff.decision = Some("changes".to_string());
        let mut plan = a_gate("20260922T000000Z-plan", gate::Kind::Plan, "t1");
        plan.opened_at = "20260922T000000Z".to_string();
        let theirs = a_gate("20260922T020000Z-verify", gate::Kind::Verify, "t2");
        let mut record = a_gate("20260922T030000Z-verify-record", gate::Kind::Verify, "t1");
        record.wait = false;

        let history = history_of("t1", vec![plan, diff, theirs.clone()], vec![record, theirs]);
        let ids = |key: &str| -> Vec<String> {
            history[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|g| g["id"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(
            ids("answered"),
            ["20260922T000000Z-plan", "20260922T010000Z-diff"]
        );
        assert_eq!(ids("records"), ["20260922T030000Z-verify-record"]);
    }

    #[test]
    fn a_history_path_names_one_task() {
        assert_eq!(history_id("/api/tasks/t1/history"), Some("t1"));
        for bad in [
            "/api/tasks//history",
            "/api/tasks/a/b/history",
            "/api/tasks/t1",
        ] {
            assert_eq!(history_id(bad), None, "{bad}");
        }
    }

    const TOKEN: &str = "s3cret";
    const PORT: u16 = 4577;

    #[test]
    fn a_taken_port_falls_back_to_a_free_one() {
        let taken = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let second = bind_preferring(port).unwrap();
        let got = second.local_addr().unwrap().port();
        assert_ne!(got, port);
        assert_ne!(got, 0);
    }

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

    fn upgrade_headers(origin: Option<&str>) -> Vec<(&str, &str)> {
        let mut headers = vec![
            ("Connection", "Upgrade"),
            ("Upgrade", "websocket"),
            ("Sec-WebSocket-Version", "13"),
            ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
        ];
        headers.extend(origin.map(|o| ("Origin", o)));
        headers
    }

    #[test]
    fn a_handshake_with_no_origin_is_refused_even_with_the_token() {
        let req = request(
            "GET",
            "/b/x/api/sessions/hub/terminal?token=s3cret",
            &upgrade_headers(None),
        );
        assert_eq!(refuse(TOKEN, PORT, &req), Some((403, "no Origin header")));
    }

    #[test]
    fn a_handshake_from_another_site_is_refused() {
        for origin in [
            "http://evil.example",
            "http://127.0.0.1:4578",
            "https://127.0.0.1:4577",
            "null",
        ] {
            let req = request(
                "GET",
                "/b/x/api/sessions/hub/terminal?token=s3cret",
                &upgrade_headers(Some(origin)),
            );
            assert_eq!(
                refuse(TOKEN, PORT, &req),
                Some((403, "cross-origin request")),
                "{origin}"
            );
        }
    }

    #[test]
    fn a_handshake_needs_the_token_however_good_its_origin() {
        for path in [
            "/b/x/api/sessions/hub/terminal",
            "/b/x/api/sessions/hub/terminal?token=guess",
        ] {
            let req = request("GET", path, &upgrade_headers(Some("http://127.0.0.1:4577")));
            assert_eq!(
                refuse(TOKEN, PORT, &req),
                Some((403, "bad or missing token")),
                "{path}"
            );
        }
    }

    /// A browser cannot set headers on a WebSocket, so the token rides in the URL.
    #[test]
    fn a_handshake_from_the_boards_own_origin_may_carry_the_token_in_the_url() {
        for origin in ["http://127.0.0.1:4577", "http://localhost:4577"] {
            let req = request(
                "GET",
                "/b/x/api/sessions/hub/terminal?token=s3cret",
                &upgrade_headers(Some(origin)),
            );
            assert_eq!(refuse(TOKEN, PORT, &req), None, "{origin}");
        }
    }

    /// The origin rule is for handshakes: the page and its polling GETs are as they were.
    #[test]
    fn a_plain_get_is_still_asked_for_no_origin() {
        let req = request("GET", "/b/x/api/state?token=s3cret", &[]);
        assert_eq!(refuse(TOKEN, PORT, &req), None);
    }

    #[test]
    fn a_terminal_route_names_a_session_id() {
        fn id(path: &str) -> Option<String> {
            terminal_route(path).map(|id| id.unwrap())
        }
        assert_eq!(id("/api/sessions/hub/terminal"), Some("hub".into()));
        assert_eq!(
            id("/api/sessions/worker-widget/terminal"),
            Some("worker-widget".into())
        );
        // As `encodeURIComponent` sends a hub id with a slash or a space in it.
        assert_eq!(
            id("/api/sessions/hub-foo%2Fbar/terminal"),
            Some("hub-foo/bar".into())
        );
        assert_eq!(
            id("/api/sessions/hub-%E8%A6%AA%20%E3%82%AD%E3%83%BC/terminal"),
            Some("hub-親 キー".into())
        );
        for bad in [
            "/api/sessions/hub-%zz/terminal",
            "/api/sessions/%FF/terminal",
        ] {
            assert!(terminal_route(bad).is_some_and(|id| id.is_err()), "{bad}");
        }
        for other in [
            "/api/sessions//terminal",
            "/api/sessions/a/b/terminal",
            "/api/sessions/hub",
            "/api/sessions/hub/terminal/x",
            "/api/hubs/hub/terminal",
            "/terminal",
            "/",
        ] {
            assert!(terminal_route(other).is_none(), "{other}");
        }
    }

    #[test]
    fn a_link_route_names_a_session_id() {
        fn id(path: &str) -> Option<String> {
            link_route(path).map(|id| id.unwrap())
        }
        assert_eq!(id("/api/sessions/worker-x/link"), Some("worker-x".into()));
        // As `encodeURIComponent` sends an id with a slash or a space in it.
        assert_eq!(
            id("/api/sessions/worker-a%2Fb%20c/link"),
            Some("worker-a/b c".into())
        );
        assert!(link_route("/api/sessions/%FF/link").is_some_and(|id| id.is_err()));
        for other in [
            "/api/sessions",
            "/api/sessions//link",
            "/api/sessions/a/b/link",
            "/api/sessions/worker-x/terminal",
            "/api/sessions/worker-x/link/x",
            "/api/tasks/x/link",
        ] {
            assert!(link_route(other).is_none(), "{other}");
        }
    }

    #[test]
    fn the_terminal_assets_are_only_served_by_the_resident_server() {
        assert!(vendor_asset("/vendor/xterm.js", false).is_none());
        assert!(vendor_asset("/vendor/xterm.css", false).is_none());
        let (kind, js) = vendor_asset("/vendor/xterm.js", true).unwrap();
        assert!(kind.starts_with("text/javascript"));
        assert!(js.starts_with("/*! xterm.js - MIT License"));
        assert!(js.contains("Permission is hereby granted"));
        // The library and both addons come in the one script, each defining its global.
        for global in ["Terminal", "FitAddon", "Unicode11Addon"] {
            assert!(js.contains(global), "{global}");
        }
        assert!(!js.contains("sourceMappingURL"));
        let (kind, css) = vendor_asset("/vendor/xterm.css", true).unwrap();
        assert!(kind.starts_with("text/css"));
        assert!(css.starts_with("/*! xterm.js - MIT License"));
        assert!(css.contains(".xterm"));
        assert!(vendor_asset("/vendor/other.js", true).is_none());
    }

    /// The library is fetched by a page that opens a terminal, not carried by every page.
    #[test]
    fn the_page_does_not_carry_the_terminal_library() {
        assert!(!UI_HTML.contains("Permission is hereby granted"));
    }
}
