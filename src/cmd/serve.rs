//! `adj serve` — the board, served to a browser on this machine.
//!
//! The dashboard is not a second coordination system. Every button on it ends in something
//! this binary could already do: a task handed over becomes a `request` in the hub's inbox
//! and a poke on its tab, exactly as `adj send` would. What the server adds is a view of
//! state that until now could only be read one `adj` invocation at a time, and a place to
//! put the questions a worker used to have to ask into a tab nobody was watching.
//!
//! It holds one clock, and only in the resident server: the poll that keeps the cards' pull
//! requests up to date (`pr_poll`). A board served by itself, or by a hub, has none: there a
//! request arrives because a person clicked, and that is the only thing that moves. `/api/state`
//! never asks GitHub on either, since the page polls it every couple of seconds.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::gate;
use crate::http::{self, Request};
use crate::messaging;
use crate::repo::Worktree;
use crate::runner;
use crate::session;
use crate::task;

mod assets;
mod auth;
mod daemon;
mod index;
mod registry;
mod resident;

use assets::{UI_HTML, vendor_asset};
use auth::{refuse, token};
use daemon::open_browser;
pub use daemon::{resident_running, server_restart, server_start, server_status, server_stop};
pub(super) use registry::forget_board;
pub use registry::{board_json, dashboards_running, note_board, running};
use registry::{board_url, record, resident_url};

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
    /// The titles of the parent tasks the hubs are named after: a cache of the tracker's, kept
    /// on disk, and read from a thread of its own.
    hub_titles: Arc<super::HubTitles>,
    /// The last line each session's pane showed, for the pages that ask for it (`?lines=1`).
    /// A cache: the pane is the answer.
    last_lines: Arc<LastLines>,
    /// What tmux this machine has, when the board may open terminals on it: only the resident
    /// server serves one, and only where `tmux -V` answered when it started.
    pub(super) tmux: Option<(u32, u32)>,
    /// How many board terminals are open across every board, which is what is capped.
    pub(super) terminals: Arc<AtomicUsize>,
    /// The resident server's PR poll, whose health the page shows. `None` on a board that is
    /// served by itself: nothing polls there, and the page says nothing about it.
    pr_poll: Option<Arc<super::PrPoll>>,
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
                hub_titles: Arc::default(),
                last_lines: Arc::default(),
                tmux: None,
                terminals: Arc::default(),
                pr_poll: None,
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

/// The paths that are the page itself. One document serves them all, and the page reads its
/// own address to know which view to draw.
fn is_page_path(path: &str) -> bool {
    matches!(path, "/" | "/index.html" | "/review")
}

fn route(server: &Server, req: &Request, out: &mut impl Write) -> std::io::Result<()> {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", path) if is_page_path(path) => http::html(out, UI_HTML),
        ("GET", "/api/state") => http::json(
            out,
            200,
            &state(
                server,
                req.param("sessions") != Some("0"),
                req.param("lines") == Some("1"),
            )
            .to_string(),
        ),
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
                .and_then(|id| session_git(server, &id));
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
        ("POST", "/api/sessions") => reply(out, super::session::start_request(server, &req.body)),
        ("POST", path) if session_route_for(path, "link").is_some() => {
            let result = session_route_for(path, "link")
                .unwrap_or_else(|| Err("no such route".to_string()))
                .and_then(|id| super::session::link(server, &id, &req.body));
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
                "resume" => super::board_actions::resume(server, &id, &req.body),
                "restart" => super::board_actions::restart(server, &id, &req.body),
                "open" => super::board_actions::open(server, &id),
                _ => super::board_actions::cleanup(server, &id, &req.body),
            });
            reply(out, result)
        }
        ("POST", "/api/hubs") if server.resident => reply(
            out,
            super::board_actions::start_parent_hub(server, &req.body),
        ),
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
    views: &mut HashMap<PathBuf, TmuxView>,
    pid: Option<u32>,
) -> session::SessionTerminal {
    if let Some(recorded) = record
        .and_then(|r| r.get("terminal"))
        .and_then(|t| serde_json::from_value::<session::SessionTerminal>(t.clone()).ok())
    {
        return recorded;
    }
    let tmux = terminal_settings.spawn.is_none() && terminal_settings.is_tmux();
    // Asked of tmux only here: a session whose record says where it runs needs no look at the
    // settings' own server.
    let pane = pid.filter(|_| tmux).and_then(|p| {
        let view = tmux_view(views, terminal_settings.tmux_socket());
        crate::terminal::find_matching_pane(&view.panes, Some(p), None)
    });
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

/// The title of task `id` in the task directory of the hub `slug`, if there is such a record.
/// Read here rather than taken from the page's own task list so that a worker under another
/// hub names its task as well.
fn linked_task_title(state_dir: &Path, slug: &str, id: &str) -> Option<String> {
    let title = task::load(&task::dir(state_dir, slug), id).ok()?.title;
    Some(title.trim().to_string()).filter(|t| !t.is_empty())
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

/// `with_sessions` false leaves `sessions` empty: the page that merges several boards has no
/// use for them, and listing them is the dearest part of a poll. `with_lines` adds each
/// session's last line of output (`lastLine`), which reads its tmux pane: only the page that
/// shows it asks.
fn state(server: &Server, with_sessions: bool, with_lines: bool) -> Value {
    // Before the gates are read: a gate whose worker has moved on is closed here rather than
    // by a timer, since nothing in the server polls on one.
    let _ = super::gate::close_resumed(&server.ctx);
    let repo = &server.ctx.repo;
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
    // One `git worktree list` and one `ps` serve every question below, so what a poll costs
    // does not grow with the number of worktrees. The `ps` is only run if a record names a pid.
    let processes = messaging::ProcessTable::snapshot();
    // The board shows what it can; `adj work` is the one that refuses on a failed listing.
    let listed = crate::repo::worktrees(&repo.main).unwrap_or_default();
    let (main_branch, linked) = split_main(&repo.main, listed);
    let linked_paths: Vec<String> = linked.iter().map(|w| w.path.clone()).collect();
    // Counted as `adj work` counts, main checkout included, though it is not listed below.
    let mut busy = usize::from(messaging::holds_worker_slot_with(
        &processes,
        Path::new(&repo.main),
        now,
    ));
    let mut hubs = messaging::all_repo_hubs_among_with(&processes, repo, &linked_paths);
    let slugs: Vec<String> = hubs.iter().map(|h| h.slug.clone()).collect();
    for h in &mut hubs {
        h.title = server.hub_titles.look(&server.ctx, h, &slugs);
    }
    server
        .hub_titles
        .keep_only(&slugs.iter().cloned().collect());
    // The repository's own hub is one of `hubs`; asked separately only if it is not there.
    let hub = hubs
        .iter()
        .find(|h| h.slug == repo.slug)
        .map(|h| h.state.clone())
        .unwrap_or_else(|| {
            let status = messaging::hub_status_with(&processes, &repo.slug, &repo.hub_name);
            session::RepoHubState {
                present: status.present,
                stale: status.stale,
                pid: status.pid,
                started_at: status.started_at,
            }
        });
    let mut workers_data = Vec::with_capacity(linked.len());
    let mut workers: Vec<Value> = Vec::with_capacity(linked.len());
    for Worktree { path, branch } in &linked {
        let status = messaging::worker_status_with(&processes, Path::new(path));
        // A present worker holds a slot without asking `ps` again; the rest are asked
        // the way `adj work` asks, so the header and the refusal cannot disagree.
        if status.present || messaging::holds_worker_slot_with(&processes, Path::new(path), now) {
            busy += 1;
        }
        let branch = branch.clone();
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

    let sessions = if with_sessions {
        sessions_of(
            server,
            &settings,
            &hubs,
            &linked_paths,
            Listing {
                processes: &processes,
                main_branch,
                with_lines,
            },
            None,
            |index, _| workers_data[index].clone(),
        )
    } else {
        Vec::new()
    };

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
        // Whether the PR poll is running and whether it is failing, for the one line the page
        // shows when it is. `null` where nothing polls. Read from memory: this is polled every
        // couple of seconds and must not reach GitHub.
        "prPoll": server.pr_poll.as_ref().map(|p| p.health_json()),
        // Whether the board may start a hub: only where the settings mean a tmux window.
        "hubStart": { "available": super::hub_startable(&settings.terminal) },
        // Whether the board can open a terminal on a session that runs in tmux: the resident
        // server, on a machine that has tmux. Which sessions is for the page to read from
        // `sessions[].terminal` and `present`.
        "boardTerminal": { "available": server.resident && server.tmux.is_some() },
        // Whether the board can open a session in the person's own terminal, and through what:
        // `terminal.attach` when it is set, iTerm2 where that is installed.
        "sessionOpen": super::board_actions::open_state(server, &settings),
        // Whether the board can resume a stopped worker, so the page offers it only where it
        // can work, and says why not where it cannot.
        "sessionResume": super::board_actions::resume_state(&settings),
        // The same for restarting a running hub on its conversation, which needs the hub's own
        // resume line rather than the worker's.
        "hubResume": super::board_actions::hub_resume_state(&settings),
        // The command line a hub runs, as configured: the server sends the template with its
        // placeholders in place, and the Sessions sidebar fills in only `{name}` to show it.
        "hubRunner": settings
            .hub_runner
            .as_deref()
            .unwrap_or(runner::DEFAULT_HUB_RUNNER),
        // The agent a session started from the board runs, which is the only one its dialog offers.
        "sessionStart": {
            "agent": runner::agent_from_runner(
                settings.agent_runner.as_deref().unwrap_or(runner::DEFAULT_AGENT_RUNNER),
            ),
        },
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

/// A tmux socket as a lookup key: the path of the server it names, so that no setting, a bare
/// name and the path a record kept for the same server are one key and one pair of `list-*`
/// calls. The directory is resolved when it can be, because `/tmp` is `/private/tmp` on a Mac.
fn socket_key(socket: Option<&str>) -> PathBuf {
    #[cfg(unix)]
    let uid = unsafe { libc::getuid() };
    #[cfg(not(unix))]
    let uid = 0;
    socket_key_in(
        socket,
        std::env::var("TMUX").ok().as_deref(),
        std::env::var("TMUX_TMPDIR").ok().as_deref(),
        uid,
    )
}

/// `socket_key` with the environment it reads handed in.
fn socket_key_in(
    socket: Option<&str>,
    tmux_env: Option<&str>,
    tmpdir: Option<&str>,
    uid: u32,
) -> PathBuf {
    let path = crate::terminal::tmux_socket_path(socket, tmux_env, tmpdir, uid);
    match (
        path.parent().and_then(|dir| dir.canonicalize().ok()),
        path.file_name(),
    ) {
        (Some(dir), Some(leaf)) => dir.join(leaf),
        _ => path,
    }
}

/// What one tmux server said about its panes and clients in one poll.
struct TmuxView {
    panes: Vec<crate::terminal::TmuxPane>,
    /// Clients attached to each window, by window id.
    attached: HashMap<String, u32>,
}

impl TmuxView {
    fn look(socket: Option<&str>) -> Self {
        let panes = crate::terminal::list_tmux_panes(socket).unwrap_or_default();
        let clients = crate::terminal::list_tmux_clients(socket);
        let attached = crate::terminal::attached_counts(&panes, &clients);
        TmuxView { panes, attached }
    }
}

/// What the tmux server on `socket` says, asked the first time it is needed and kept after.
/// The first spelling of a server's socket is the one tmux is run with.
fn tmux_view<'a>(views: &'a mut HashMap<PathBuf, TmuxView>, socket: Option<&str>) -> &'a TmuxView {
    views
        .entry(socket_key(socket))
        .or_insert_with(|| TmuxView::look(socket))
}

/// When a session's tmux window last had activity and how many clients are on it, from the
/// server its own record names — which is not always the settings' one. Both `None` for a
/// session that is not in tmux or whose window is not there.
fn tmux_activity(
    views: &mut HashMap<PathBuf, TmuxView>,
    terminal: &session::SessionTerminal,
) -> (Option<i64>, Option<u32>) {
    let Some(window) = terminal
        .window
        .as_deref()
        .filter(|_| terminal.backend == "tmux")
    else {
        return (None, None);
    };
    let view = tmux_view(views, terminal.socket.as_deref());
    let Some(pane) = view.panes.iter().find(|p| p.window_id == window) else {
        return (None, None);
    };
    (pane.window_activity, view.attached.get(window).copied())
}

/// The pane whose screen stands for a session: the one its record names, else the first of its
/// window's. `None` for a session that is not in tmux or whose window is not there.
fn tmux_pane_of(
    views: &mut HashMap<PathBuf, TmuxView>,
    terminal: &session::SessionTerminal,
) -> Option<String> {
    let window = terminal
        .window
        .as_deref()
        .filter(|_| terminal.backend == "tmux")?;
    let view = tmux_view(views, terminal.socket.as_deref());
    let first = view.panes.iter().find(|p| p.window_id == window)?;
    Some(
        terminal
            .pane
            .clone()
            .unwrap_or_else(|| first.pane_id.clone()),
    )
}

/// How soon a pane's screen is read again. A busy agent moves its window's activity on every
/// poll, and reading its screen that often would be most of what a poll costs.
const LAST_LINE_MIN_AGE: Duration = Duration::from_secs(5);

/// A pane's last line, with the window activity it was read at and when.
struct LastRead {
    activity: Option<i64>,
    at: Instant,
    /// The epoch second it was read in, against a window activity that has one-second
    /// resolution: activity in the same second may have come after the read.
    secs: i64,
    line: Option<String>,
}

/// The last line of output of each pane a page has asked for, by pane.
#[derive(Default)]
struct LastLines {
    read: Mutex<HashMap<String, LastRead>>,
}

impl LastLines {
    /// The line of `key`, which `read` produces only when the window has had activity since
    /// the last read and that was at least `LAST_LINE_MIN_AGE` ago.
    fn look(
        &self,
        key: &str,
        activity: Option<i64>,
        now: Instant,
        secs: i64,
        read: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        if let Some(last) = self.read.lock().ok()?.get(key) {
            let recent = now.duration_since(last.at) < LAST_LINE_MIN_AGE;
            // Unknown activity says nothing of whether the pane moved, and a read that found
            // nothing may only have been early: both are read again once the age is up.
            let unchanged = last.line.is_some()
                && activity.is_some_and(|a| last.activity == Some(a) && last.secs > a);
            if recent || unchanged {
                return last.line.clone();
            }
        }
        // Not under the lock: reading the pane runs a command.
        let line = read();
        if let Ok(mut all) = self.read.lock() {
            all.insert(
                key.to_string(),
                LastRead {
                    activity,
                    at: now,
                    secs,
                    line: line.clone(),
                },
            );
        }
        line
    }

    /// Forgets the panes that are no longer listed.
    fn keep_only(&self, keys: &HashSet<String>) {
        if let Ok(mut all) = self.read.lock() {
            all.retain(|key, _| keys.contains(key));
        }
    }
}

fn session_waiting(
    hub: &session::RepoHub,
    open: &[&gate::Gate],
) -> Option<session::SessionWaiting> {
    let first = open.first()?;
    Some(session::SessionWaiting {
        id: first.id.clone(),
        kind: first.kind.as_str().to_string(),
        hub: hub.id.clone(),
        slug: hub.slug.clone(),
        title: Some(first.title.clone()).filter(|t| !t.is_empty()),
        opened_at: first.opened_at.clone(),
        count: open.len(),
        options: if first.options.is_empty() {
            first.kind.default_options()
        } else {
            first.options.clone()
        },
        choices: first
            .choices
            .iter()
            .map(|c| session::WaitingChoice {
                id: c.id.clone(),
                label: c.label.clone(),
            })
            .collect(),
        focus: first
            .focus
            .as_deref()
            .map(|f| cut_chars(f, WAITING_FOCUS_CHARS))
            .filter(|f| !f.is_empty()),
    })
}

/// How much of a gate's focus the Sessions banner carries.
const WAITING_FOCUS_CHARS: usize = 400;

/// `text` cut to at most `max` characters, on a character boundary, with an ellipsis when cut.
fn cut_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text.to_string(),
    }
}

/// One hub's open gates, and what could show their workers moved on, read once per poll.
struct HubGates {
    open: Vec<gate::Gate>,
    signals: Vec<gate::Gate>,
}

impl HubGates {
    fn read(state_dir: &Path, slug: &str) -> Self {
        let dir = gate::dir(state_dir, slug);
        let open = gate::list(&dir);
        // Left unread when nothing waits on a worker: the archives only grow.
        let signals = if open.iter().any(|g| g.wait && !g.answered_by_hub()) {
            gate::resume_signals(
                &open,
                &dir,
                &gate::records_dir(state_dir, slug),
                &gate::answered_dir(state_dir, slug),
            )
        } else {
            Vec::new()
        };
        HubGates { open, signals }
    }
}

/// The gates of the hubs a poll reaches, each hub's read the first time one of its sessions
/// asks.
struct GateCache {
    state_dir: PathBuf,
    read: HashMap<String, HubGates>,
}

impl GateCache {
    fn of(&mut self, slug: &str) -> &HubGates {
        self.read
            .entry(slug.to_string())
            .or_insert_with(|| HubGates::read(&self.state_dir, slug))
    }
}

/// The gate a worker is waiting to have answered: the oldest still open in its hub's gate
/// directory that it opened from `worktree` and has not moved on from. "Moved on" is judged as
/// `close_resumed` judges it, from the same signals, but only reads: a hub's directory is
/// closed by that hub's board.
fn waiting_worker(
    hub: &session::RepoHub,
    gates: &HubGates,
    worktree: &str,
    started: Option<&str>,
    phase_at: Option<i64>,
) -> Option<session::SessionWaiting> {
    let phase_at = phase_at.map(messaging::utc_stamp);
    let open: Vec<&gate::Gate> = gates
        .open
        .iter()
        .filter(|g| g.worktree == worktree && g.wait && !g.answered_by_hub())
        .filter(|g| {
            let later = gates
                .signals
                .iter()
                .filter(|s| s.worktree == g.worktree && s.id != g.id && s.opened_at > g.opened_at)
                .map(|s| s.opened_at.as_str())
                .min();
            gate::resumed_at(g, started, phase_at.as_deref(), later).is_none()
        })
        .collect();
    session_waiting(hub, &open)
}

/// What a hub is waiting on: the gates it opened for a person to answer.
fn waiting_hub(hub: &session::RepoHub, gates: &[gate::Gate]) -> Option<session::SessionWaiting> {
    let open: Vec<&gate::Gate> = gates
        .iter()
        .filter(|g| g.wait && g.answered_by_hub())
        .collect();
    session_waiting(hub, &open)
}

/// What one poll has already asked of the system, so `sessions_of` does not ask again: the
/// process table, and the branch the main checkout's listing entry names.
struct Listing<'a> {
    processes: &'a messaging::ProcessTable,
    main_branch: Option<String>,
    /// Whether each session carries the last line of its pane: reading it runs a command per
    /// session, so only the page that shows it asks.
    with_lines: bool,
}

/// The sessions this board lists, hubs first and then the workers of `linked_paths`, as the
/// page reads them and as a board terminal resolves an id. `worker_data` is asked for a
/// worker's status and branch by its place in `linked_paths`.
///
/// With `only`, the one session of that id: the others are skipped before anything is read or
/// run for them, so that the one and the whole list are the same code and cannot drift.
fn sessions_of(
    server: &Server,
    settings: &crate::config::Settings,
    hubs: &[session::RepoHub],
    linked_paths: &[String],
    listing: Listing<'_>,
    only: Option<&str>,
    mut worker_data: impl FnMut(usize, &str) -> (messaging::WorkerStatus, Option<String>),
) -> Vec<session::Session> {
    let repo = &server.ctx.repo;
    let terminal_settings = &settings.terminal;
    let skipped = |id: &str| only.is_some_and(|wanted| wanted != id);
    // What tmux says, asked once per socket per poll and only for a socket a listed session
    // needs: its record's own, or the settings' when the record says none.
    let mut views: HashMap<PathBuf, TmuxView> = HashMap::new();
    // Read once per poll, for the hubs of the sessions listed, so a worker under a parent-task
    // hub shows its gate on the repository board too. Read-only — closing a resumed gate stays
    // with the board that owns the hub's directory.
    let mut gates = GateCache {
        state_dir: messaging::state_dir(),
        read: HashMap::new(),
    };
    let worker_waiting = |gates: &mut GateCache,
                          hub_id: &str,
                          worktree: &str,
                          started: Option<&str>,
                          phase_at: Option<i64>| {
        let hub = hubs.iter().find(|h| h.id == hub_id)?;
        waiting_worker(hub, gates.of(&hub.slug), worktree, started, phase_at)
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

    let Listing {
        processes,
        main_branch,
        with_lines,
    } = listing;
    // The last line of a session's pane, when the page asked for it: read for a session that
    // runs in tmux, and cached by pane (see `LastLines`).
    let mut screens: HashSet<String> = HashSet::new();
    let mut last_line = |views: &mut HashMap<PathBuf, TmuxView>,
                         terminal: &session::SessionTerminal,
                         agent: &str,
                         present: bool,
                         activity: Option<i64>| {
        if !with_lines || !present {
            return None;
        }
        let pane = tmux_pane_of(views, terminal)?;
        let key = format!(
            "{}\t{pane}",
            socket_key(terminal.socket.as_deref()).display()
        );
        screens.insert(key.clone());
        let agent = crate::prompts::Agent::parse(agent).unwrap_or(crate::prompts::Agent::Generic);
        server.last_lines.look(
            &key,
            activity,
            Instant::now(),
            messaging::now_secs(),
            || {
                let screen = crate::terminal::look_at_tmux_pane(terminal.socket.as_deref(), &pane)?;
                crate::terminal::last_output_line(agent, &screen)
            },
        )
    };

    let main_branch = || main_branch.clone();
    let mut sessions: Vec<session::Session> = Vec::new();

    // 1. Hub sessions from hubs
    for h in hubs.iter().filter(|h| !skipped(&h.id)) {
        let record = messaging::read_json(&messaging::hub_record_path(&h.slug));
        let terminal =
            session_terminal(record.as_ref(), terminal_settings, &mut views, h.state.pid);
        let (last_activity_at, attached) = tmux_activity(&mut views, &terminal);
        let line = last_line(
            &mut views,
            &terminal,
            &hub_agent,
            h.state.present,
            last_activity_at,
        );

        sessions.push(session::Session {
            id: h.id.clone(),
            conversation: messaging::hub_session(&h.slug).map(|s| s.session_id),
            kind: "hub".to_string(),
            agent: hub_agent.clone(),
            terminal,
            hub: None,
            key: h.key.clone(),
            worktree: repo.main.clone(),
            branch: main_branch(),
            task: None,
            title: Some(h.name.clone()),
            task_title: None,
            present: h.state.present,
            stale: h.state.stale,
            pid: h.state.pid,
            started_at: h.state.started_at.clone(),
            phase: None,
            phase_at: None,
            phases: Vec::new(),
            last_activity_at,
            last_line: line,
            attached,
            waiting: waiting_hub(h, &gates.of(&h.slug).open),
        });
    }

    // 2. Worker sessions from linked worktrees
    // Whether the main checkout is listed below as `worker-main`, which a worktree of that
    // name would otherwise collide with.
    let main_listed = messaging::read_json(&messaging::worker_record_path(Path::new(&repo.main)))
        .is_some()
        || messaging::worker_session(Path::new(&repo.main)).is_some();
    let worker_ids = worker_session_ids(linked_paths, main_listed);
    for (index, (path, id)) in linked_paths.iter().zip(worker_ids).enumerate() {
        if skipped(&id) {
            continue;
        }
        let (status, branch) = worker_data(index, path);
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
            &mut views,
            status.pid,
        );
        let (last_activity_at, attached) = tmux_activity(&mut views, &terminal);
        let line = last_line(
            &mut views,
            &terminal,
            &worker_agent,
            status.present,
            last_activity_at,
        );
        let waiting = worker_waiting(
            &mut gates,
            &parent_hub,
            path,
            started_at.as_deref(),
            status.phase_at,
        );

        let saved_title = saved_session.and_then(|s| s.title);
        let task_id = messaging::worker_task(wt_path);

        let title = status.title.or(saved_title);
        let task_title = task_id.as_deref().and_then(|id| {
            let slug =
                crate::repo::slug_for(&repo.nwo, messaging::worker_hub_key(wt_path).as_deref());
            linked_task_title(&gates.state_dir, &slug, id)
        });

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
            task_title,
            present: status.present,
            stale: status.stale,
            pid: status.pid,
            started_at,
            phase: status.phase,
            phase_at: status.phase_at,
            phases: status.phases,
            last_activity_at,
            last_line: line,
            attached,
            waiting,
        });
    }

    // Also check worker in main checkout if one exists
    let main_record_path = messaging::worker_record_path(Path::new(&repo.main));
    if skipped("worker-main") {
        return sessions;
    }
    if let Some(record_json) = messaging::read_json(&main_record_path) {
        let status = messaging::worker_status_with(processes, Path::new(&repo.main));
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
            &mut views,
            status.pid,
        );
        let (last_activity_at, attached) = tmux_activity(&mut views, &terminal);
        let line = last_line(
            &mut views,
            &terminal,
            &worker_agent,
            status.present,
            last_activity_at,
        );
        let waiting = worker_waiting(
            &mut gates,
            &parent_hub,
            &repo.main,
            started_at.as_deref(),
            status.phase_at,
        );

        let task_id = record_json
            .get("task")
            .and_then(Value::as_str)
            .map(str::to_string);

        let task_title = task_id.as_deref().and_then(|id| {
            let slug = crate::repo::slug_for(
                &repo.nwo,
                messaging::worker_hub_key(Path::new(&repo.main)).as_deref(),
            );
            linked_task_title(&gates.state_dir, &slug, id)
        });
        sessions.push(session::Session {
            id: "worker-main".to_string(),
            conversation: messaging::worker_session(Path::new(&repo.main)).map(|s| s.session_id),
            kind: "worker".to_string(),
            agent: worker_agent.clone(),
            terminal,
            hub: Some(parent_hub),
            key: None,
            worktree: repo.main.clone(),
            branch: main_branch(),
            task: task_id,
            title: status.title,
            task_title,
            present: status.present,
            stale: status.stale,
            pid: status.pid,
            started_at,
            phase: status.phase,
            phase_at: status.phase_at,
            phases: status.phases,
            last_activity_at,
            last_line: line,
            attached,
            waiting,
        });
    } else if let Some(saved) = messaging::worker_session(Path::new(&repo.main)) {
        let parent_hub = parent_hub_id(repo, hubs, saved.hub.as_deref());
        let terminal = session_terminal(None, terminal_settings, &mut views, None);
        let (last_activity_at, attached) = tmux_activity(&mut views, &terminal);
        // Not present, so there is no pane to read.
        let line = last_line(
            &mut views,
            &terminal,
            &worker_agent,
            false,
            last_activity_at,
        );
        let waiting = worker_waiting(&mut gates, &parent_hub, &repo.main, None, None);

        let task_title = saved.task.as_deref().and_then(|id| {
            let slug = crate::repo::slug_for(&repo.nwo, saved.hub.as_deref());
            linked_task_title(&gates.state_dir, &slug, id)
        });
        sessions.push(session::Session {
            id: "worker-main".to_string(),
            conversation: Some(saved.session_id.clone()),
            kind: "worker".to_string(),
            agent: worker_agent.clone(),
            terminal,
            hub: Some(parent_hub),
            key: None,
            worktree: repo.main.clone(),
            branch: main_branch(),
            task: saved.task,
            title: saved.title,
            task_title,
            present: false,
            stale: false,
            pid: None,
            started_at: None,
            phase: None,
            phase_at: None,
            phases: Vec::new(),
            last_activity_at,
            last_line: line,
            attached,
            waiting,
        });
    }
    if with_lines {
        server.last_lines.keep_only(&screens);
    }
    sessions
}

/// The session `id` of this board, resolved without listing the others: no `ps` or `git` for
/// another worktree, no gates of another hub. Equal to its entry in the list `state` carries.
pub(super) fn board_session(
    server: &Server,
    settings: &crate::config::Settings,
    id: &str,
) -> Option<session::Session> {
    let repo = &server.ctx.repo;
    let listed = crate::repo::worktrees(&repo.main).unwrap_or_default();
    let (main_branch, linked) = split_main(&repo.main, listed);
    let linked_paths: Vec<String> = linked.iter().map(|w| w.path.clone()).collect();
    // A `ps` for each of the few it is asked about, not the whole process table.
    let processes = messaging::ProcessTable::each();
    let hubs = messaging::all_repo_hubs_among_with(&processes, repo, &linked_paths);
    sessions_of(
        server,
        settings,
        &hubs,
        &linked_paths,
        Listing {
            processes: &processes,
            main_branch,
            with_lines: false,
        },
        Some(id),
        |index, path| {
            (
                messaging::worker_status_with(&processes, Path::new(path)),
                linked[index].branch.clone(),
            )
        },
    )
    .into_iter()
    .next()
}

/// The main checkout's branch and the linked worktrees, out of one listing. The branch is
/// asked of git when the listing does not name the main checkout at all.
fn split_main(main: &str, listed: Vec<Worktree>) -> (Option<String>, Vec<Worktree>) {
    let mut main_branch = None;
    let mut found_main = false;
    let mut linked = Vec::with_capacity(listed.len());
    for worktree in listed {
        if Path::new(&worktree.path) == Path::new(main) {
            found_main = true;
            main_branch = worktree.branch;
        } else {
            linked.push(worktree);
        }
    }
    if !found_main {
        main_branch = branch_of(main);
    }
    (main_branch, linked)
}

/// The tasks as the board reads them, each live one with what its worker recorded without
/// stopping (`records`, oldest first, each with its diff's byte length as `diffSize` in place
/// of the diff) and the plan a person approved (`approvedPlan`, whose
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
                // Without their diffs, which are most of what a poll weighs: the page reads
                // one from the task's history when it shows it, and `diffSize` says it is there.
                value["records"] = records
                    .into_iter()
                    .map(|r| {
                        let mut record = json!(r);
                        if let Some(diff) = record.as_object_mut().and_then(|f| f.remove("diff"))
                            && let Some(diff) = diff.as_str()
                        {
                            record["diffSize"] = json!(diff.len());
                        }
                        record
                    })
                    .collect();
                value["approvedPlan"] = json!(plan);
                // Whose turn the PR is, from what the last read kept on the record. Derived
                // here, on every poll, so the rule can change without rewriting a record.
                value["prTurn"] = json!(t.pr_status.as_ref().and_then(task::pr_turn));
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

/// `/api/hubs/<id>/<action>` as its id and action, for the five actions there are (start, stop,
/// close, reset and restart). The id is one path segment, percent-decoded — the page sends it through
/// `encodeURIComponent`, and a key may hold a `/`, a space or a letter that is not ASCII. The
/// raw segment is checked for a `/` first, so an encoded one names an id and a bare one is
/// another route. An encoding that is not UTF-8 is an error for the caller to say, not a
/// different route.
fn hub_route(path: &str) -> Option<(Result<String, String>, &str)> {
    let (raw, action) = path.strip_prefix("/api/hubs/")?.split_once('/')?;
    (!raw.is_empty()
        && !raw.contains('/')
        && matches!(action, "start" | "stop" | "close" | "reset" | "restart"))
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

/// `/api/sessions/<id>/<action>` as the session id, percent-decoded as `terminal_route` does,
/// and the action, for the six there are besides the terminal (which is a WebSocket and
/// answered before routing). Which of them a board serves is for `route` to say.
fn session_route(path: &str) -> Option<(Result<String, String>, &str)> {
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
fn session_route_for(path: &str, action: &str) -> Option<Result<String, String>> {
    session_route(path)
        .filter(|(_, found)| *found == action)
        .map(|(id, _)| id)
}

/// How long the git check of one session may take in all. A worktree on a slow disk or a
/// network mount must not hold a connection thread indefinitely.
const GIT_CHECK_SECS: u64 = 10;

/// The session `id` of this board, from the board's own records.
pub(super) fn find_session(
    server: &Server,
    settings: &crate::config::Settings,
    id: &str,
) -> Result<session::Session, String> {
    board_session(server, settings, id).ok_or_else(|| format!("no such session: {id}"))
}

/// The slug of the hub the worker in `worktree` reports to, from its own record: the same one
/// `parent_hub_id` and the hub listing arrive at, without listing the hubs.
fn worker_hub_slug(repo: &crate::repo::RepoInfo, worktree: &Path) -> String {
    match messaging::worker_hub_key(worktree) {
        Some(key) => crate::repo::slug_for(&repo.nwo, Some(&key)),
        None => match &repo.hub {
            Some(_) => repo
                .clone()
                .addressed(None)
                .map(|default| default.slug)
                .unwrap_or_else(|_| repo.slug.clone()),
            None => repo.slug.clone(),
        },
    }
}

/// What one session's worktree holds that no remote has, `None` when the directory is gone.
/// The path comes from the board's own record of the session, never from the request.
pub(super) fn git_state_of(
    server: &Server,
    session: &session::Session,
) -> Result<Option<crate::repo::GitState>, String> {
    // The task's own base, when it has one: work meant for a release branch is not merged
    // because it is in the default branch.
    let base = session.task.as_deref().and_then(|task_id| {
        let slug = worker_hub_slug(&server.ctx.repo, Path::new(&session.worktree));
        task::load(&task::dir(&messaging::state_dir(), &slug), task_id)
            .ok()?
            .base
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(GIT_CHECK_SECS);
    crate::repo::worktree_git_state(Path::new(&session.worktree), base.as_deref(), deadline)
}

/// What one session's worktree holds that no remote has, asked when a person looks rather than
/// on every poll.
fn session_git(server: &Server, id: &str) -> Result<Value, String> {
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    match git_state_of(server, &session)? {
        Some(state) => {
            serde_json::to_value(state).map_err(|e| format!("cannot describe the worktree: {e}"))
        }
        None => Err(format!("{} does not exist", session.worktree)),
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

/// How a hub is to be started, from the `start` a request names: `auto` when it names none.
pub(super) fn hub_start_of(input: &Value) -> Result<super::HubStart, String> {
    match input.get("start").and_then(Value::as_str).unwrap_or("auto") {
        "auto" => Ok(super::HubStart::Auto),
        "resume" => Ok(super::HubStart::Resume),
        "new" => Ok(super::HubStart::New),
        other => Err(format!("no such start: {other}")),
    }
}

/// The context that starts `hub`: addressed by its key, refused when a parent hub's key is not
/// known (it could only be started as some other hub).
fn hub_start_context(
    server: &Server,
    hub: &crate::session::RepoHub,
    settings: crate::config::Settings,
) -> Result<super::Context, String> {
    if hub.parent && hub.key.is_none() {
        return Err(
            "the key of this hub is not known; start it with adj hub --hub <key>".to_string(),
        );
    }
    Ok(super::Context {
        repo: server.ctx.repo.clone().addressed(hub.key.as_deref())?,
        resolved: server.ctx.resolved.clone(),
        settings,
    })
}

/// The context that stops `hub`. Addressed by the slug the hub was listed under: a hub whose
/// key cannot be told can still be stopped, and nothing here needs the key for it.
fn hub_stop_context(
    server: &Server,
    hub: &crate::session::RepoHub,
    settings: crate::config::Settings,
) -> super::Context {
    let mut stopping = server.ctx.repo.clone();
    stopping.slug = hub.slug.clone();
    stopping.hub_name = hub.name.clone();
    super::Context {
        repo: stopping,
        resolved: server.ctx.resolved.clone(),
        settings,
    }
}

/// Start, stop, close, reset or restart one of the repository's hubs from the board. `id` is the
/// `hubs[].id` the page was given, so the page can only name a hub this repository was found to
/// have.
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
            let start = hub_start_of(&input)?;
            let ctx = hub_start_context(server, &hub, settings)?;
            match super::start_hub(&ctx, start)? {
                super::TabOutcome::Opened(done) => {
                    Ok(json!({ "started": true, "description": done.description }))
                }
                super::TabOutcome::AlreadyRunning(status) => {
                    Ok(json!({ "alreadyRunning": true, "pid": status.pid }))
                }
            }
        }
        "reset" => {
            // Everything start would refuse is refused before the hub is stopped: a reset that
            // could not start again would only have taken the hub down.
            if !super::hub_startable(&settings.terminal) {
                return Err(
                    "starting a hub from the board needs terminal.preset \"tmux\"".to_string(),
                );
            }
            let start_ctx = hub_start_context(server, &hub, settings.clone())?;
            let was_running = super::stop_hub(&hub_stop_context(server, &hub, settings))?;
            match super::start_hub(&start_ctx, super::HubStart::New) {
                Ok(super::TabOutcome::Opened(done)) => Ok(json!({
                    "reset": true,
                    "wasRunning": was_running,
                    "started": true,
                    "description": done.description,
                })),
                // A hub is up that this request did not start (nothing was running, or another
                // start won the race after the stop), so it is not a new conversation, and
                // the answer must not say it is.
                Ok(super::TabOutcome::AlreadyRunning(status)) => Ok(json!({
                    "reset": false,
                    "wasRunning": was_running,
                    "alreadyRunning": true,
                    "pid": status.pid,
                })),
                Err(e) if was_running => Err(format!(
                    "stopped {}, but could not start it again: {e}",
                    hub.name
                )),
                Err(e) => Err(e),
            }
        }
        "restart" => {
            // Everything the start would refuse is refused before the hub is stopped, the
            // saved conversation included: a restart that cannot reopen it has only taken the
            // hub down. The saved session file is not touched; `--resume` reads it as it is.
            if let Some(refusal) = super::board_actions::hub_resume_refusal(&settings) {
                return Err(refusal);
            }
            let start_ctx = hub_start_context(server, &hub, settings.clone())?;
            super::hub_resume_check(&start_ctx)?;
            let restarting = super::board_actions::Restarting::claim(&hub.slug, &hub.name)?;
            let was_running = super::stop_hub(&hub_stop_context(server, &hub, settings))?;
            match super::start_hub(&start_ctx, super::HubStart::Resume) {
                Ok(super::TabOutcome::Opened(done)) => {
                    // The window is open but the new hub has not registered yet: another
                    // restart now would stop it or open a second window beside it.
                    restarting.hold();
                    Ok(json!({
                        "restarted": true,
                        "wasRunning": was_running,
                        "started": true,
                        "description": done.description,
                    }))
                }
                // As for a reset: a hub is up that this request did not start, so the
                // answer must not say it was restarted.
                Ok(super::TabOutcome::AlreadyRunning(status)) => Ok(json!({
                    "restarted": false,
                    "wasRunning": was_running,
                    "alreadyRunning": true,
                    "pid": status.pid,
                })),
                Err(e) if was_running => Err(format!(
                    "stopped {}, but could not start it again: {e}",
                    hub.name
                )),
                Err(e) => Err(e),
            }
        }
        "stop" | "close" => {
            let closing = action == "close";
            if closing {
                super::closable_check(repo, &hub)?;
            }
            // A hub that will not stop is not closed: nothing is forgotten until it is gone.
            let was_running = super::stop_hub(&hub_stop_context(server, &hub, settings))?;
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

#[cfg(test)]
mod tests;
