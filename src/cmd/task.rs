//! `adj task` — the records the board is a view of.
//!
//! The dashboard is not the only caller. The hub writes back here when it picks a task up
//! ("When a request arrives" Step 5 replies to the requester, and for a request from the dashboard
//! the requester is a file rather than a session), and a worker or a script can add one
//! without a browser. So the verbs live here and the HTTP layer calls them, rather than the
//! other way round.

use serde_json::{Value, json};

use crate::mail::{DeliveryOutcome, Reached};
use crate::registry::Context;
use crate::task::{self, Checked, PrState, Status, Task, create, fetch_issue, refresh, update};

/// Read the issue of a task that has just started and has none kept. A failure is reported
/// on stderr and nothing more: the command that got here did what it was asked, and the
/// issue can be read again from the board or `adj task fetch-issue`.
///
/// `changed` is whether this very command started the task or moved its issue. Without it,
/// an issue `gh` cannot read would make every later `adj task update --note …` wait on `gh`
/// and print the same failure again.
fn snapshot_if_started(ctx: &Context, task: Task, changed: bool) -> Task {
    if !changed || task::needs_snapshot(&task).is_none() {
        return task;
    }
    match fetch_issue(ctx, &task.id) {
        Ok(read) => read,
        Err(why) => {
            eprintln!("could not read the issue: {why}");
            task
        }
    }
}

/// `refresh`'s answer as the MCP tool and the board hand it on: sorted by what happened, so
/// a reader finds what changed without going through what did not.
pub fn refresh_json(checked: &[Checked]) -> Value {
    let entry = |c: &Checked| {
        let mut out = json!({
            "id": c.task.id,
            "title": c.task.title,
            "pr": c.task.pr,
            "status": c.task.status.as_str(),
            "state": c.state.as_str(),
            // Whose turn it is, read from the summary kept on the record; null until one was.
            "turn": c.task.pr_status.as_ref().and_then(task::pr_turn),
        });
        if let PrState::Unreadable(why) = &c.state {
            out["error"] = json!(why);
        }
        if let Some(why) = &c.failed {
            out["error"] = json!(why);
        }
        out
    };
    let with = |pick: fn(&Checked) -> bool| -> Vec<Value> {
        checked.iter().filter(|c| pick(c)).map(entry).collect()
    };
    json!({
        "done": with(|c| c.moved),
        "open": with(|c| c.state == PrState::Open),
        "closed": with(|c| c.state == PrState::Closed),
        "unreadable": with(|c| matches!(c.state, PrState::Unreadable(_))),
        // Merged, but the record changed while `gh` was being asked, so it was left as it is.
        "skipped": with(|c| c.state == PrState::Merged && !c.moved && c.failed.is_none()),
        // Merged, but the record could not be moved. `error` says why.
        "failed": with(|c| c.failed.is_some()),
    })
}

// ── the subcommands ──────────────────────────────────────────────────

pub struct AddArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub kind: &'a str,
    pub done_when: &'a str,
    pub stop_at: &'a str,
    pub executor: &'a str,
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
    let ctx = crate::registry::context(args.repo, args.hub)?;
    let body = super::read_body(args.body)?;
    let mut new = task::NewTask {
        title: args.title.map(str::to_string),
        body,
        kind: task::Kind::parse(args.kind)?,
        done_when: task::DoneWhen::parse(args.done_when)?,
        stop_at: if args.stop_at.is_empty() {
            task::StopAt::default()
        } else {
            task::StopAt::parse(args.stop_at)?
        },
        executor: task::Executor::parse(args.executor)
            .ok_or_else(|| format!("no such executor: {} (worker or jules)", args.executor))?,
        issue_url: args.issue_url.map(str::to_string),
        base: args.base.map(str::to_string),
        parent: args.parent.map(str::to_string),
        worktree_name: args.worktree_name.map(str::to_string),
        auto_start: !args.ask_first,
        status: if args.queue || args.waiting_in.is_some() {
            Status::Queued
        } else {
            Status::Backlog
        },
        ..task::NewTask::default()
    };
    // The hub writing down work it has prepared a worktree for: before it starts the worker,
    // so the brief can carry the id, or after `adj work` turned it away for want of a slot.
    // Queued either way, so a free slot can take it; but not handed over, because the inbox
    // it would land in is the caller's own, and a hub that messages itself is woken mid-turn
    // to be told what it just did.
    let mut hand_over = true;
    if let Some(worktree) = args.waiting_in {
        new.worktree = Some(
            crate::infra::paths::expand_home(worktree)
                .to_string_lossy()
                .to_string(),
        );
        hand_over = false;
    }
    let (task, handed) = create(&ctx, new, hand_over)?;
    let task = snapshot_if_started(&ctx, task, true);
    if args.json {
        println!(
            "{}",
            json!({ "task": task, "handed": handed.as_ref().map(crate::mail::Handed::from) })
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
    pub base: Option<&'a str>,
    pub jules_session: Option<&'a str>,
    pub executor: Option<&'a str>,
    pub note: Option<&'a str>,
    pub instruction: Option<&'a str>,
    pub auto_start: Option<bool>,
    pub no_hand_over: bool,
    pub json: bool,
}

pub fn update_cmd(args: &UpdateArgs<'_>) -> Result<(), String> {
    let ctx = crate::registry::context(args.repo, args.hub)?;
    // `--note -` reads it from stdin: a note is often text from elsewhere — an error, a
    // comment typed on the board — and does not belong inside quotes on a command line.
    let note = args.note.map(super::dash_is_stdin).transpose()?;
    let instruction = args.instruction.map(super::dash_is_stdin).transpose()?;
    // Trimmed, and blank is no change, as the board's JSON has always been read.
    fn word(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|s| !s.is_empty())
    }
    // An empty one clears: a command line has no way to say `null`.
    let text = |v: Option<&str>| v.map(|v| Some(v.to_string()).filter(|v| !v.is_empty()));
    let patch = task::TaskPatch {
        status: word(args.status)
            .map(|s| Status::parse(s).ok_or(format!("no such status: {s}")))
            .transpose()?,
        order: args.order,
        auto_start: args.auto_start,
        executor: word(args.executor)
            .map(|s| {
                task::Executor::parse(s).ok_or(format!("no such executor: {s} (worker or jules)"))
            })
            .transpose()?,
        worktree: text(args.worktree),
        issue: text(args.issue),
        pr: text(args.pr),
        base: text(args.base),
        jules_session: text(args.jules_session),
        jules_by: None,
        note: text(note.as_deref()),
        instruction: text(instruction.as_deref()),
    };
    // Read without the lock and `.ok()`: a failed or raced read can only cost one extra fetch
    // attempt, since `needs_snapshot` still guards it.
    let before = task::get(&ctx.state, &ctx.repo.slug, args.id).ok();
    let (task, handed) = update(&ctx, args.id, &patch, !args.no_hand_over)?;
    let changed = before.is_none_or(|b| {
        !matches!(b.status, Status::Dispatched | Status::Pr)
            || task::issue_to_fetch(&b) != task::issue_to_fetch(&task)
    });
    let task = snapshot_if_started(&ctx, task, changed);
    if args.json {
        println!(
            "{}",
            json!({ "task": task, "handed": handed.as_ref().map(crate::mail::Handed::from) })
        );
        return Ok(());
    }
    println!("{} — {} ({})", task.id, task.title, task.status.as_str());
    say_where_it_went(&ctx, &task, &handed);
    Ok(())
}

/// `adj task next`: the queued task a free worker slot should take, and the queued tasks that
/// ask first and have no `dispatch` gate open yet.
pub fn next_cmd(repo: Option<&str>, hub: Option<&str>, as_json: bool) -> Result<(), String> {
    let ctx = crate::registry::context(repo, hub)?;
    // Open gates only: an answered one has gone to the archive, and its answer is the hub's
    // to act on from the inbox.
    let gated: std::collections::HashSet<String> = crate::gate::list_of_kind(
        &ctx.state,
        &ctx.repo.slug,
        crate::gate::Shelf::Open,
        crate::gate::Kind::Dispatch,
    )
    .into_iter()
    .filter_map(|g| g.task)
    .collect();
    let next = task::next(task::list(&ctx.state, &ctx.repo.slug), &gated);
    if as_json {
        println!("{}", json!(next));
        return Ok(());
    }
    match &next.task {
        Some(t) => println!("next: {} — {}", t.id, t.title),
        None => println!("Nothing queued can be started."),
    }
    for t in &next.needs_dispatch_gate {
        println!("needs a dispatch gate: {} — {}", t.id, t.title);
    }
    Ok(())
}

pub fn list(
    repo: Option<&str>,
    hub: Option<&str>,
    status: Option<&str>,
    worktree: Option<&str>,
    as_json: bool,
) -> Result<(), String> {
    let ctx = crate::registry::context(repo, hub)?;
    let wanted = match status {
        Some(text) => Some(Status::parse(text).ok_or(format!("no such status: {text}"))?),
        None => None,
    };
    // Compared resolved: the hub names the worktree as `git worktree list` printed it, and
    // the record holds whatever path it was written with.
    let resolved = |path: &str| {
        let path = crate::infra::paths::expand_home(path);
        path.canonicalize().unwrap_or(path)
    };
    let at = worktree.map(resolved);
    let tasks: Vec<Task> = task::list(&ctx.state, &ctx.repo.slug)
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
    let ctx = crate::registry::context(repo, hub)?;
    let task = task::get(&ctx.state, &ctx.repo.slug, id)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&task).map_err(|e| e.to_string())?
    );
    Ok(())
}

/// `adj task fetch-issue`: read the issue again, whatever the record holds.
pub fn fetch_issue_cmd(
    repo: Option<&str>,
    hub: Option<&str>,
    id: &str,
    as_json: bool,
) -> Result<(), String> {
    let ctx = crate::registry::context(repo, hub)?;
    let task = fetch_issue(&ctx, id)?;
    if as_json {
        println!("{}", json!({ "task": task }));
        return Ok(());
    }
    let title = task
        .issue_snapshot
        .as_ref()
        .map_or("", |s| s.title.as_str());
    println!("{} — {}", task.id, title);
    Ok(())
}

pub fn refresh_cmd(repo: Option<&str>, hub: Option<&str>, as_json: bool) -> Result<(), String> {
    let ctx = crate::registry::context(repo, hub)?;
    let checked = refresh(&ctx)?;
    if as_json {
        println!("{}", refresh_json(&checked));
        return Ok(());
    }
    if checked.is_empty() {
        println!("No task is waiting on a pull request.");
        return Ok(());
    }
    for c in &checked {
        let what = match (&c.state, c.moved) {
            _ if c.failed.is_some() => format!(
                "merged, but the record could not be moved: {}",
                c.failed.as_deref().unwrap_or_default()
            ),
            (_, true) => "done".to_string(),
            (PrState::Merged, false) => {
                "merged, but the record changed meanwhile; left alone".to_string()
            }
            (PrState::Unreadable(why), _) => format!("unreadable: {why}"),
            (state, _) => format!("{}; left alone", state.as_str()),
        };
        println!(
            "{:<10} {}  {}  {}",
            c.task.status.as_str(),
            c.task.id,
            c.task.pr.as_deref().unwrap_or("-"),
            what
        );
    }
    Ok(())
}

/// What to say when nothing was put in the hub's inbox, which depends on where the task is
/// now: only a backlog task is "kept in the backlog". A queued one was either handed over
/// earlier or is waiting for a slot the hub itself asked for; anything further along is
/// already shown by its status.
fn not_handed_line(status: Status, hub_name: &str) -> Option<String> {
    match status {
        Status::Backlog => Some(format!(
            "Kept in the backlog. Nothing is in {hub_name}'s inbox yet."
        )),
        Status::Queued => Some(format!("Queued, but not handed to {hub_name} this time.")),
        Status::Dispatched | Status::Pr | Status::Done | Status::Cancelled => None,
    }
}

fn say_where_it_went(ctx: &Context, task: &Task, handed: &Option<DeliveryOutcome>) {
    let Some(handed) = handed else {
        if let Some(line) = not_handed_line(task.status, &ctx.repo.hub_name) {
            println!("{line}");
        }
        return;
    };
    println!("handed to {}: {}", ctx.repo.hub_name, handed.path.display());
    match handed.reached {
        Reached::Woken => println!("Woke the hub; it will pick this up."),
        Reached::Running { .. } => {
            println!("The hub is running; it will pick this up the next time it checks its inbox.")
        }
        Reached::NotRunning => println!(
            "The hub is not running. Waiting in its inbox for the next time it starts (task {}).",
            task.id
        ),
    }
}

/// What `adj task brief` is given.
pub struct BriefArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    /// The task the worker is for. `None` writes the brief of a session with no task.
    pub id: Option<&'a str>,
    pub worktree: &'a str,
    pub base: &'a str,
    pub key: Option<&'a str>,
    pub tracker: Option<&'a str>,
    pub parent: Option<&'a str>,
    pub instruction: Option<&'a str>,
    pub out: Option<&'a str>,
    pub json: bool,
}

/// A flag value that says something: a blank one is the same as not given.
fn given(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

/// `adj task brief`: write the worker's `.claude/task-brief.md` from the record and the settings.
///
/// The hub used to fill the brief in from a template by hand. Written here, the lines come
/// from the record the board shows, so the two cannot disagree, and the one thing the hub
/// has to get right is the record.
pub fn brief(args: &BriefArgs<'_>) -> Result<(), String> {
    let ctx = crate::registry::context(args.repo, args.hub)?;
    let of = match args.id {
        Some(id) => task::BriefOf::Task {
            id: id.to_string(),
            key: given(args.key).map(str::to_string),
            // Before the record is read, in the words the list it replaces used.
            tracker: given(args.tracker)
                .map(crate::kernel::brief::Tracker::parse)
                .transpose()?,
            parent: given(args.parent).map(str::to_string),
        },
        None => {
            let instruction = args
                .instruction
                .ok_or("a brief with no --id needs --instruction")?;
            task::BriefOf::Session {
                instruction: super::dash_is_stdin(instruction)?,
            }
        }
    };
    let written = task::write_brief(
        &ctx,
        &task::BriefRequest {
            worktree: args.worktree.to_string(),
            base: args.base.to_string(),
            out: args.out.map(str::to_string),
            of,
        },
    )?;
    if args.json {
        println!(
            "{}",
            json!({ "path": written.path, "task": args.id, "branch": written.branch })
        );
    } else {
        println!("{}", written.path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
