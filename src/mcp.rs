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
                "arguments": [{
                    "name": "arguments",
                    "description": "Free text passed to the procedure (what you found, which task to pick up). Optional.",
                    "required": false,
                }],
            })
        })
        .collect();
    json!({ "prompts": list })
}

fn prompt_get(params: &Value) -> Result<Value, String> {
    let name = params["name"].as_str().unwrap_or("");
    let prompt = prompts::find(name).ok_or_else(|| format!("Unknown prompt: {name}"))?;
    let arguments = params["arguments"]["arguments"].as_str().unwrap_or("");
    Ok(json!({
        "description": prompts::description(prompt),
        "messages": [{
            "role": "user",
            "content": { "type": "text", "text": prompts::render(prompt, arguments) },
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
            "description": "The resolved configuration for a repository: task sources (always an array, flat shorthand already expanded), defaults already merged, and the machine settings (terminal, agent runner, notification, editor). Also reports warnings about the config rather than failing on it. Call this once at startup instead of reading the config file.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_property(), "cwd": cwd_property() },
            },
        },
        {
            "name": "adjutant_hub_status",
            "description": "The hub session name for a repository, and whether that hub is currently running. The name is the address a report is sent to; derive it here rather than reconstructing it, so both sides always agree.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_property(), "cwd": cwd_property() },
            },
        },
        {
            "name": "adjutant_send",
            "description": "Deliver a message to a repository's hub. Never fails for want of a listener: if the hub is not running the message waits in its inbox and is picked up when it next starts, and the reply says which of the two happened. Use for bug reports found mid-task, answers to a hub's question, acknowledgements, and telling the hub a task is finished so it can close this tab and clear the worktree.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "body": { "type": "string", "description": "The message. Markdown; keep it under 30 lines — the hub reshapes it into an issue." },
                    "subject": { "type": "string", "description": "One line stating the conclusion. This is all a human sees in a listing." },
                    "from": { "type": "string", "description": "Who is sending: your session or worktree name." },
                    "kind": { "type": "string", "description": "report (default) | question | answer | ack | done | needs-user" },
                    "repo": repo_property(),
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
            "name": "adjutant_skill",
            "description": "The full text of one of adjutant's procedures: adj-hub (running the hub), adj-worker (taking a task from brief to handover), adj-report (handing a bug you found to the hub). Same text the MCP prompts serve; use this tool when prompts are not available to you.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "enum": ["adj-hub", "adj-worker", "adj-report"] },
                    "arguments": { "type": "string", "description": "Free text substituted into the procedure where it asks for it." },
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
    repo::resolve_in(
        cwd.as_deref(),
        args["repo"].as_str().filter(|s| !s.is_empty()),
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
                        .map(|e| json!({"name": e.name, "from": e.from, "kind": e.kind, "subject": e.subject}))
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
        "adjutant_skill" => {
            let name = args["name"].as_str().unwrap_or("");
            let prompt = prompts::find(name).ok_or_else(|| {
                format!("no such procedure: {name} (adj-hub / adj-worker / adj-report)")
            })?;
            Ok(json!({
                "name": prompt.name,
                "description": prompts::description(prompt),
                "content": prompts::render(prompt, args["arguments"].as_str().unwrap_or("")),
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

pub fn run_server() -> Result<(), Box<dyn std::error::Error>> {
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
        "initialize" => respond(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {}, "prompts": {} },
                "serverInfo": { "name": SERVER_NAME, "version": VERSION },
                "instructions": prompts::INSTRUCTIONS,
            }),
        ),
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
        other => Err(format!("target must be claude-code or json: {other}")),
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
        other => Err(format!("target must be claude-code: {other}")),
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
        // machine — harmless while all seven happen to be read-only, and a fixture dropped
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

    #[test]
    fn the_skill_tool_serves_the_same_text_as_the_prompt() {
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
}
