//! The MCP tool schemas.

use serde_json::{Value, json};

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

pub(super) fn tool_definitions() -> Value {
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
                    "kind": { "type": "string", "description": "report (default) | question | answer | ack | done | needs-user | request | next | gate | jules-pr | jules-review" },
                    "wake": { "type": "boolean", "description": "Whether to wake the hub. Defaults to automatic: wakes for actionable messages (reports, answers, done, requests, next, gate, jules-pr, jules-review); delivers without waking for questions, needs-user, ack, or when sending to yourself." },
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
            "description": "Leave a message for the worker in a worktree, and wake it if it is sitting there. This is the hub-to-worker direction: the address is the worktree, not a session, so it reaches whatever agent is working there whatever it is doing. Start the subject with `[question <id>]` when you need an answer back — that marker is what tells the worker it may reply.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "worktree": { "type": "string", "description": "Absolute path of the worktree." },
                    "subject": { "type": "string", "description": "One line stating the point. `[question <id>]` asks for an answer; `[ack]` acknowledges; anything else is a notice." },
                    "body": { "type": "string", "description": "The message. Markdown." },
                    "from": { "type": "string", "description": "Who is speaking (default: this repository's hub name)." },
                    "wake": { "type": "boolean", "description": "Whether to wake the worker. Defaults to automatic based on the subject: wakes for `[question <id>]` and gate answers; delivers without waking for `[ack]` and plain notices." },
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
                    "cwd": cwd_property(),
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
                    "openedBy": { "type": "string", "enum": ["worker", "hub"], "description": "For the hub only; a worker leaves it out. plan only: `hub` when the hub opens the plan of a task handed to Jules, so the answer reaches the hub's inbox rather than the worktree's outbox. Needs task. Default: worker." },
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
                    "commands": { "type": "array", "items": { "type": "object" }, "description": "verify: command, result (pass | fail), time, output, attempts (runs it took; above 1 when it failed first)." },
                    "manual": { "type": "array", "items": { "type": "string" }, "description": "verify: the checks left for a person." },
                    "stoppedBy": { "type": "array", "items": { "type": "string" }, "description": "diff / verify that waits: the rules that made it stop rather than be kept as a record — round-limit, verify-failed, manual-check, unsure, stop-at." },
                    "repo": repo_property(),
                    "hub": hub_property(),
                    "cwd": cwd_property(),
                },
                "required": ["kind", "title"],
            },
        },
        {
            "name": "adjutant_gate_close",
            "description": "Archive an open gate without delivering an answer to the outbox, the same as `adj gate close`: used when the question was answered directly in the terminal tab or rendered moot, so the gate does not stay on the board waiting. When the person answered in the terminal, pass `terminal: true` and put what they decided in `comment`: the gate is then recorded as answered in the terminal.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The gate id to close." },
                    "comment": { "type": "string", "description": "Optional reason for closing. With `terminal`, what the person decided." },
                    "terminal": { "type": "boolean", "description": "The person answered this gate in your terminal: record it as answered in the terminal (put what they decided in comment)." },
                    "repo": repo_property(),
                    "hub": hub_property(),
                    "cwd": cwd_property(),
                },
                "required": ["id"],
            },
        },
        {
            "name": "adjutant_refresh",
            "description": "Bring the task records up to date with their pull requests, the same as `adj task refresh`: every record with a `pr` that is not done or cancelled is read with `gh` in one query, its state is kept on the record, and the ones whose PR was merged are moved to done. A PR still open, one `gh` cannot read, or one closed without merging is left alone and listed instead (a closed one is the person's to decide, never cancelled here) — say those to the person rather than deciding for them. Each entry says whose `turn` it is. A hub calls this once at startup.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_property(), "hub": hub_property(), "cwd": cwd_property() },
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
                    "agent": { "type": "string", "enum": ["claude", "claude-code", "agy", "antigravity", "generic", "codex"], "description": "Target agent format: claude (claude-code) | agy (antigravity) | generic (codex). Defaults to auto-detect." },
                    "worktree": { "type": "string", "description": "Path to the worktree or repository root. Defaults to the server's working directory." },
                },
                "required": ["name"],
            },
        },
    ]})
}
