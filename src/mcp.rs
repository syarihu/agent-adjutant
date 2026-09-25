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

use crate::config;
use crate::messaging::{self, Message};
use crate::notify;
use crate::prompts;
use crate::repo;
use crate::terminal;

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
    let agent = prompts::resolve_agent(explicit_agent, client_name().as_deref(), runner.as_deref());
    Ok(json!({
        "description": prompts::description(prompt),
        "messages": [{
            "role": "user",
            "content": { "type": "text", "text": prompts::render_for(prompt, arguments, agent) },
        }],
    }))
}

// ── tools ────────────────────────────────────────────────────────────

fn repo_property() -> Value {
    json!({
        "type": "string",
        "description": "owner/name. Defaults to the repository of `cwd`, taken from its origin remote.",
    })
}

/// Named `hub` rather than `hubName`: what goes in here is the identifier a person typed
/// after `--hub`, not the `adjutant-…` session name the tools answer with.
fn hub_property() -> Value {
    json!({
        "type": "string",
        "description": "Which hub of the repository, when it is not the repository's own one. Leave it out unless you were told otherwise: a hub already knows its own, and a worker's is read from the worktree it is in.",
    })
}

fn cwd_property() -> Value {
    json!({
        "type": "string",
        "description": "Directory to answer for. Defaults to the server's own working directory; pass the worktree you are in if that is somewhere else.",
    })
}

fn tool_definitions() -> Value {
    json!({ "tools": [
        {
            "name": "adjutant_config",
            "description": "The resolved configuration for a repository: task sources (always an array, flat shorthand already expanded), defaults already merged, and the machine settings (terminal, agent runner, notification, editor, whether to collect the dashboard at startup). Also reports warnings about the config rather than failing on it. Call this once at startup instead of reading the config file.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_property(), "hub": hub_property(), "cwd": cwd_property() },
            },
        },
        {
            "name": "adjutant_hub_status",
            "description": "The hub session name for a repository, and whether that hub is currently running. The name is the address a report is sent to; derive it here rather than reconstructing it, so both sides always agree.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_property(), "hub": hub_property(), "cwd": cwd_property() },
            },
        },
        {
            "name": "adjutant_send",
            "description": "Deliver a message to a repository's hub. Never fails for want of a listener: if the hub is not running the message waits in its inbox and is picked up when it next starts, and the reply says which of the two happened. Use for bug reports found mid-task, answers to a hub's question, acknowledgements, and telling the hub a task is finished so it can close this tab and clear the worktree. The message records the worktree you are sending from, derived from `cwd`, so pass `cwd` whenever you are not in it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "body": { "type": "string", "description": "The message. Markdown; keep it under 30 lines — the hub reshapes it into an issue." },
                    "subject": { "type": "string", "description": "One line stating the conclusion. This is all a human sees in a listing." },
                    "from": { "type": "string", "description": "Who is sending: your session or worktree name." },
                    "kind": { "type": "string", "description": "report (default) | question | answer | ack | done | needs-user" },
                    "repo": repo_property(),
                    "hub": hub_property(),
                    "cwd": cwd_property(),
                },
                "required": ["body"],
            },
        },
        {
            "name": "adjutant_pending",
            "description": "The messages waiting for a hub. action=list (default) summarises them, action=read returns one in full, action=ack files one away once it has been dealt with. A hub reads this at startup and again before going back to waiting.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "read", "ack"], "description": "Default: list." },
                    "name": { "type": "string", "description": "Message file name, as given by action=list. Required for read and ack." },
                    "repo": repo_property(),
                    "hub": hub_property(),
                    "cwd": cwd_property(),
                },
            },
        },
        {
            "name": "adjutant_tell",
            "description": "Leave a message for the worker in a worktree, and wake it if it is sitting there. This is the hub-to-worker direction: the address is the worktree, not a session, so it reaches whatever agent is working there whatever it is doing. Start the subject with `[質問 <id>]` when you need an answer back — that marker is what tells the worker it may reply.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "worktree": { "type": "string", "description": "Absolute path of the worktree." },
                    "subject": { "type": "string", "description": "One line stating the point. `[質問 <id>]` asks for an answer; `[ack]` acknowledges; anything else is a notice." },
                    "body": { "type": "string", "description": "The message. Markdown." },
                    "from": { "type": "string", "description": "Who is speaking (default: this repository's hub name)." },
                    "repo": repo_property(),
                    "hub": hub_property(),
                    "cwd": cwd_property(),
                },
                "required": ["worktree", "subject", "body"],
            },
        },
        {
            "name": "adjutant_outbox",
            "description": "What the hub has left for the worker in a worktree. action=read (default) returns everything waiting, action=clear says it has all been dealt with. A worker reads this every time it comes back from asking the user — that is where messages pile up.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["read", "clear"], "description": "Default: read." },
                    "worktree": { "type": "string", "description": "Default: the server's working directory." },
                },
            },
        },
        {
            "name": "adjutant_gate_open",
            "description": "Put something in front of a person on the board, the same as `adj gate open`: the arguments are the gate's payload. By default the gate waits — end your turn and read `adjutant_outbox` when woken; `server` says whether a board is up to see it at all, and when it is `down` ask in your own tab instead. With `wait: false` (diff and verify only) it is kept as a record: nobody is asked, and you go on with your work.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["plan", "diff", "verify", "dispatch", "issue", "question", "result"] },
                    "title": { "type": "string", "description": "What this is, in one line. Most of what the board shows." },
                    "task": { "type": "string", "description": "The task record id from the brief, when there is one." },
                    "worktree": { "type": "string", "description": "Where the answer goes. Default: the worktree `cwd` is in." },
                    "wait": { "type": "boolean", "description": "false to keep a diff or verify gate as a record instead of waiting on it. Default: true." },
                    "facts": { "type": "array", "items": { "type": "string" }, "description": "True whatever is decided: rounds run, tests passed, lines changed." },
                    "focus": { "type": "string", "description": "What the person has to decide. Enough to answer from alone." },
                    "decided": { "type": "string", "description": "What is settled. Shown folded away." },
                    "unsure": { "type": "string", "description": "Where your confidence ran out." },
                    "body": { "type": "string", "description": "A report, for a result gate." },
                    "run": { "type": "string", "description": "How to run it, for verify." },
                    "diff": { "type": "string", "description": "The diff, for diff." },
                    "choices": { "type": "array", "items": { "type": "object" }, "description": "Designs to choose between: id, label, why, points, recommended." },
                    "options": { "type": "array", "items": { "type": "string" }, "description": "The buttons. Default: by kind." },
                    "rounds": { "type": "integer", "description": "How many times this same point has gone back and forth with a person." },
                    "problem": { "type": "string", "description": "plan: what is wrong today, from the request and the issue." },
                    "goal": { "type": "string", "description": "plan: what done looks like." },
                    "reviewRounds": { "type": "array", "items": { "type": "object" }, "description": "diff: one per review round — engine, must, want, scope, falsePositives." },
                    "findings": { "type": "array", "items": { "type": "object" }, "description": "diff: severity (must | want | scope), location, text, outcome (open | fixed | declined), reason when declined." },
                    "commands": { "type": "array", "items": { "type": "object" }, "description": "verify: command, result (pass | fail), time, output." },
                    "manual": { "type": "array", "items": { "type": "string" }, "description": "verify: the checks left for a person." },
                    "repo": repo_property(),
                    "hub": hub_property(),
                    "cwd": cwd_property(),
                },
                "required": ["kind", "title"],
            },
        },
        {
            "name": "adjutant_skill",
            "description": "The full text of one of adjutant's procedures: adj-hub (running the hub), adj-worker (taking a task from brief to handover), adj-report (handing a bug you found to the hub). Same text the MCP prompts serve; use this tool when prompts are not available to you.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "enum": ["adj-hub", "adj-worker", "adj-report"] },
                    "arguments": { "type": "string", "description": "Free text substituted into the procedure where it asks for it." },
                    "agent": { "type": "string", "enum": ["claude", "agy", "generic"], "description": "Target agent format: claude | agy | generic. Defaults to auto-detect." },
                    "worktree": { "type": "string", "description": "Path to the worktree or repository root. Defaults to the server's working directory." },
                },
                "required": ["name"],
            },
        },
    ]})
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
            let subject = messaging::header_value(&messaging::render_message(&message), "subject")
                .unwrap_or_default();
            let delivery = messaging::send(&info.slug, &info.hub_name, &message)?;
            // A file appearing in a directory wakes nobody. Whichever way this went, the
            // person is the one who has to go and look, so they get told.
            let settings = config::resolve_config(&info.nwo)
                .map(|r| r.settings)
                .unwrap_or_default();
            let woken = match (
                delivery.present,
                messaging::hub_status(&info.slug, &info.hub_name).pid,
            ) {
                (true, Some(pid)) => terminal::wake(
                    &settings.hub_wake,
                    pid,
                    &subject,
                    terminal::HUB_WAKE_LINE,
                    false,
                )
                .map(|done| done.ran)
                .unwrap_or(false),
                _ => false,
            };
            if let Some(command) = notify::repo_command(&settings.notification, &info, &subject) {
                let _ = terminal::run_shell(&command);
            }
            Ok(json!({
                "hubName": info.hub_name,
                "present": delivery.present,
                "woken": woken,
                "path": delivery.path.to_string_lossy(),
                "note": match (delivery.present, woken) {
                    (true, true) => "Woke the hub; it will pick this up. Do not wait for a reply, go back to your own task.",
                    (true, false) => "The hub is running; it will pick this up the next time it checks its inbox. Do not wait for a reply, go back to your own task.",
                    (false, _) => "The hub is not running. Left in its inbox; it will be picked up the next time it starts. If this is urgent, ask the user to run `adj hub`.",
                },
            }))
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
            let from = args["from"].as_str().unwrap_or(&info.hub_name).to_string();
            let path = messaging::tell(&worktree, &from, subject, body)?;
            let settings = config::resolve_config(&info.nwo)
                .map(|r| r.settings)
                .unwrap_or_default();
            let status = messaging::worker_status(&worktree);
            let woken = match (status.present, status.pid) {
                (true, Some(pid)) => terminal::wake(
                    &settings.worker_wake,
                    pid,
                    subject,
                    terminal::WORKER_WAKE_LINE,
                    false,
                )
                .map(|done| done.ran)
                .unwrap_or(false),
                _ => false,
            };
            // Only fall back to interrupting the human when the worker itself could not be
            // reached; a woken worker is about to read it without anyone's help.
            if !woken
                && let Some(command) = notify::repo_command(&settings.notification, &info, subject)
            {
                let _ = terminal::run_shell(&command);
            }
            Ok(json!({
                "worktree": worktree.to_string_lossy(),
                "outbox": path.to_string_lossy(),
                "present": status.present,
                "woken": woken,
                "note": match (status.present, woken) {
                    (true, true) => "Woke the worker. Do not wait for a reply, go back to waiting.",
                    (true, false) => "The worker is running; it will read this the next time it checks its outbox.",
                    (false, _) => "The worker is not running; it will read this the next time it starts.",
                },
            }))
        }
        "adjutant_outbox" => {
            let worktree = match args["worktree"].as_str().filter(|s| !s.is_empty()) {
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
            Ok(crate::cmd::gate_open_json(&gate, served))
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
            let agent =
                prompts::resolve_agent(explicit_agent, client_name().as_deref(), runner.as_deref());
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

pub fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    let heartbeat = start_heartbeat();
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
        "tools/list" => respond(id, tool_definitions()),
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
mod tests {
    use super::*;

    fn call(method: &str, params: Value) -> Value {
        let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let response = handle_line(&line.to_string()).expect("a request always gets a reply");
        serde_json::to_value(&response).unwrap()
    }

    #[test]
    fn the_registration_never_names_a_binary_that_is_not_this_one() {
        // Either the bare name — which must then resolve to this very binary — or an
        // absolute path to it. Anything else registers someone else's build.
        let command = server_command();
        match BIN_NAMES.contains(&command.as_str()) {
            true => {
                let found = std::env::split_paths(&std::env::var("PATH").unwrap_or_default())
                    .map(|d| d.join(&command))
                    .find(|p| p.is_file())
                    .and_then(|p| std::fs::canonicalize(p).ok());
                let me = std::env::current_exe()
                    .ok()
                    .and_then(|p| std::fs::canonicalize(p).ok());
                assert_eq!(found, me);
            }
            false => assert!(Path::new(&command).is_absolute(), "{command}"),
        }
    }

    #[test]
    fn initialize_advertises_both_prompts_and_tools() {
        let out = call("initialize", json!({}));
        assert_eq!(out["result"]["serverInfo"]["name"], SERVER_NAME);
        assert!(out["result"]["capabilities"]["prompts"].is_object());
        assert!(out["result"]["capabilities"]["tools"].is_object());
        assert!(
            out["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("adj-report")
        );
    }

    #[test]
    fn a_notification_gets_no_reply() {
        let line = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert!(handle_line(&line.to_string()).is_none());
    }

    #[test]
    fn a_non_json_line_is_a_parse_error_rather_than_a_crash() {
        let out = serde_json::to_value(handle_line("not json").unwrap()).unwrap();
        assert_eq!(out["error"]["code"], -32700);
    }

    #[test]
    fn all_three_procedures_are_listed_and_fetchable() {
        let listed = call("prompts/list", json!({}));
        let names: Vec<&str> = listed["result"]["prompts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["adj-hub", "adj-worker", "adj-report"]);

        let got = call(
            "prompts/get",
            json!({"name": "adj-report", "arguments": {"arguments": "画像が潰れる"}}),
        );
        let text = got["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("画像が潰れる"));
        assert!(!text.contains("$ARGUMENTS"));
    }

    #[test]
    fn an_unknown_prompt_is_an_error_not_an_empty_procedure() {
        let out = call("prompts/get", json!({"name": "adj-nope"}));
        assert_eq!(out["error"]["code"], -32602);
    }

    #[test]
    fn every_advertised_tool_is_one_the_dispatcher_knows() {
        // Every tool here is called for real, and several of them read the config and the
        // state directory. Without this the test answered about the developer's own
        // machine — harmless while they all happened to be read-only, and a fixture dropped
        // into a live hub's inbox the day one of them is not.
        let _sandbox = crate::testing::Sandbox::empty();
        let listed = call("tools/list", json!({}));
        for tool in listed["result"]["tools"].as_array().unwrap() {
            let name = tool["name"].as_str().unwrap();
            assert!(!tool["description"].as_str().unwrap().is_empty(), "{name}");
            assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
            // "Unknown tool" is the dispatcher saying it has never heard of this name; any
            // other error means it tried and failed, which is what we want here.
            if let Err(e) = call_tool(name, &json!({})) {
                assert!(
                    !e.starts_with("Unknown tool"),
                    "{name} is advertised but not wired up"
                );
            }
        }
    }

    /// A hub an agent cannot name is a hub an agent cannot reach. The tools are the only
    /// way in for a session that has no shell, so every one of them that answers about a
    /// repository's hub has to take which hub of it — otherwise the address MCP reaches is
    /// always the repository's own, whatever the session was started as.
    #[test]
    fn every_tool_that_takes_a_repository_takes_which_hub_of_it() {
        let listed = call("tools/list", json!({}));
        for tool in listed["result"]["tools"].as_array().unwrap() {
            let properties = &tool["inputSchema"]["properties"];
            if properties.get("repo").is_none() {
                continue;
            }
            assert!(
                properties.get("hub").is_some(),
                "{} takes a repository but not which hub of it",
                tool["name"]
            );
        }
    }

    /// The same split the command line makes, made where the agent actually stands.
    #[test]
    fn a_tool_call_naming_a_hub_moves_the_address_and_not_the_lookup() {
        let _sandbox = crate::testing::Sandbox::empty();
        let dir = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["remote", "add", "origin", "git@github.com:acme/widget.git"],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(&args)
                    .current_dir(dir.path())
                    .status()
                    .unwrap()
                    .success(),
                "git {args:?}"
            );
        }
        let cwd = json!(dir.path().to_string_lossy());

        let plain = resolve_repo(&json!({"cwd": cwd})).unwrap();
        let feature = resolve_repo(&json!({"cwd": cwd, "hub": "wid-957"})).unwrap();
        assert!(plain.hub.is_none());
        assert_eq!(feature.hub.as_deref(), Some("wid-957"));
        // Two addresses…
        assert_ne!(plain.hub_name, feature.hub_name);
        assert_ne!(plain.slug, feature.slug);
        // …and one repository, which is the key `config::resolve_config` is handed.
        assert_eq!(plain.nwo, feature.nwo);
        assert_eq!(feature.nwo, "acme/widget");

        // Blank is silence here too. A client filling every advertised property in with an
        // empty string would otherwise address a hub nobody can name a second time.
        assert_eq!(
            resolve_repo(&json!({"cwd": cwd, "hub": ""}))
                .unwrap()
                .hub_name,
            plain.hub_name
        );
    }

    #[test]
    fn a_tool_failure_comes_back_as_content_the_model_can_read() {
        let out = call(
            "tools/call",
            json!({"name": "adjutant_skill", "arguments": {"name": "nope"}}),
        );
        assert_eq!(out["result"]["isError"], true);
        assert!(
            out["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("adj-hub")
        );
        assert!(out["error"].is_null());
    }

    static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct ClientNameGuard;
    impl Drop for ClientNameGuard {
        fn drop(&mut self) {
            set_client_name(None);
        }
    }

    #[test]
    fn the_skill_tool_serves_the_same_text_as_the_prompt() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let via_tool = call_tool("adjutant_skill", &json!({"name": "adj-worker"})).unwrap();
        let via_prompt = prompt_get(&json!({"name": "adj-worker"})).unwrap();
        assert_eq!(
            via_tool["content"].as_str().unwrap(),
            via_prompt["messages"][0]["content"]["text"]
                .as_str()
                .unwrap()
        );
    }

    #[test]
    fn an_unknown_method_is_method_not_found() {
        let out = call("resources/list", json!({}));
        assert_eq!(out["error"]["code"], -32601);
    }

    #[test]
    fn the_skill_tool_respects_explicit_agent_format() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let claude = call_tool(
            "adjutant_skill",
            &json!({"name": "adj-hub", "agent": "claude"}),
        )
        .unwrap();
        let agy = call_tool(
            "adjutant_skill",
            &json!({"name": "adj-hub", "agent": "agy"}),
        )
        .unwrap();

        assert_eq!(claude["agent"], "claude");
        assert_eq!(agy["agent"], "agy");

        let claude_text = claude["content"].as_str().unwrap();
        let agy_text = agy["content"].as_str().unwrap();

        assert!(claude_text.contains("AskUserQuestion"));
        assert!(!agy_text.contains("AskUserQuestion"));
        assert!(agy_text.contains("ask_question"));
    }

    #[test]
    fn client_info_initialization_defaults_agent_format() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let _guard = ClientNameGuard;
        let _ = call(
            "initialize",
            json!({"clientInfo": {"name": "antigravity-cli"}}),
        );
        let via_prompt = prompt_get(&json!({"name": "adj-hub"})).unwrap();
        let text = via_prompt["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(!text.contains("AskUserQuestion"));
        assert!(text.contains("ask_question"));
    }

    #[test]
    fn procedure_runner_selection_distinguishes_hub_and_worker() {
        let settings = config::Settings {
            hub_runner: Some("claude -n {name} {prompt}".to_string()),
            agent_runner: Some("agy --dangerously-skip-permissions -i {prompt}".to_string()),
            ..Default::default()
        };

        assert_eq!(
            runner_for_procedure(&settings, "adj-hub"),
            Some("claude -n {name} {prompt}".to_string())
        );
        assert_eq!(
            runner_for_procedure(&settings, "adj-worker"),
            Some("agy --dangerously-skip-permissions -i {prompt}".to_string())
        );
        assert_eq!(
            runner_for_procedure(&settings, "adj-report"),
            Some("agy --dangerously-skip-permissions -i {prompt}".to_string())
        );

        assert_eq!(
            prompts::resolve_agent(
                None,
                None,
                runner_for_procedure(&settings, "adj-hub").as_deref()
            ),
            prompts::Agent::Claude
        );
        assert_eq!(
            prompts::resolve_agent(
                None,
                None,
                runner_for_procedure(&settings, "adj-worker").as_deref()
            ),
            prompts::Agent::Agy
        );
    }

    #[test]
    fn unsupported_agent_value_is_rejected() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let res = call_tool(
            "adjutant_skill",
            &json!({"name": "adj-hub", "agent": "unknown"}),
        );
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err(),
            "agent must be claude, agy, or generic: unknown"
        );

        let prompt_res = prompt_get(&json!({"name": "adj-hub", "arguments": {"agent": "unknown"}}));
        assert!(prompt_res.is_err());
        assert_eq!(
            prompt_res.unwrap_err(),
            "agent must be claude, agy, or generic: unknown"
        );
    }

    #[test]
    fn skill_and_prompt_resolve_runner_for_specified_worktree() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let _guard = ClientNameGuard;
        let _sandbox = crate::testing::Sandbox::new(
            r#"{
            "defaults": {
                "agentRunner": "agy --dangerously-skip-permissions -i {prompt}"
            }
        }"#,
        );

        let dir = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "remote",
                "add",
                "origin",
                "git@github.com:acme/agy-repo.git",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(&args)
                    .current_dir(dir.path())
                    .status()
                    .unwrap()
                    .success(),
                "git {args:?}"
            );
        }

        let worktree = dir.path().to_str().unwrap();

        let via_prompt = prompt_get(&json!({
            "name": "adj-worker",
            "arguments": { "worktree": worktree }
        }))
        .unwrap();
        let prompt_text = via_prompt["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(prompt_text.contains("ask_question"));
        assert!(!prompt_text.contains("AskUserQuestion"));

        let via_skill = call_tool(
            "adjutant_skill",
            &json!({
                "name": "adj-worker",
                "worktree": worktree
            }),
        )
        .unwrap();
        assert_eq!(via_skill["agent"], "agy");
        let skill_text = via_skill["content"].as_str().unwrap();
        assert!(skill_text.contains("ask_question"));
        assert!(!skill_text.contains("AskUserQuestion"));
    }

    #[test]
    fn install_validates_target_names() {
        assert!(install("unknown-agent").is_err());
        assert!(uninstall("unknown-agent").is_err());
    }
}
