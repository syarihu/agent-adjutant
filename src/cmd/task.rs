//! `adj task` — the records the board is a view of.
//!
//! The dashboard is not the only caller. The hub writes back here when it picks a task up
//! ("依頼が届いたら" Step 5 replies to the requester, and for a request from the dashboard
//! the requester is a file rather than a session), and a worker or a script can add one
//! without a browser. So the verbs live here and the HTTP layer calls them, rather than the
//! other way round.

use serde_json::{Value, json};

use super::{Context, Delivered};
use crate::config;
use crate::messaging::{self, Message};
use crate::task::{self, Status, Task};

use std::path::PathBuf;

pub fn dir(ctx: &Context) -> PathBuf {
    task::dir(&messaging::state_dir(), &ctx.repo.slug)
}

fn stamp() -> String {
    messaging::utc_stamp(messaging::now_secs())
}

/// Derive a card title from the input title or the first non-empty line of the body.
fn derive_title(input: &Value) -> Option<String> {
    if let Some(title) = string(input, "title") {
        return Some(title);
    }
    let body = string(input, "body")?;
    let first_line = body.lines().map(str::trim).find(|l| !l.is_empty())?;
    let title: String = first_line.chars().take(80).collect();
    if title.is_empty() { None } else { Some(title) }
}

/// Refuse the two values a person types on the board that the hub later puts on a command
/// line: the worktree name becomes a path and a branch, and the issue URL is quoted as it is.
/// Checked here, where they come in, rather than in every command the procedures write — an
/// apostrophe in either would close the quote around it and run the rest as shell.
fn check_typed_values(input: &Value) -> Result<(), String> {
    // An empty field is a form left blank, not a value.
    let typed = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
    };
    if let Some(name) = typed("worktreeName") {
        let charset = name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !charset {
            return Err(format!(
                "a worktree name may only use letters, digits, '.', '_' and '-': {name}"
            ));
        }
        // Whether it can name a branch is git's to say, not a list kept here: saved, a name git
        // refuses would sit in the queue until the hub failed to create its branch. Asked with
        // the name alone, which `branchPattern` puts after a prefix — a name that fails on its
        // own fails there too. A leading '-' is refused first so git cannot read it as a flag.
        let branchable = !name.starts_with('-')
            && std::process::Command::new("git")
                .args(["check-ref-format", "--branch", name])
                .output()
                .is_ok_and(|out| out.status.success());
        if !branchable {
            return Err(format!(
                "git cannot name a branch after this worktree name: {name}"
            ));
        }
    }
    if let Some(url) = typed("issueUrl") {
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"));
        // The host is what is left of the authority once a port is taken off. Checked as a
        // name rather than as "some text before the path", which `https://:8080/` passed.
        // A port, when there is one, is digits.
        let authority = rest
            .and_then(|r| r.split(['/', '?', '#']).next())
            .unwrap_or("");
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        let named = !host.is_empty()
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
            && port.is_none_or(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        let plain = !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "'\"`$\\;&|<>(){}".contains(c));
        if !named || !plain {
            return Err(format!("not an issue URL: {url}"));
        }
    }
    Ok(())
}

/// Write a new record, and hand it over if it was created already queued.
pub fn create(ctx: &Context, input: &Value) -> Result<(Task, Option<Delivered>), String> {
    let stamp = stamp();
    let title = derive_title(input).ok_or("a task needs content or a title")?;
    // Before the id is claimed: claiming writes a reservation, and a refusal after it would
    // leave that behind.
    check_typed_values(input)?;
    let id = task::claim_id(&dir(ctx), &stamp, &title)?;
    let mut defaults = with_defaults(input, &id, &stamp)?;
    defaults["title"] = json!(title);
    // An instruction to this function rather than part of the record.
    let hand = defaults
        .as_object_mut()
        .and_then(|fields| fields.remove("handOver"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut task: Task = serde_json::from_value(defaults).map_err(|e| format!("bad task: {e}"))?;
    task.worktree = task.worktree.as_deref().map(resolved_worktree);
    task.order = next_order(ctx);

    // Written before the message is sent, and never the other way round: the record is what
    // the hub checks when it is about to act, so a message that arrived first would name a
    // task nothing can look up.
    task::save(&dir(ctx), &task)?;
    let handed = match task.status {
        Status::Queued if hand => Some(hand_over(ctx, &task)?),
        _ => None,
    };
    Ok((task, handed))
}

/// A worktree path as `git worktree list` prints it: absolute, symlinks resolved. The board
/// matches a task to its worker by this string, and `./wt` or `/tmp/…` against git's
/// `/private/tmp/…` would read as a worker that is not there. A path that does not exist
/// (yet) is resolved through the nearest part of it that does, with the rest put back on,
/// so that it is absolute — against the directory of the command giving it, not of some
/// later one — and matches what git prints once the worktree is created there.
fn resolved_worktree(path: &str) -> String {
    let path = config::expand_home(path);
    let absolute = std::path::absolute(&path).unwrap_or(path);
    let mut existing = absolute.as_path();
    let mut rest = Vec::new();
    let resolved = loop {
        if let Ok(real) = existing.canonicalize() {
            break rest
                .iter()
                .rev()
                .fold(real, |acc: std::path::PathBuf, part| acc.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            // Nothing of it exists, not even the root: keep what we were given.
            _ => break absolute.clone(),
        }
    };
    resolved.to_string_lossy().to_string()
}

/// One of a record's text fields as an update gives it. `null` and `""` clear it; anything
/// that is not a string is refused rather than read as "clear" — `{"pr": 42}` from a mistaken
/// caller would otherwise wipe the URL it meant to set.
fn text_field(key: &str, value: &Value) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(v) if v.is_empty() => Ok(None),
        Value::String(v) => Ok(Some(v.clone())),
        other => Err(format!("{key} has to be a string or null, not {other}")),
    }
}

/// Change a record, and hand it over if this is the change that queued it.
/// Hold the write lock of one task record until the returned handle is dropped.
///
/// A change is a read of the whole record and a write of the whole record, and two of them
/// at once — the hub updating a task while a gate for it is answered on the board — would
/// each write back what they read, and the later would undo the earlier. An advisory lock
/// on an open file, like the dispatch lock: the system lets it go if its holder dies, so
/// there is nothing to clear by hand. Held only across a load and a save, so waiting on it
/// is short.
pub fn lock_task(ctx: &Context, id: &str) -> Result<std::fs::File, String> {
    let dir = dir(ctx);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    // Beside the record, and not named `.json`, so the listing never reads it as a task.
    let path = dir.join(format!("{id}.lock"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    file.lock()
        .map_err(|e| format!("cannot lock {}: {e}", path.display()))?;
    Ok(file)
}

pub fn update(ctx: &Context, id: &str, input: &Value) -> Result<(Task, Option<Delivered>), String> {
    let lock = lock_task(ctx, id)?;
    let mut task = task::load(&dir(ctx), id)?;
    let was = task.status;

    if let Some(status) = string(input, "status") {
        task.status = Status::parse(&status).ok_or(format!("no such status: {status}"))?;
    }
    if let Some(order) = input.get("order").and_then(Value::as_u64) {
        task.order = order as u32;
    }
    // Set by the hub when a person approved a task that asked to be confirmed first, so that
    // being turned away for a slot afterwards does not put the same question to them again.
    if let Some(auto_start) = input.get("autoStart").and_then(Value::as_bool) {
        task.auto_start = auto_start;
    }
    for (key, field) in [
        ("worktree", &mut task.worktree),
        ("issue", &mut task.issue),
        ("pr", &mut task.pr),
        ("note", &mut task.note),
    ] {
        if let Some(value) = input.get(key) {
            // An explicit `null` clears; an absent key leaves it alone. Without the
            // distinction there is no way to take back a worktree the hub wrote down. An
            // empty string clears too, since a command line has no way to say `null` — and a
            // "waiting for a slot" note has to go once the worker starts.
            *field = text_field(key, value)?;
        }
    }
    // Only a worktree given in this update: one already stored was resolved when it was
    // given, against the directory of the command that gave it, and re-resolving it here
    // would read it against wherever this update happens to be run from.
    if input.get("worktree").is_some() {
        task.worktree = task.worktree.as_deref().map(resolved_worktree);
    }
    task.updated_at = stamp();
    task::save(&dir(ctx), &task)?;
    drop(lock);

    // Handing over is a *transition*, not a status: re-sending on every save would put one
    // task in the inbox once for every time somebody dragged its card.
    // Not when the hub is the one queueing it: the inbox it would land in is its own. That is
    // a resumed worker turned away for a slot, whose record was `dispatched` or `pr`.
    let hand = input
        .get("handOver")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let handed = if hand && was != Status::Queued && task.status == Status::Queued {
        Some(hand_over(ctx, &task)?)
    } else {
        None
    };
    Ok((task, handed))
}

/// Put the task in the hub's inbox and poke its tab — the same delivery `adj send` performs,
/// through the same code, so waking and notifying cannot drift between the two callers.
pub fn hand_over(ctx: &Context, task: &Task) -> Result<Delivered, String> {
    let message = Message {
        from: "dashboard".to_string(),
        // Deliberately none. The sender is a person at a browser, not a worktree, and a
        // `worktree:` header here would name whichever directory the server was started in
        // — which the hub would then act on as if a worker had reported from it.
        worktree: None,
        kind: "request".to_string(),
        subject: task.title.clone(),
        body: task::render_request(task),
    };
    super::deliver_to_hub(ctx, &message)
}

/// Ask the hub to start the next queued task if a worker slot is free.
///
/// For the one case the hub's own procedure cannot see: a worker that died without sending
/// `done`. Its slot came free and nothing woke the hub to say so. A message rather than a
/// bare wake, because a hub that is woken and finds its inbox empty goes straight back to
/// waiting — and one that is not running should find this waiting when it starts.
pub fn nudge(ctx: &Context) -> Result<Delivered, String> {
    let message = Message {
        from: "dashboard".to_string(),
        // None, for the reason `hand_over` gives.
        worktree: None,
        kind: "next".to_string(),
        subject: "start the next queued task if a worker slot is free".to_string(),
        body: String::new(),
    };
    super::deliver_to_hub(ctx, &message)
}

fn next_order(ctx: &Context) -> u32 {
    task::list(&dir(ctx))
        .iter()
        .map(|t| t.order)
        .max()
        .unwrap_or(0)
        + 1
}

/// Fill in what a caller may leave out, so a form can post the fields a person filled and
/// nothing else.
fn with_defaults(input: &Value, id: &str, stamp: &str) -> Result<Value, String> {
    let mut value = input.clone();
    let fields = value.as_object_mut().ok_or("expected an object")?;
    fields.insert("id".to_string(), json!(id));
    fields.insert("createdAt".to_string(), json!(stamp));
    fields.insert("updatedAt".to_string(), json!(stamp));
    fields.entry("kind").or_insert(json!("start"));
    fields.entry("doneWhen").or_insert(json!("pr"));
    fields.entry("autoStart").or_insert(json!(true));
    fields.entry("status").or_insert(json!("backlog"));
    fields.entry("body").or_insert(json!(""));
    Ok(value)
}

fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// ── the subcommands ──────────────────────────────────────────────────

pub struct AddArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub kind: &'a str,
    pub done_when: &'a str,
    pub issue_url: Option<&'a str>,
    pub base: Option<&'a str>,
    pub parent: Option<&'a str>,
    pub worktree_name: Option<&'a str>,
    pub ask_first: bool,
    pub queue: bool,
    pub waiting_in: Option<&'a str>,
    pub json: bool,
}

pub fn add(args: &AddArgs<'_>) -> Result<(), String> {
    let ctx = super::context(args.repo, args.hub)?;
    let body = super::read_body(args.body)?;
    let mut input = json!({
        "body": body,
        "kind": args.kind,
        "doneWhen": args.done_when,
        "issueUrl": args.issue_url,
        "base": args.base,
        "parent": args.parent,
        "worktreeName": args.worktree_name,
        "autoStart": !args.ask_first,
        "status": if args.queue || args.waiting_in.is_some() { "queued" } else { "backlog" },
    });
    // The hub writing down work it has prepared a worktree for: before it starts the worker,
    // so the brief can carry the id, or after `adj work` turned it away for want of a slot.
    // Queued either way, so a free slot can take it; but not handed over, because the inbox
    // it would land in is the caller's own, and a hub that messages itself is woken mid-turn
    // to be told what it just did.
    if let Some(worktree) = args.waiting_in {
        let worktree = config::expand_home(worktree).to_string_lossy().to_string();
        input["worktree"] = json!(worktree);
        input["handOver"] = json!(false);
    }
    if let Some(title) = args.title {
        input["title"] = json!(title);
    }
    let (task, handed) = create(&ctx, &input)?;
    if args.json {
        println!(
            "{}",
            json!({ "task": task, "handed": handed_json(&handed) })
        );
        return Ok(());
    }
    println!("{} — {}", task.id, task.title);
    say_where_it_went(&ctx, &task, &handed);
    Ok(())
}

pub struct UpdateArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub id: &'a str,
    pub status: Option<&'a str>,
    pub order: Option<u32>,
    pub worktree: Option<&'a str>,
    pub issue: Option<&'a str>,
    pub pr: Option<&'a str>,
    pub note: Option<&'a str>,
    pub auto_start: Option<bool>,
    pub no_hand_over: bool,
    pub json: bool,
}

pub fn update_cmd(args: &UpdateArgs<'_>) -> Result<(), String> {
    let ctx = super::context(args.repo, args.hub)?;
    let mut input = json!({});
    let fields = input.as_object_mut().expect("just built");
    // `--note -` reads it from stdin: a note is often text from elsewhere — an error, a
    // comment typed on the board — and does not belong inside quotes on a command line.
    let note = args.note.map(super::dash_is_stdin).transpose()?;
    for (key, value) in [
        ("status", args.status),
        ("worktree", args.worktree),
        ("issue", args.issue),
        ("pr", args.pr),
        ("note", note.as_deref()),
    ] {
        if let Some(value) = value {
            fields.insert(key.to_string(), json!(value));
        }
    }
    if let Some(order) = args.order {
        fields.insert("order".to_string(), json!(order));
    }
    if let Some(auto_start) = args.auto_start {
        fields.insert("autoStart".to_string(), json!(auto_start));
    }
    if args.no_hand_over {
        fields.insert("handOver".to_string(), json!(false));
    }
    let (task, handed) = update(&ctx, args.id, &input)?;
    if args.json {
        println!(
            "{}",
            json!({ "task": task, "handed": handed_json(&handed) })
        );
        return Ok(());
    }
    println!("{} — {} ({})", task.id, task.title, task.status.as_str());
    say_where_it_went(&ctx, &task, &handed);
    Ok(())
}

pub fn list(
    repo: Option<&str>,
    hub: Option<&str>,
    status: Option<&str>,
    worktree: Option<&str>,
    as_json: bool,
) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    let wanted = match status {
        Some(text) => Some(Status::parse(text).ok_or(format!("no such status: {text}"))?),
        None => None,
    };
    // Compared resolved: the hub names the worktree as `git worktree list` printed it, and
    // the record holds whatever path it was written with.
    let resolved = |path: &str| {
        let path = config::expand_home(path);
        path.canonicalize().unwrap_or(path)
    };
    let at = worktree.map(resolved);
    let tasks: Vec<Task> = task::list(&dir(&ctx))
        .into_iter()
        .filter(|t| wanted.is_none_or(|w| t.status == w))
        .filter(|t| {
            at.as_ref()
                .is_none_or(|at| t.worktree.as_deref().map(resolved).as_ref() == Some(at))
        })
        .collect();

    if as_json {
        println!("{}", json!(tasks));
        return Ok(());
    }
    if tasks.is_empty() {
        println!("No tasks for {}.", ctx.repo.nwo);
        return Ok(());
    }
    for task in &tasks {
        println!(
            "{:<10} {:>3}  {}  {}",
            task.status.as_str(),
            task.order,
            task.id,
            task.title
        );
    }
    Ok(())
}

pub fn show(repo: Option<&str>, hub: Option<&str>, id: &str) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    let task = task::load(&dir(&ctx), id)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&task).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn handed_json(handed: &Option<Delivered>) -> Value {
    match handed {
        Some(d) => json!({
            "present": d.delivery.present,
            "woken": d.woken,
            "path": d.delivery.path.display().to_string(),
        }),
        None => Value::Null,
    }
}

fn say_where_it_went(ctx: &Context, task: &Task, handed: &Option<Delivered>) {
    let Some(handed) = handed else {
        println!(
            "Kept in the backlog. Nothing is in {}'s inbox yet.",
            ctx.repo.hub_name
        );
        return;
    };
    println!(
        "handed to {}: {}",
        ctx.repo.hub_name,
        handed.delivery.path.display()
    );
    match (handed.delivery.present, handed.woken) {
        (true, true) => println!("Woke the hub; it will pick this up."),
        (true, false) => {
            println!("The hub is running; it will pick this up the next time it checks its inbox.")
        }
        (false, _) => println!(
            "The hub is not running. Waiting in its inbox for the next time it starts (task {}).",
            task.id
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_taken_verbatim_when_present() {
        let input = json!({ "title": "explicit title", "body": "first line\nsecond line" });
        assert_eq!(derive_title(&input).as_deref(), Some("explicit title"));
    }

    #[test]
    fn title_is_derived_from_the_first_non_empty_line_of_the_body() {
        let input =
            json!({ "body": "\n\n  Fix the flaky network retry logic  \nand more details" });
        assert_eq!(
            derive_title(&input).as_deref(),
            Some("Fix the flaky network retry logic")
        );
    }

    #[test]
    fn title_is_capped_at_eighty_characters() {
        let long_line = "a".repeat(120);
        let input = json!({ "body": long_line });
        let derived = derive_title(&input).expect("derived");
        assert_eq!(derived.len(), 80);
    }

    #[test]
    fn a_text_field_is_cleared_by_null_or_empty_and_refused_as_anything_else() {
        assert_eq!(
            text_field("pr", &json!("https://x/pull/1"))
                .unwrap()
                .as_deref(),
            Some("https://x/pull/1")
        );
        assert_eq!(text_field("pr", &json!(null)).unwrap(), None);
        assert_eq!(text_field("note", &json!("")).unwrap(), None);
        for bad in [json!(42), json!(true), json!(["a"]), json!({"a": 1})] {
            assert!(text_field("pr", &bad).is_err(), "{bad} was taken");
        }
    }

    #[test]
    fn empty_title_and_body_produce_nothing() {
        let input = json!({ "title": "   ", "body": "   \n\n  " });
        assert!(derive_title(&input).is_none());
    }
}
