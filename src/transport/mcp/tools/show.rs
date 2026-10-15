use serde_json::{Value, json};

use super::text_param;
use crate::board::aide::{DiffSource, Pick, PrOf, ShowRequest, TaskOf, show};

/// The keys each kind takes, and the ones it must have. A key outside the first list is
/// refused rather than ignored, so a reference that means something else is not shown as if it
/// meant this.
const KINDS: [(&str, &[&str]); 7] = [
    ("gate", &["board", "id"]),
    ("waits", &[]),
    ("read", &["pr", "point", "level"]),
    ("task", &["board", "id", "parent"]),
    ("pr", &["board", "id", "pr"]),
    ("session", &["board", "id"]),
    ("diff", &["pr", "board", "id", "path", "from", "to", "side"]),
];

const KEYS: [&str; 10] = [
    "board", "id", "parent", "pr", "point", "level", "path", "from", "to", "side",
];

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_show",
        "description": "Put something in the display area next to the aide's terminal on the page. Pass a kind and a reference only: the page reads the content from the records and draws it. `board` is always the `slug` from adjutant_work. kind=gate: one gate (`board`, `id`). kind=waits: everything that waits on the person. kind=read: the read-through (AI 下読み) of a PR (`pr` = owner/repo#N), its attention points by default, or one by `point` (counted from 1) or all of one `level`; the answer lists the points as `readThrough`. kind=task: a task (`board`, `id`) or a parent with the progress of its sub-issues (`board`, `parent` = the key of `repos[].parents[]`). kind=pr: where a PR stands, for a task (`board`, `id`) or by `pr` (a PR with a record). kind=session: a worker session (`board`, `id` = `rows[].session.id`). kind=diff: some lines of a diff (`path`, `from`, `to` of at most 60 lines, `side` right or left), of the stored diff of a PR (`pr`) or of an open gate's diff (`board`, `id`). Showing something of another topic clears the area and keeps the old topic by name; something of the same topic is added on top, newest first. It writes nothing but the display area.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["gate", "waits", "read", "task", "pr", "session", "diff"], "description": "What to show." },
                "board": { "type": "string", "description": "The `slug` of the board of the gate, task or session." },
                "id": { "type": "string", "description": "A gate, task or session id." },
                "parent": { "type": "string", "description": "kind=task: a parent's key, `repos[].parents[].key`." },
                "pr": { "type": "string", "description": "A pull request, owner/repo#N." },
                "point": { "type": "integer", "description": "kind=read: one point of the read-through, counted from 1." },
                "level": { "type": "string", "enum": ["attention", "check", "info"], "description": "kind=read: every point of this level." },
                "path": { "type": "string", "description": "kind=diff: the file." },
                "from": { "type": "integer", "description": "kind=diff: the first line, counted from 1." },
                "to": { "type": "integer", "description": "kind=diff: the last line, at most 59 after `from`." },
                "side": { "type": "string", "enum": ["right", "left"], "description": "kind=diff: the new file (right, the default) or the old one." },
            },
            "required": ["kind"],
        },
    })
}

/// A key that is not a string or a whole number is refused, not read as absent.
fn number(args: &Value, key: &str) -> Result<Option<u32>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64().and_then(|n| u32::try_from(n).ok()) {
            Some(n) if n >= 1 => Ok(Some(n)),
            _ => Err(format!("`{key}` is a whole number from 1")),
        },
    }
}

fn need<'a>(args: &'a Value, kind: &str, key: &str, what: &str) -> Result<&'a str, String> {
    text_param(args, key).ok_or_else(|| format!("kind={kind} needs `{key}` ({what})"))
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let kind = text_param(args, "kind").unwrap_or("");
    let Some((_, allowed)) = KINDS.iter().find(|(k, _)| *k == kind) else {
        return Err("kind must be gate, waits, read, task, pr, session or diff".to_string());
    };
    // A key of the wrong type is refused, not read as absent: `level: 1` is not "no level". `null` is absent, as in the
    // other tools, and so is a blank string (`text_param`).
    for key in KEYS {
        let v = &args[key];
        let typed = match key {
            "point" | "from" | "to" => v.is_null() || v.is_number(),
            _ => v.is_null() || v.is_string(),
        };
        if !typed {
            let want = if matches!(key, "point" | "from" | "to") {
                "a whole number from 1"
            } else {
                "a string"
            };
            return Err(format!("`{key}` must be {want}"));
        }
        let given = text_param(args, key).is_some() || v.is_number();
        if given && !allowed.contains(&key) {
            return Err(format!("kind={kind} takes no `{key}`"));
        }
    }
    let get = |key: &str| text_param(args, key).map(str::to_string);
    let board = |what: &str| need(args, kind, "board", what).map(str::to_string);
    let request =
        match kind {
            "gate" => ShowRequest::Gate {
                board: board("the slug of the gate's board")?,
                id: need(args, kind, "id", "the gate id")?.to_string(),
            },
            "waits" => ShowRequest::Waits,
            "read" => {
                let (point, level) = (number(args, "point")?, get("level"));
                let pick = match (point, level) {
                    (Some(_), Some(_)) => {
                        return Err("kind=read takes `point` or `level`, not both".to_string());
                    }
                    (Some(n), None) => Pick::Point(n),
                    (None, Some(level)) => Pick::Level(level),
                    (None, None) => Pick::All,
                };
                ShowRequest::Read {
                    pr: need(args, kind, "pr", "owner/repo#N")?.to_string(),
                    pick,
                }
            }
            "task" => {
                let of = match (get("id"), get("parent")) {
                    (Some(_), Some(_)) => {
                        return Err("kind=task takes `id` or `parent`, not both".to_string());
                    }
                    (Some(id), None) => TaskOf::Id(id),
                    (None, Some(parent)) => TaskOf::Parent(parent),
                    (None, None) => {
                        return Err("kind=task needs `id` (a task) or `parent` (a parent's key)"
                            .to_string());
                    }
                };
                ShowRequest::Task {
                    board: board("the slug of the task's board")?,
                    of,
                }
            }
            "pr" => {
                let of = match (get("pr"), get("board"), get("id")) {
                    (Some(pr), None, None) => PrOf::Pr(pr),
                    (None, Some(board), Some(id)) => PrOf::Task { board, id },
                    _ => {
                        return Err(
                            "kind=pr needs `pr` (owner/repo#N), or `board` and `id` (a task)"
                                .to_string(),
                        );
                    }
                };
                ShowRequest::Pr { of }
            }
            "session" => ShowRequest::Session {
                board: board("the slug of the session's board")?,
                id: need(args, kind, "id", "rows[].session.id")?.to_string(),
            },
            _ => {
                let source = match (get("pr"), get("board"), get("id")) {
                    (Some(pr), None, None) => DiffSource::Pr(pr),
                    (None, Some(board), Some(id)) => DiffSource::Gate { board, id },
                    _ => return Err(
                        "kind=diff needs `pr` (owner/repo#N), or `board` and `id` (an open gate)"
                            .to_string(),
                    ),
                };
                ShowRequest::Diff {
                    source,
                    path: need(args, kind, "path", "the file")?.to_string(),
                    from: number(args, "from")?.ok_or("kind=diff needs `from` (a line, from 1)")?,
                    to: number(args, "to")?.ok_or("kind=diff needs `to` (a line, from `from`)")?,
                    side: get("side"),
                }
            }
        };
    let shown = show(&crate::registry::state_root(None), request)?;
    let mut out = serde_json::to_value(&shown.display).map_err(|e| e.to_string())?;
    out["cleared"] = json!(shown.cleared);
    if shown.replaced_unreadable {
        out["replacedUnreadable"] = json!(true);
    }
    if let Some(read) = shown.read_through {
        out["readThrough"] = read;
    }
    Ok(out)
}
