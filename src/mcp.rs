//! The stdio MCP server: JSON-RPC in on stdin, JSON-RPC out on stdout.
//!
//! Hand-rolled rather than taken from an SDK. What is actually needed is `initialize`,
//! `prompts/list`, `prompts/get`, `tools/list` and `tools/call` — five methods over
//! newline-delimited JSON, which is less code than the wiring an SDK would need.
//!
//! Nothing here decides anything. The tools answer questions about the machine (where is the
//! main checkout, what is configured, is the hub running, what is waiting) and move messages
//! around; every judgement — which task to pick, whether a bug is a duplicate, whether a
//! review finding is real — lives in the procedures, where a human can read it.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use crate::cmd::HubBoard;
use crate::config;
use crate::messaging::{self, Message};
use crate::prompts;
use crate::repo;

mod schema;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SERVER_NAME: &str = "adjutant";

// ── JSON-RPC 2.0 ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    method: String,
    #[serde(default)]
    params: Value,
}

/// The id a response carries when there is no request id to echo: an error the client
/// cannot be matched to anything else. Explicitly null rather than absent — the field is
/// required in a response, and `skip_serializing_if` drops only `None`.
fn no_id() -> Option<Value> {
    Some(Value::Null)
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

fn respond(id: Option<Value>, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn respond_err(id: Option<Value>, code: i64, message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
        }),
    }
}

// ── prompts ──────────────────────────────────────────────────────────

fn prompt_definitions() -> Value {
    let list: Vec<Value> = prompts::PROMPTS
        .iter()
        .map(|prompt| {
            json!({
                "name": prompt.name,
                "description": prompts::description(prompt),
                "arguments": [
                    {
                        "name": "arguments",
                        "description": "Free text passed to the procedure (what you found, which task to pick up). Optional.",
                        "required": false,
                    },
                    {
                        "name": "agent",
                        "description": "Target agent format: claude | agy | generic. Defaults to auto-detect.",
                        "required": false,
                    },
                    {
                        "name": "worktree",
                        "description": "Path to the worktree or repository root. Defaults to the server's working directory.",
                        "required": false,
                    },
                ],
            })
        })
        .collect();
    json!({ "prompts": list })
}

static CLIENT_NAME: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

pub fn client_name() -> Option<String> {
    CLIENT_NAME.read().ok().and_then(|lock| lock.clone())
}

pub fn set_client_name(name: Option<&str>) {
    if let Ok(mut lock) = CLIENT_NAME.write() {
        *lock = name.map(str::to_string);
    }
}

pub fn runner_for_procedure(settings: &config::Settings, procedure: &str) -> Option<String> {
    match procedure {
        "adj-hub" => settings.hub_runner.clone(),
        _ => settings.agent_runner.clone(),
    }
}

pub fn resolve_runner_for(cwd: Option<&Path>, procedure: &str) -> Option<String> {
    let info = repo::resolve_in(cwd, None, None).ok()?;
    let settings = config::resolve_config(&info.nwo).ok()?.settings;
    runner_for_procedure(&settings, procedure)
}

fn prompt_get(params: &Value) -> Result<Value, String> {
    let name = params["name"].as_str().unwrap_or("");
    let prompt = prompts::find(name).ok_or_else(|| format!("Unknown prompt: {name}"))?;
    let arguments = params["arguments"]["arguments"].as_str().unwrap_or("");
    let explicit_agent = params["arguments"]["agent"].as_str();
    if let Some(value) = explicit_agent
        && !matches!(value, "claude" | "agy" | "generic")
    {
        return Err(format!("agent must be claude, agy, or generic: {value}"));
    }
    let worktree = params["arguments"]["worktree"]
        .as_str()
        .or_else(|| params["arguments"]["cwd"].as_str())
        .filter(|s| !s.is_empty())
        .map(config::expand_home);
    let runner = resolve_runner_for(worktree.as_deref(), name);
    let runner_agent = runner.as_deref().map(crate::runner::agent_from_runner);
    let agent = prompts::resolve_agent(
        explicit_agent,
        client_name().as_deref(),
        runner_agent.as_deref(),
    );
    Ok(json!({
        "description": prompts::description(prompt),
        "messages": [{
            "role": "user",
            "content": { "type": "text", "text": prompts::render_for(prompt, arguments, agent) },
        }],
    }))
}

fn cwd_param(args: &Value) -> Option<PathBuf> {
    args["cwd"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(config::expand_home)
}

fn resolve_repo(args: &Value) -> Result<repo::RepoInfo, String> {
    let cwd = cwd_param(args);
    // `cwd` and not the server's own directory, for the same reason the repository is
    // resolved from it: a worker's answer is written in the worktree it is standing in, and
    // this server is started once and then asked about whichever checkout the session is
    // sitting in.
    let hub = messaging::hub_id(args["hub"].as_str(), cwd.as_deref())?;
    repo::resolve_in(
        cwd.as_deref(),
        args["repo"].as_str().filter(|s| !s.is_empty()),
        hub.as_deref(),
    )
}

pub fn call_tool(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "adjutant_config" => {
            let info = resolve_repo(args)?;
            let resolved = config::resolve_config(&info.nwo)?;
            Ok(json!({
                "repo": info.nwo,
                "main": info.main,
                "hub": info.hub,
                "hubName": info.hub_name,
                // The board running for this hub, or null, and whether the resident server
                // serves it. Read from records, so one started by hand with `adj serve` is
                // found as well as the hub's own.
                "board": crate::cmd::board_json(&info),
                "registered": resolved.registered,
                "configPath": resolved.config_path,
                "warnings": resolved.warnings,
                "settings": resolved.settings,
                "config": resolved.config,
            }))
        }
        "adjutant_hub_status" => {
            let info = resolve_repo(args)?;
            let status = messaging::hub_status(&info.slug, &info.hub_name);
            let mut out = messaging::status_json(&status);
            out["repo"] = json!(info.nwo);
            out["hub"] = json!(info.hub);
            out["main"] = json!(info.main);
            out["waiting"] = json!(messaging::list(&info.slug).len());
            Ok(out)
        }
        "adjutant_send" => {
            let info = resolve_repo(args)?;
            let body = args["body"].as_str().unwrap_or("").trim();
            if body.is_empty() {
                return Err("body is empty".to_string());
            }
            let message = Message {
                from: args["from"].as_str().unwrap_or("unknown").to_string(),
                // From the caller's own `cwd` when it gave one: this server is started once
                // and then asked about whichever checkout the session is sitting in, so the
                // process's own directory is not the sender's.
                worktree: repo::current_worktree(cwd_param(args).as_deref()),
                kind: args["kind"].as_str().unwrap_or("report").to_string(),
                subject: args["subject"].as_str().unwrap_or("").to_string(),
                body: body.to_string(),
            };
            let wake = args.get("wake").and_then(|v| v.as_bool());
            let ctx = crate::cmd::context_of(info)?;
            let delivered = crate::cmd::deliver_to_hub_with_wake(&ctx, &message, true, wake)?;
            let mut note = match (
                delivered.delivery.present,
                delivered.woken,
                delivered.wake_needed,
            ) {
                (true, true, _) => "Woke the hub; it will pick this up. Do not wait for a reply, go back to your own task.",
                (true, false, false) => "The hub is running; waking was skipped because this message needs no action. It will pick this up the next time it checks its inbox.",
                (true, false, true) => "The hub is running; it will pick this up the next time it checks its inbox. Do not wait for a reply, go back to your own task.",
                (false, _, _) => "The hub is not running. Left in its inbox; it will be picked up the next time it starts. If this is urgent, ask the user to run `adj hub`.",
            }
            .to_string();
            if let Some(why) = &delivered.wake_note {
                note = format!("{} {note}", crate::cmd::wake_note_sentence(why));
            }
            let mut out = json!({
                "hubName": ctx.repo.hub_name,
                "present": delivered.delivery.present,
                "woken": delivered.woken,
                "path": delivered.delivery.path.to_string_lossy(),
                "note": note,
            });
            if let Some(why) = &delivered.wake_note {
                out["wakeNote"] = json!(why);
            }
            Ok(out)
        }
        "adjutant_pending" => {
            let info = resolve_repo(args)?;
            let action = args["action"].as_str().unwrap_or("list");
            match action {
                "list" => {
                    let entries: Vec<Value> = messaging::list(&info.slug)
                        .into_iter()
                        .map(|e| {
                            json!({"name": e.name, "from": e.from, "worktree": e.worktree,
                                   "kind": e.kind, "subject": e.subject})
                        })
                        .collect();
                    Ok(json!({
                        "hubName": info.hub_name,
                        "dir": messaging::inbox_dir(&info.slug).to_string_lossy(),
                        "count": entries.len(),
                        "messages": entries,
                    }))
                }
                "read" => {
                    let name = args["name"].as_str().unwrap_or("");
                    Ok(json!({ "name": name, "content": messaging::read(&info.slug, name)? }))
                }
                "ack" => {
                    let name = args["name"].as_str().unwrap_or("");
                    let moved = messaging::ack(&info.slug, name)?;
                    Ok(json!({ "name": name, "archived": moved.to_string_lossy() }))
                }
                other => Err(format!("action must be list, read or ack: {other}")),
            }
        }
        "adjutant_tell" => {
            let info = resolve_repo(args)?;
            let worktree = config::expand_home(args["worktree"].as_str().unwrap_or(""));
            if !worktree.is_dir() {
                return Err(format!("no such worktree: {}", worktree.display()));
            }
            let subject = args["subject"].as_str().unwrap_or("").trim();
            let body = args["body"].as_str().unwrap_or("").trim();
            if subject.is_empty() || body.is_empty() {
                return Err("subject and body are both required".to_string());
            }
            let ctx = crate::cmd::context_of(info)?;
            let from = args["from"]
                .as_str()
                .unwrap_or(&ctx.repo.hub_name)
                .to_string();
            let wake = args.get("wake").and_then(|v| v.as_bool());
            let told = crate::cmd::deliver_to_worker(&ctx, &worktree, &from, subject, body, wake)?;
            let mut note = match (told.present, told.woken, told.wake_needed) {
                (true, true, _) => "Woke the worker. Do not wait for a reply, go back to waiting.",
                (true, false, false) => "The worker is running; waking was skipped because this message needs no action. It will read this the next time it checks its outbox.",
                (true, false, true) => "The worker is running; it will read this the next time it checks its outbox.",
                (false, _, _) => "The worker is not running; it will read this the next time it starts.",
            }
            .to_string();
            if let Some(why) = &told.wake_note {
                note = format!("{} {note}", crate::cmd::wake_note_sentence(why));
            }
            let mut out = json!({
                "worktree": worktree.to_string_lossy(),
                "outbox": told.path.to_string_lossy(),
                "present": told.present,
                "woken": told.woken,
                "note": note,
            });
            if let Some(why) = &told.wake_note {
                out["wakeNote"] = json!(why);
            }
            Ok(out)
        }
        "adjutant_outbox" => {
            let worktree = match args["worktree"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| args["cwd"].as_str())
                .filter(|s| !s.is_empty())
            {
                Some(path) => config::expand_home(path),
                None => std::env::current_dir()
                    .map_err(|e| format!("cannot determine the current directory: {e}"))?,
            };
            match args["action"].as_str().unwrap_or("read") {
                "read" => {
                    let text = messaging::read_outbox(&worktree);
                    Ok(json!({
                        "path": messaging::outbox_path(&worktree).to_string_lossy(),
                        "empty": text.trim().is_empty(),
                        "content": text,
                    }))
                }
                "clear" => {
                    messaging::clear_outbox(&worktree)?;
                    Ok(json!({ "cleared": true }))
                }
                other => Err(format!("action must be read or clear: {other}")),
            }
        }
        "adjutant_gate_open" => {
            let ctx = crate::cmd::context_of(resolve_repo(args)?)?;
            let mut payload = args.clone();
            let fields = payload
                .as_object_mut()
                .ok_or("arguments must be an object")?;
            for key in ["repo", "hub", "cwd"] {
                fields.remove(key);
            }
            // From the caller's `cwd`, for the reason `adjutant_send` gives: the server's own
            // directory is not where the worker is standing, and this is where the answer goes.
            // A client that fills in every advertised property sends `null` or `""` for one it
            // has no value for; either is as good as absent.
            let given = fields
                .get("worktree")
                .and_then(Value::as_str)
                .is_some_and(|w| !w.trim().is_empty());
            if !given {
                let here = repo::current_worktree(cwd_param(args).as_deref())
                    .ok_or("not inside a worktree: pass worktree or cwd")?;
                fields.insert("worktree".to_string(), json!(here));
            }
            let (gate, served) = crate::cmd::gate_open_payload(&ctx, &payload)?;
            Ok(crate::cmd::gate_open_json(&ctx, &gate, served))
        }
        "adjutant_gate_close" => {
            let id = args["id"].as_str().ok_or("a gate needs an id")?;
            let comment = args.get("comment").and_then(Value::as_str);
            let ctx = crate::cmd::context_of(resolve_repo(args)?)?;
            let terminal = args
                .get("terminal")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let gate = crate::cmd::gate_close_payload(&ctx, id, comment, terminal)?;
            let on_board = gate.answered_on_board();
            Ok(json!({ "gate": gate, "closed": !on_board, "alreadyAnswered": on_board }))
        }
        "adjutant_refresh" => {
            let ctx = crate::cmd::context_of(resolve_repo(args)?)?;
            let checked = crate::cmd::task_refresh(&ctx)?;
            Ok(crate::cmd::task_refresh_json(&checked))
        }
        "adjutant_skill" => {
            let name = args["name"].as_str().unwrap_or("");
            let prompt = prompts::find(name).ok_or_else(|| {
                format!("no such procedure: {name} (adj-hub / adj-worker / adj-report)")
            })?;
            let explicit_agent = args["agent"].as_str();
            if let Some(value) = explicit_agent
                && !matches!(value, "claude" | "agy" | "generic")
            {
                return Err(format!("agent must be claude, agy, or generic: {value}"));
            }
            let worktree = args["worktree"]
                .as_str()
                .or_else(|| args["cwd"].as_str())
                .filter(|s| !s.is_empty())
                .map(config::expand_home);
            let runner = resolve_runner_for(worktree.as_deref(), name);
            let runner_agent = runner.as_deref().map(crate::runner::agent_from_runner);
            let agent = prompts::resolve_agent(
                explicit_agent,
                client_name().as_deref(),
                runner_agent.as_deref(),
            );
            Ok(json!({
                "name": prompt.name,
                "agent": agent.as_str(),
                "description": prompts::description(prompt),
                "content": prompts::render_for(prompt, args["arguments"].as_str().unwrap_or(""), agent),
            }))
        }
        other => Err(format!("Unknown tool: {other}")),
    }
}

// ── the loop ─────────────────────────────────────────────────────────

/// The longest line this server will try to make sense of. Generous next to any real call
/// — the biggest thing that crosses this pipe is a report body — and small enough that a
/// runaway writer cannot spend the machine's memory before it is answered.
const MAX_LINE: usize = 8 * 1024 * 1024;

/// How often a hub's MCP server says the hub is still there. The error in "when did it
/// end" is at most this, for an ending that gave the server no chance to say so itself.
const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(60);

/// Which hub session this server is running under, if any — `ADJUTANT_HUB_SESSION`, put on
/// the line `adj hub` execs and inherited from the agent.
fn hub_session() -> Option<(String, String)> {
    let value = std::env::var(messaging::HUB_SESSION_ENV).ok()?;
    let (slug, session) = value.trim().split_once('/')?;
    (!slug.is_empty() && !session.is_empty()).then(|| (slug.to_string(), session.to_string()))
}

/// Keep the hub's `lastAlive` current for as long as this server runs.
///
/// The server is the agent's child and lives exactly as long as the session does, which is
/// what `adj hub` cannot see for itself: it has `exec`ed into the agent and is gone. A
/// thread rather than a timer in the loop, because the loop sits in a blocking read for as
/// long as the hub is idle — which is most of the time.
fn start_heartbeat() -> Option<(String, String)> {
    let (slug, session) = hub_session()?;
    let (beat_slug, beat_session) = (slug.clone(), session.clone());
    std::thread::spawn(move || {
        loop {
            let _ = messaging::touch_hub_session(&beat_slug, &beat_session);
            std::thread::sleep(HEARTBEAT);
        }
    });
    Some((slug, session))
}

/// How long a hub's MCP server gives a board that is still recorded as running to go away,
/// and how often it looks. A reconnect starts this server again inside the same session,
/// and the server it replaces — whose board that is — may take a moment to exit and let go
/// of the port. A board still there after this was started by somebody else, and is left
/// to serve.
const BOARD_HANDOVER: std::time::Duration = std::time::Duration::from_secs(5);
const BOARD_HANDOVER_STEP: std::time::Duration = std::time::Duration::from_millis(250);

/// How often a hub that left its board to the resident server looks whether the resident is
/// still there. Not a tight loop: a stopped resident is noticed within this, and nothing is
/// lost meanwhile, since the board is only a view of records that stay on disk.
const RESIDENT_WATCH: std::time::Duration = std::time::Duration::from_secs(5);

/// The second look before a hub serves a board of its own in the resident's place: a poll that
/// lands in the middle of a restart sees no resident, and a board bound then would stay bound
/// beside the one that comes back.
const RESIDENT_RECHECK: std::time::Duration = std::time::Duration::from_secs(1);

/// The slug of the hub whose board this server serves, if any — `ADJUTANT_HUB_SERVE`, put
/// on the line `adj hub` execs when `hubServe` is on.
fn hub_serve() -> Option<String> {
    let value = std::env::var(messaging::HUB_SERVE_ENV).ok()?;
    let slug = value.trim();
    (!slug.is_empty()).then(|| slug.to_string())
}

/// The hub `slug` names, resolved the way `adj serve` resolves it from where it stands: the
/// hub runs in the main checkout, and a hub for a parent task has `ADJUTANT_HUB` on its line.
/// A slug that does not come out the same is a server started somewhere else than the hub
/// it was told about, and serving whatever it resolved to would put up the wrong board.
fn hub_board_context(slug: &str) -> Result<crate::cmd::Context, String> {
    let ctx = crate::cmd::context(None, None)?;
    if ctx.repo.slug != slug {
        return Err(format!(
            "this server resolves to {} rather than {slug}",
            ctx.repo.slug
        ));
    }
    Ok(ctx)
}

/// Serve the board of the hub this server was started for, when it was started for one.
///
/// Before the loop rather than beside it, so that the URL is on record by the time the hub's
/// first `adjutant_config` asks for it. Everything goes to stderr: stdout is the protocol.
/// A board that cannot be served is said and got past — the hub works without one, as it
/// always has.
fn start_board() {
    let Some(slug) = hub_serve() else {
        return;
    };
    let served = hub_board_context(&slug).and_then(crate::cmd::serve_for_hub);
    match &served {
        Ok(HubBoard::Serving(url)) => say_serving(url),
        Ok(HubBoard::Resident(url)) => say_resident(url),
        Ok(HubBoard::AlreadyRunning) => {}
        Err(e) => eprintln!("adjutant: not serving the board: {e}"),
    }
    // The one thread that looks after this hub's board from here on, whichever way it got
    // here: nothing else starts one, so two never run.
    if let Some(mode) = watch_mode(&served, false) {
        std::thread::spawn(move || watch_board(&slug, mode));
    }
}

/// Where the board is, without the token: this line lands in the client's log of the server,
/// which is the kind of file that gets attached to a bug report. The hub gets the whole URL
/// from `adjutant_config`.
fn say_serving(url: &str) {
    let place = url.split('?').next().unwrap_or(url);
    eprintln!("adjutant: serving the board at {place}");
}

/// The same for a board the resident server serves, which this hub started nothing for.
fn say_resident(url: &str) {
    let place = url.split('?').next().unwrap_or(url);
    eprintln!("adjutant: the resident server serves the board at {place}");
}

/// What a hub's board watcher is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The resident server serves the board; wait for it to go away.
    Resident,
    /// A board recorded as running is somebody else's or a previous server's; wait
    /// `BOARD_HANDOVER` for it to go away, and leave it be if it does not.
    Handover,
}

/// The mode to watch in after an attempt to serve the board, or `None` when there is nothing
/// left to watch. `retrying` is a watcher that has been through one before: an error there is
/// tried again on the next poll, while at the very start it is said and got past — the hub
/// works without a board, as it always has.
fn watch_mode(served: &Result<HubBoard, String>, retrying: bool) -> Option<Mode> {
    match served {
        Ok(HubBoard::Serving(_)) => None,
        Ok(HubBoard::Resident(_)) => Some(Mode::Resident),
        Ok(HubBoard::AlreadyRunning) => Some(Mode::Handover),
        Err(_) => retrying.then_some(Mode::Resident),
    }
}

/// Whether an error is worth saying: the first of a kind, not the same one on every poll.
fn is_new_error(last: &mut Option<String>, message: &str) -> bool {
    if last.as_deref() == Some(message) {
        return false;
    }
    *last = Some(message.to_string());
    true
}

/// Look after this hub's board for as long as there is something to look after: the resident
/// that serves it may stop or crash, and a board that was in the way may go. Lives as long as
/// this server does and looks only at intervals, never in a tight loop.
fn watch_board(slug: &str, mut mode: Mode) {
    let mut last_error: Option<String> = None;
    let mut waited = std::time::Duration::ZERO;
    loop {
        match mode {
            Mode::Resident => {
                std::thread::sleep(RESIDENT_WATCH);
                if crate::cmd::resident_running() {
                    continue;
                }
                std::thread::sleep(RESIDENT_RECHECK);
                if crate::cmd::resident_running() {
                    continue;
                }
            }
            Mode::Handover => {
                if waited >= BOARD_HANDOVER {
                    eprintln!("adjutant: a board for this hub is already running; leaving it be");
                    return;
                }
                std::thread::sleep(BOARD_HANDOVER_STEP);
                waited += BOARD_HANDOVER_STEP;
                if crate::cmd::board_running(slug).is_some() {
                    continue;
                }
            }
        }
        let served = hub_board_context(slug).and_then(crate::cmd::serve_for_hub);
        match &served {
            Ok(HubBoard::Serving(url)) => say_serving(url),
            Ok(HubBoard::Resident(url)) => say_resident(url),
            Ok(HubBoard::AlreadyRunning) => {}
            Err(e) => {
                if is_new_error(&mut last_error, e) {
                    eprintln!("adjutant: not serving the board: {e}");
                }
            }
        }
        match watch_mode(&served, true) {
            None => return,
            Some(next) => {
                if next != mode {
                    waited = std::time::Duration::ZERO;
                }
                mode = next;
            }
        }
    }
}

pub fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    let heartbeat = start_heartbeat();
    start_board();
    let served = serve_stdio();
    // The client closed the pipe: the session is ending now, which is a better answer than
    // the last beat. Written on the way out whatever the loop ended with.
    if let Some((slug, session)) = &heartbeat {
        let _ = messaging::touch_hub_session(slug, session);
    }
    served
}

fn serve_stdio() -> Result<(), Box<dyn std::error::Error>> {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    // Bytes, not `lines()`. `lines()` hands back an error for a line that is not valid
    // UTF-8, and treating that as end-of-input ended the server on one bad byte — every
    // later call from that session then failed, with nothing said about why. A line that
    // cannot be decoded is one malformed message, which is what a parse error is for.
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        buffer.clear();
        match stdin.read_until(b'\n', &mut buffer) {
            // Nothing more is coming: the client closed the pipe.
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => return Err(Box::new(e)),
        }
        // One line is one message, and a message this size is not one this server has a
        // use for. Answering rather than parsing keeps a runaway writer from turning into
        // a parse of the same size, and the buffer gives the memory back rather than
        // holding the high-water mark for the rest of the session.
        if buffer.len() > MAX_LINE {
            buffer = Vec::new();
            let response = respond_err(
                no_id(),
                -32600,
                &format!("Invalid Request: a message may not exceed {MAX_LINE} bytes"),
            );
            if let Ok(text) = serde_json::to_string(&response) {
                writeln!(stdout, "{text}")?;
                stdout.flush()?;
            }
            continue;
        }
        let response = match std::str::from_utf8(&buffer) {
            Ok(line) => match line.trim() {
                "" => continue,
                line => handle_line(line),
            },
            Err(e) => Some(respond_err(
                no_id(),
                -32700,
                &format!("Parse error: the message is not valid UTF-8: {e}"),
            )),
        };
        let Some(response) = response else {
            continue;
        };
        if let Ok(text) = serde_json::to_string(&response) {
            writeln!(stdout, "{text}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn handle_line(line: &str) -> Option<JsonRpcResponse> {
    // Syntax first, shape second. Deserializing straight into the request struct ran the
    // two together and got both wrong: `{}` came back as a *parse* error when the JSON
    // parsed perfectly and it was the request that was invalid, and — because serde will
    // read a struct from a sequence — `["2.0", 1, "ping", {}]` was accepted and answered
    // as a call.
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => return Some(respond_err(no_id(), -32700, &format!("Parse error: {e}"))),
    };
    let Some(object) = value.as_object() else {
        return Some(respond_err(
            no_id(),
            -32600,
            "Invalid Request: a request is a JSON object (batches are not supported)",
        ));
    };
    // Whether this is a notification is decided by the id, but only once the thing is
    // recognisable as a request at all: `{}` has no id *and* no method, and answering it
    // with silence — as if it were a notification — leaves a client waiting on a message
    // it will never get told was nonsense.
    if !object.get("method").is_some_and(Value::is_string) {
        return Some(respond_err(
            no_id(),
            -32600,
            "Invalid Request: method must be a string",
        ));
    }
    // A notification has no id and takes no reply. `notifications/initialized` is the one
    // that always arrives; replying to it is a protocol error. An id that is *present and
    // null* is not one of these — it is a request, and it gets an answer. (Base JSON-RPC
    // allows a null id; MCP's own schema does not, so this is leniency on our side rather
    // than something a client should rely on.)
    let id = Some(object.get("id")?.clone());
    if !matches!(
        object.get("id"),
        Some(Value::Null | Value::Number(_) | Value::String(_))
    ) {
        // Echoing an id that is not one only hands the client back its own confusion.
        return Some(respond_err(
            no_id(),
            -32600,
            "Invalid Request: id must be a string, a number, or null",
        ));
    }
    let request: JsonRpcRequest = match serde_json::from_value(value.clone()) {
        Ok(request) => request,
        Err(e) => return Some(respond_err(id, -32600, &format!("Invalid Request: {e}"))),
    };
    if request.jsonrpc.as_deref() != Some("2.0") {
        return Some(respond_err(
            id,
            -32600,
            "Invalid Request: jsonrpc must be \"2.0\"",
        ));
    }
    Some(match request.method.as_str() {
        "initialize" => {
            if let Some(client) = request
                .params
                .get("clientInfo")
                .and_then(|c| c.get("name"))
                .and_then(Value::as_str)
            {
                set_client_name(Some(client));
            }
            respond(
                id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {}, "prompts": {} },
                    "serverInfo": { "name": SERVER_NAME, "version": VERSION },
                    "instructions": prompts::INSTRUCTIONS,
                }),
            )
        }
        "ping" => respond(id, json!({})),
        "prompts/list" => respond(id, prompt_definitions()),
        "prompts/get" => match prompt_get(&request.params) {
            Ok(result) => respond(id, result),
            Err(e) => respond_err(id, -32602, &e),
        },
        "tools/list" => respond(id, schema::tool_definitions()),
        "tools/call" => {
            let name = request.params["name"].as_str().unwrap_or("");
            match call_tool(name, &request.params["arguments"]) {
                Ok(result) => respond(
                    id,
                    json!({ "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&result).unwrap_or_default(),
                    }]}),
                ),
                // A tool failure is reported inside the result, not as a JSON-RPC error: the
                // model has to see the message to act on it, and a transport-level error is
                // handled by the client before it ever gets there.
                Err(e) => respond(
                    id,
                    json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
                ),
            }
        }
        other => respond_err(id, -32601, &format!("Method not found: {other}")),
    })
}

/// Register this binary as an MCP server with an agent that keeps its servers in a JSON file.
/// How the registration should name this binary.
///
/// The bare name whenever PATH already resolves to *this* binary, and an absolute path
/// otherwise. Registering the absolute path unconditionally is the footgun: run
/// `install-mcp` out of a build directory and every session from then on starts the build
/// that happened to be sitting there, so an upgrade changes nothing and a `cargo clean`
/// breaks the server.
/// The two names this program is installed under. `adj` is the one a person types; the long
/// one is what a config file should say, so reading it back later needs no guessing.
pub const BIN_NAMES: [&str; 2] = ["adjutant", "adj"];

pub fn server_command() -> String {
    let Ok(exe) = std::env::current_exe() else {
        return BIN_NAMES[0].to_string();
    };
    let canonical = std::fs::canonicalize(&exe).unwrap_or(exe.clone());
    // Both names are tried, because `adj install-mcp` is how it will actually be typed —
    // and looking only for the long one would fall through to the absolute path there,
    // which is the very footgun this exists to avoid.
    for name in BIN_NAMES {
        if resolves_to(name, &canonical) {
            return name.to_string();
        }
    }
    exe.to_string_lossy().to_string()
}

/// Would a bare `name` start exactly this binary? Only the first one on PATH counts — that
/// is the one that would run, so a match further along would be starting someone else.
fn resolves_to(name: &str, canonical: &Path) -> bool {
    for dir in std::env::split_paths(&std::env::var("PATH").unwrap_or_default()) {
        let candidate = dir.join(name);
        if !candidate.is_file() {
            continue;
        }
        return std::fs::canonicalize(&candidate).is_ok_and(|resolved| resolved == *canonical);
    }
    false
}

pub fn install(target: &str) -> Result<(), String> {
    let exe = server_command();
    match target {
        "claude-code" => {
            let status = std::process::Command::new("claude")
                .args([
                    "mcp",
                    "add",
                    "--scope",
                    "user",
                    "--transport",
                    "stdio",
                    SERVER_NAME,
                    "--",
                    &exe,
                    "mcp",
                ])
                .status()
                .map_err(|e| format!("cannot run claude: {e}"))?;
            if !status.success() {
                return Err(format!(
                    "claude mcp add failed (exit {})",
                    status.code().unwrap_or(-1)
                ));
            }
            eprintln!("registered {SERVER_NAME} with Claude Code");
            Ok(())
        }
        "agy" | "antigravity" => {
            let status = std::process::Command::new("agy")
                .args(["mcp", "add", SERVER_NAME, &exe, "mcp"])
                .status()
                .map_err(|e| format!("cannot run agy: {e}"))?;
            if !status.success() {
                return Err(format!(
                    "agy mcp add failed (exit {})",
                    status.code().unwrap_or(-1)
                ));
            }
            eprintln!("registered {SERVER_NAME} with Antigravity (agy)");
            Ok(())
        }
        "json" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "mcpServers": { SERVER_NAME: { "command": exe, "args": ["mcp"] } }
                }))
                .unwrap_or_default()
            );
            Ok(())
        }
        other => Err(format!("target must be claude-code, agy, or json: {other}")),
    }
}

pub fn uninstall(target: &str) -> Result<(), String> {
    match target {
        "claude-code" => {
            let status = std::process::Command::new("claude")
                .args(["mcp", "remove", "--scope", "user", SERVER_NAME])
                .status()
                .map_err(|e| format!("cannot run claude: {e}"))?;
            if !status.success() {
                return Err(format!(
                    "claude mcp remove failed (exit {})",
                    status.code().unwrap_or(-1)
                ));
            }
            eprintln!("removed {SERVER_NAME} from Claude Code");
            Ok(())
        }
        "agy" | "antigravity" => {
            let status = std::process::Command::new("agy")
                .args(["mcp", "remove", SERVER_NAME])
                .status()
                .map_err(|e| format!("cannot run agy: {e}"))?;
            if !status.success() {
                return Err(format!(
                    "agy mcp remove failed (exit {})",
                    status.code().unwrap_or(-1)
                ));
            }
            eprintln!("removed {SERVER_NAME} from Antigravity (agy)");
            Ok(())
        }
        other => Err(format!("target must be claude-code or agy: {other}")),
    }
}

#[cfg(test)]
mod tests;
