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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::gate;
use crate::http::{self, Request};
use crate::messaging;
use crate::task;

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
    include_str!("../ui/page-body.html"),
    include_str!("../ui/core.js"),
    include_str!("../ui/board.js"),
    include_str!("../ui/actions.js"),
    include_str!("../ui/review.js"),
    include_str!("../ui/task-view.js"),
    include_str!("../ui/main.js"),
    include_str!("../ui/page-end.html"),
);

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

    record(&ctx.repo.slug, port)?;
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

// ── is anybody serving? ──────────────────────────────────────────────

fn record_path(slug: &str) -> PathBuf {
    messaging::state_dir()
        .join("dashboards")
        .join(format!("{slug}.json"))
}

/// The port a live dashboard is on, or `None`.
///
/// This is what `adj gate open` asks before it hands the ball over: a gate written with
/// nobody serving is a message into a directory no one opens, and an agent that waited on
/// one would wait for ever. Anchored on the recorded process start time like every other
/// record here, so a crashed server leaves a file that reads as absent rather than as a
/// dashboard that is about to answer.
pub fn running(slug: &str) -> Option<u16> {
    let record: Value = std::fs::read_to_string(record_path(slug))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())?;
    let pid = record.get("pid").and_then(Value::as_u64)? as u32;
    let started = record.get("psStarted").and_then(Value::as_str);
    if messaging::ps_started(pid).as_deref() != started {
        return None;
    }
    record.get("port").and_then(Value::as_u64).map(|p| p as u16)
}

fn record(slug: &str, port: u16) -> Result<(), String> {
    let path = record_path(slug);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let pid = std::process::id();
    let record = json!({ "pid": pid, "port": port, "psStarted": messaging::ps_started(pid) });
    std::fs::write(&path, format!("{record:#}\n"))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
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
        ("GET", path) if path.starts_with("/api/tasks/") && path.ends_with("/history") => {
            reply(out, task_history(server, path))
        }
        ("POST", "/api/tasks") => reply(out, create_task(server, &req.body)),
        ("POST", path) if path.starts_with("/api/tasks/") => {
            reply(out, update_task(server, req.tail(), &req.body))
        }
        ("POST", "/api/refresh") => reply(out, refresh_tasks(server)),
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

fn state(server: &Server) -> Value {
    let repo = &server.ctx.repo;
    let hub = messaging::hub_status(&repo.slug, &repo.hub_name);
    let tasks = with_records(
        task::list(&super::task::dir(&server.ctx)),
        gate::list(&super::gate::records_dir(&server.ctx)),
        gate::list_of_kind(&super::gate::answered_dir(&server.ctx), gate::Kind::Plan),
    );

    let now = messaging::now_secs();
    let settings = settings_now(server);
    // Counted as `adj work` counts, main checkout included, though it is not listed below.
    let mut busy = usize::from(messaging::holds_worker_slot(Path::new(&repo.main), now));
    // The board shows what it can; `adj work` is the one that refuses on a failed listing.
    let workers: Vec<Value> = crate::repo::linked_worktrees(&repo.main)
        .unwrap_or_default()
        .into_iter()
        .map(|path| {
            let status = messaging::worker_status(Path::new(&path));
            // A present worker holds a slot without asking `ps` again; the rest are asked
            // the way `adj work` asks, so the header and the refusal cannot disagree.
            if status.present || messaging::holds_worker_slot(Path::new(&path), now) {
                busy += 1;
            }
            json!({
                "worktree": path,
                "name": Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()),
                "branch": branch_of(&path),
                "present": status.present,
                "stale": status.stale,
                "title": status.title,
                "phase": status.phase,
                "phaseAt": status.phase_at,
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
fn settings_now(server: &Server) -> crate::config::Settings {
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
    let done = crate::terminal::focus(
        settings.terminal.focus.as_deref(),
        pid,
        &repo.hub_name,
        false,
    )?;
    Ok(json!({ "present": true, "ran": done.ran }))
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
            base: None,
            parent: None,
            worktree_name: None,
            auto_start: true,
            order: 0,
            status,
            worktree: None,
            issue: None,
            pr: None,
            note: None,
            instruction: None,
            gate_answered_at: None,
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
