//! `adj task` — the records the board is a view of.
//!
//! The dashboard is not the only caller. The hub writes back here when it picks a task up
//! ("依頼が届いたら" Step 5 replies to the requester, and for a request from the dashboard
//! the requester is a file rather than a session), and a worker or a script can add one
//! without a browser. So the verbs live here and the HTTP layer calls them, rather than the
//! other way round.

use serde_json::{Value, json};

use super::{Context, Delivered};
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

/// Write a new record, and hand it over if it was created already queued.
pub fn create(ctx: &Context, input: &Value) -> Result<(Task, Option<Delivered>), String> {
    let stamp = stamp();
    let title = derive_title(input).ok_or("a task needs content or a title")?;
    let id = task::claim_id(&dir(ctx), &stamp, &title)?;
    let mut defaults = with_defaults(input, &id, &stamp)?;
    defaults["title"] = json!(title);
    let mut task: Task = serde_json::from_value(defaults).map_err(|e| format!("bad task: {e}"))?;
    task.order = next_order(ctx);

    // Written before the message is sent, and never the other way round: the record is what
    // the hub checks when it is about to act, so a message that arrived first would name a
    // task nothing can look up.
    task::save(&dir(ctx), &task)?;
    let handed = match task.status {
        Status::Queued => Some(hand_over(ctx, &task)?),
        _ => None,
    };
    Ok((task, handed))
}

/// Change a record, and hand it over if this is the change that queued it.
pub fn update(ctx: &Context, id: &str, input: &Value) -> Result<(Task, Option<Delivered>), String> {
    let mut task = task::load(&dir(ctx), id)?;
    let was = task.status;

    if let Some(status) = string(input, "status") {
        task.status = Status::parse(&status).ok_or(format!("no such status: {status}"))?;
    }
    if let Some(order) = input.get("order").and_then(Value::as_u64) {
        task.order = order as u32;
    }
    for (key, field) in [
        ("worktree", &mut task.worktree),
        ("issue", &mut task.issue),
        ("pr", &mut task.pr),
        ("note", &mut task.note),
    ] {
        if let Some(value) = input.get(key) {
            // An explicit `null` clears; an absent key leaves it alone. Without the
            // distinction there is no way to take back a worktree the hub wrote down.
            *field = value.as_str().map(str::to_string);
        }
    }
    task.updated_at = stamp();
    task::save(&dir(ctx), &task)?;

    // Handing over is a *transition*, not a status: re-sending on every save would put one
    // task in the inbox once for every time somebody dragged its card.
    let handed = if was != Status::Queued && task.status == Status::Queued {
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
        "status": if args.queue { "queued" } else { "backlog" },
    });
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
    pub json: bool,
}

pub fn update_cmd(args: &UpdateArgs<'_>) -> Result<(), String> {
    let ctx = super::context(args.repo, args.hub)?;
    let mut input = json!({});
    let fields = input.as_object_mut().expect("just built");
    for (key, value) in [
        ("status", args.status),
        ("worktree", args.worktree),
        ("issue", args.issue),
        ("pr", args.pr),
        ("note", args.note),
    ] {
        if let Some(value) = value {
            fields.insert(key.to_string(), json!(value));
        }
    }
    if let Some(order) = args.order {
        fields.insert("order".to_string(), json!(order));
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
    as_json: bool,
) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    let wanted = match status {
        Some(text) => Some(Status::parse(text).ok_or(format!("no such status: {text}"))?),
        None => None,
    };
    let tasks: Vec<Task> = task::list(&dir(&ctx))
        .into_iter()
        .filter(|t| wanted.is_none_or(|w| t.status == w))
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
    fn empty_title_and_body_produce_nothing() {
        let input = json!({ "title": "   ", "body": "   \n\n  " });
        assert!(derive_title(&input).is_none());
    }
}
