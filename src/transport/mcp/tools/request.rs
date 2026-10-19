use serde_json::{Value, json};

use super::{board_context, text_param};
use crate::task::{Kind, NewTask, Status};

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_request",
        "description": "Pass a request for work to a hub, the way the board's 「依頼」 form does: a task record is written (queued, its words unread) and handed to the hub's inbox, which reads it and asks the person on a gate before it starts anything. Call it ONLY after the person has said yes in the chat to exactly what will be sent: which repository, which hub and its parent, and the words. `board` is the hub's `slug` from adjutant_work (`repos[].hubs[].slug`, or for a parent `repos[].parents[].hub`), `repo` its owner/name; the two must agree or nothing is written. `parent` is the parent's URL (`repos[].parents[].url`) and is required for a hub of a parent (refused without it), and is left out for the repository's own hub. The confirmation you show the person must name the hub and the parent (the answer's `hubName` and `hubKey` say which hub it was), so a mismatch between them is caught there. `body` is the person's own words, as they said them; do not add a plan or a reading. A hub that is not running is not an error: the request waits in its inbox (`present: false`). The code-review hub keeps no tasks and is refused (a PR to read goes through 「AI 下読み」). The answer says the task's id; 「いまの仕事」 shows its progress.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "repo": { "type": "string", "description": "owner/name of the repository the hub works on." },
                "board": { "type": "string", "description": "The `slug` of the hub's board, from adjutant_work." },
                "body": { "type": "string", "description": "The person's words, as said." },
                "title": { "type": "string", "description": "A title for the card. Leave it out: the first line of the words is used." },
                "parent": { "type": "string", "description": "The parent task of a parent's hub: its URL, `repos[].parents[].url`, or its key." },
            },
            "required": ["repo", "board", "body"],
        },
    })
}

fn string_key(args: &Value, key: &str) -> Result<(), String> {
    match &args[key] {
        Value::Null | Value::String(_) => Ok(()),
        _ => Err(format!("`{key}` must be a string")),
    }
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    for key in ["repo", "board", "body", "title", "parent"] {
        string_key(args, key)?;
    }
    let repo = text_param(args, "repo").ok_or("a request needs `repo` (owner/name)")?;
    let body = text_param(args, "body").unwrap_or("");
    let parent = text_param(args, "parent");
    let board = text_param(args, "board").ok_or("a request needs `board`: the hub's slug")?;
    // Said before the board is looked up: the review hub is not in the address book, and "no such
    // board" would send the caller looking for a spelling that does not exist.
    if board == crate::kernel::identity::REVIEW_SLUG {
        return Err("the code-review hub keeps no tasks and takes no request: a PR to be read goes through 「AI 下読み」 (adjutant_reviews)".to_string());
    }
    let ctx = board_context(args)?;
    if ctx.repo.is_review() {
        return Err("the code-review hub keeps no tasks and takes no request".to_string());
    }
    if !ctx.repo.nwo.eq_ignore_ascii_case(repo) {
        return Err(format!(
            "that board belongs to {}, not {repo}: nothing was written",
            ctx.repo.nwo
        ));
    }
    // A parent's hub takes the request for that parent: without it the task would be filed under
    // the hub with no parent, and the hub would not know which issue it is a child of.
    if ctx.repo.hub.is_some() && parent.is_none() {
        return Err("this is the hub of a parent task: say which parent with `parent`".to_string());
    }
    crate::task::check_request(body, parent)?;
    let new = NewTask {
        title: text_param(args, "title").map(str::to_string),
        body: body.to_string(),
        parent: parent.map(str::to_string),
        status: Status::Queued,
        // The same as the board's form: only the person's words, which the hub reads and
        // confirms on a gate before it starts.
        needs_reading: true,
        kind: Kind::Start,
        ..NewTask::default()
    };
    let (task, handed) = crate::task::create(&ctx, new, true)?;
    let (present, woken) = handed
        .as_ref()
        .map(|h| (h.is_present(), h.was_woken()))
        .unwrap_or((false, false));
    Ok(json!({
        "repo": ctx.repo.nwo,
        "board": ctx.repo.slug,
        "hubName": ctx.repo.hub_name,
        "hubKey": ctx.repo.hub,
        "task": { "id": task.id, "title": task.title, "status": task.status },
        "present": present,
        "woken": woken,
    }))
}
