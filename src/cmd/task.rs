//! `adj task` — the records the board is a view of.
//!
//! The dashboard is not the only caller. The hub writes back here when it picks a task up
//! ("When a request arrives" Step 5 replies to the requester, and for a request from the dashboard
//! the requester is a file rather than a session), and a worker or a script can add one
//! without a browser. So the verbs live here and the HTTP layer calls them, rather than the
//! other way round.

use serde_json::{Value, json};

use crate::kernel::config;
use crate::mail::{DeliveryOutcome, Reached};
use crate::registry::Context;
use crate::task::{self, Status, Task};

// Old paths, kept until #363: `hub_title` calls `read_issue` through here.
pub(super) use crate::task::{PrState, read_issue};
// Old paths, kept until #363: the board, the MCP server, `session`, `pr_poll` and `hub_title`
// call the task operations through here.
pub use crate::task::{
    Checked, apply, candidates, check_worktree_name, create, fetch_issue, known_title, nudge,
    pr_refs, refresh, update, update_checked,
};
// `brief` checks and resolves what it is given as `create` does, until #359.
use crate::task::{check_parent, check_url, resolved_worktree};

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
            json!({ "task": task, "handed": handed_json(&handed) })
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

fn handed_json(handed: &Option<DeliveryOutcome>) -> Value {
    match handed {
        Some(d) => json!({
            "present": d.is_present(),
            "woken": d.was_woken(),
            "path": d.path.display().to_string(),
        }),
        None => Value::Null,
    }
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

/// The Done when the brief says. The worker branches on three phrases only, and a task that
/// goes as far as handling review has opened its PR, so `Review` is written as the PR.
fn brief_done_when(done_when: task::DoneWhen) -> &'static str {
    match done_when {
        task::DoneWhen::Review => task::DoneWhen::Pr.as_prose(),
        other => other.as_prose(),
    }
}

/// A flag value that says something: a blank one is the same as not given.
fn given(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

/// `adj task brief`: write the worker's `.claude/task-brief.md` from the record and the config.
///
/// The hub used to fill the brief in from a template by hand. Written here, the lines come
/// from the record the board shows, so the two cannot disagree, and the one thing the hub
/// has to get right is the record.
pub fn brief(args: &BriefArgs<'_>) -> Result<(), String> {
    use crate::kernel::brief as text;

    let ctx = crate::registry::context(args.repo, args.hub)?;
    let worktree = resolved_worktree(args.worktree);
    let worktree = std::path::Path::new(&worktree);
    if !worktree.is_dir() {
        return Err(format!("no such worktree: {}", worktree.display()));
    }
    // Read from the worktree rather than taken as an argument: it is the branch the worker
    // will be on, and a typed one is a second answer to a question git already has.
    let branch = crate::infra::git::git(&["symbolic-ref", "-q", "--short", "HEAD"], Some(worktree))
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            format!(
                "{} is not on a branch: the brief names the branch the worker works on",
                worktree.display()
            )
        })?;
    let config = ctx
        .resolved
        .config
        .clone()
        .unwrap_or_else(|| Value::Object(config::builtin_defaults()));
    let base = args.base.trim();
    if base.is_empty() {
        return Err("--base is empty: pass the commit-ish the worktree was cut from, or -".into());
    }

    let rendered = match args.id {
        Some(id) => {
            let (key_arg, tracker_arg, parent_arg) =
                (given(args.key), given(args.tracker), given(args.parent));
            if let Some(parent) = parent_arg {
                check_parent(parent)?;
            }
            // Written to a line the worker reads, and the hub puts it on a command line.
            if let Some(key) = key_arg.filter(|key| *key != "-" && !text::is_key(key)) {
                return Err(format!("not a tracker key: {key} (like ABC-123)"));
            }
            let record = task::get(&ctx.state, &ctx.repo.slug, id)?;
            if record.executor == task::Executor::Jules {
                return Err(format!(
                    "task {id} is handed to Jules: no worker is started, so no brief is written"
                ));
            }
            // An empty URL is none: `--issue-url ''` stores one.
            let nonblank = |url: &Option<String>| url.clone().filter(|url| !url.trim().is_empty());
            let url = nonblank(&record.issue_url).or_else(|| nonblank(&record.issue));
            let (key, tracker) = match (&url, key_arg, tracker_arg) {
                (_, Some(key), Some(tracker)) => (key.to_string(), tracker.to_string()),
                (None, key, tracker) => (
                    key.unwrap_or("-").to_string(),
                    tracker.unwrap_or("-").to_string(),
                ),
                (Some(url), key, tracker) => {
                    let sources = config
                        .get("taskSources")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    let issue_keys = config
                        .get("issueKeys")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    let (read_tracker, read_key) = text::tracker_and_key(url, sources, &issue_keys)
                        .ok_or_else(|| {
                            format!(
                                "cannot tell the tracker and key of {url}: pass --key and --tracker"
                            )
                        })?;
                    (
                        key.map_or(read_key, str::to_string),
                        tracker.map_or(read_tracker, str::to_string),
                    )
                }
            };
            if !["github", "github-project", "jira", "linear", "-"].contains(&tracker.as_str()) {
                return Err(format!(
                    "no such tracker: {tracker} (github, github-project, jira, linear or -)"
                ));
            }
            let verify = config
                .get("verify")
                .and_then(Value::as_array)
                .map(|commands| {
                    commands
                        .iter()
                        .map(|c| c.as_str().map_or_else(|| c.to_string(), str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let parent = parent_arg
                .map(str::to_string)
                .or(record
                    .parent
                    .clone()
                    .filter(|parent| !parent.trim().is_empty()))
                .unwrap_or_else(|| "-".to_string());
            // The worker fetches the parent from its URL; a bare key says nothing of the tracker.
            // Checked again here because a record written before parents were checked on the way
            // in may hold anything.
            if parent != "-" {
                if crate::kernel::brief::is_key(&parent) {
                    return Err(format!(
                        "the parent task is a key ({parent}): find its URL and pass --parent '<URL>'"
                    ));
                }
                check_url(&parent, "a task")?;
            }
            text::render_task(&text::TaskBrief {
                key,
                title: record.title.clone(),
                tracker,
                url,
                request: record.body.clone(),
                branch: branch.clone(),
                base: base.to_string(),
                parent,
                record: record.id.clone(),
                done_when: brief_done_when(record.done_when).to_string(),
                stop_at: record.stop_at.as_str().to_string(),
                handover: record
                    .instruction
                    .clone()
                    .filter(|note| !note.trim().is_empty())
                    .unwrap_or_else(|| "-".to_string()),
                copilot_review: config
                    .get("copilotReview")
                    .and_then(Value::as_str)
                    .unwrap_or("ask")
                    .to_string(),
                verify,
            })
        }
        None => {
            let instruction = args
                .instruction
                .ok_or("a brief with no --id needs --instruction")?;
            let instruction = super::dash_is_stdin(instruction)?;
            let instruction = match instruction.trim() {
                "" | "-" => text::NO_INSTRUCTION.to_string(),
                _ => instruction,
            };
            text::render_session(&text::SessionBrief {
                branch: branch.clone(),
                base: base.to_string(),
                instruction,
            })
        }
    };

    let path = match args.out {
        Some(out) => crate::infra::paths::expand_home(out),
        None => worktree.join(".claude").join("task-brief.md"),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    // Always written over: the brief is derived, and one left from an earlier start is the
    // stale answer this exists to replace.
    std::fs::write(&path, rendered).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    if args.json {
        println!(
            "{}",
            json!({ "path": path, "task": args.id, "branch": branch })
        );
    } else {
        println!("{}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
