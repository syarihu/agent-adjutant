use crate::gate::open as gate_open_payload;
use crate::kernel::identity;
use crate::transport::wording::open_json as gate_open_json;
use serde_json::{Value, json};

use super::{cwd_param, cwd_property, hub_property, repo_property, resolve_repo};

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_gate_open",
        "description": "Put something in front of a person on the board, the same as `adj gate open`: the arguments are the gate's payload. By default the gate waits — end your turn and read `adjutant_outbox` when woken; `server` says whether a board is up to see it at all, and when it is `down` ask in your own tab instead. With `wait: false` (diff and verify only) it is kept as a record: nobody is asked, and you go on with your work. A gate that waits needs `focus`, a plan needs `steps`, a result needs `sections` or a body, and a `quick` gate needs `choices` and nothing to read; one that lacks what it needs is refused and nothing is written.",
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
                "focus": { "type": "string", "description": "Required when the gate waits: the question, enough to answer from alone." },
                "decided": { "type": "string", "description": "What is settled. Shown folded away. Not the plan's steps." },
                "unsure": { "type": "string", "description": "Where your confidence ran out." },
                "body": { "type": "string", "description": "A long report kept as it is; prefer sections." },
                "run": { "type": "string", "description": "How to run it, for verify." },
                "diff": { "type": "string", "description": "The diff, for diff." },
                "choices": { "type": "array", "items": { "type": "object" }, "description": "Designs to choose between: id, label, why, points, recommended, comment (a draft to send if this one is picked and the person typed none)." },
                "options": { "type": "array", "items": { "type": "string" }, "description": "The buttons. Default: by kind." },
                "rounds": { "type": "integer", "description": "How many times this same point has gone back and forth with a person." },
                "steps": { "type": "array", "items": { "type": ["object", "string"] }, "description": "plan: required. The plan, one step each, in order: text, detail (a line under it)." },
                "sections": { "type": "array", "items": { "type": "object" }, "description": "What was found, in order: heading (you pick it to fit: 原因, 対応策, 内訳 ...), body (markdown), stage (found, the default, or plan). A result gate needs these or body. Not 懸念点: that is unsure." },
                "quick": { "type": "boolean", "description": "true for a fixed confirmation answered with one press: needs choices, facts checked first, nothing to read." },
                "problem": { "type": "string", "description": "plan: what is wrong today, from the request and the issue." },
                "goal": { "type": "string", "description": "plan: what done looks like." },
                "reviewRounds": { "type": "array", "items": { "type": "object" }, "description": "diff: one per review round — engine, must, want, scope, falsePositives." },
                "findings": { "type": "array", "items": { "type": "object" }, "description": "diff: severity (must | want | scope), location, text, outcome (open | fixed | declined), reason when declined." },
                "commands": { "type": "array", "items": { "type": "object" }, "description": "verify: command, result (pass | fail), time, output, attempts (runs it took; above 1 when it failed first)." },
                "manual": { "type": "array", "items": { "type": "string" }, "description": "verify: the checks left for a person." },
                "report": { "type": "string", "description": "A self-contained HTML file (no external scripts, styles, fonts or images; images inline as data: URLs), relative to the worktree or absolute. Copied when the gate opens (up to 5 MiB, UTF-8) and shown on the board in a sandbox. One per gate; not on a quick gate." },
                "stoppedBy": { "type": "array", "items": { "type": "string" }, "description": "diff / verify that waits: the rules that made it stop rather than be kept as a record — round-limit, verify-failed, manual-check, unsure, stop-at." },
                "repo": repo_property(),
                "hub": hub_property(),
                "cwd": cwd_property(),
            },
            "required": ["kind", "title"],
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let ctx = crate::registry::context_of(resolve_repo(args)?)?;
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
        let here = identity::current_worktree(cwd_param(args).as_deref())
            .ok_or("not inside a worktree: pass worktree or cwd")?;
        fields.insert("worktree".to_string(), json!(here));
    }
    let (gate, served) = gate_open_payload(&ctx, crate::gate::GateRequest::from_json(&payload)?)?;
    Ok(gate_open_json(&ctx, &gate, served))
}
