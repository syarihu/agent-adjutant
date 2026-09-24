//! The subcommands. This is the only layer allowed to know about more than one of the
//! others: everything below is a leaf that answers one question, and joining them up is
//! what a command is.

use serde_json::{Value, json};
use std::io::Read;
use std::time::Duration;

use crate::config::{self, Settings};
use crate::ide;
use crate::messaging::{self, Message};
use crate::notify;
use crate::repo::{self, RepoInfo};
use crate::runner;
use crate::terminal::{self, SpawnRequest};

mod gate;
mod serve;
mod task;

pub use gate::{
    AnswerArgs, CloseArgs, answer_cmd as gate_answer, close_cmd as gate_close, list as gate_list,
    open_cmd as gate_open, show as gate_show,
};
pub use serve::{DEFAULT_PORT, serve};
pub use task::{
    AddArgs, UpdateArgs, add as task_add, list as task_list, show as task_show,
    update_cmd as task_update,
};

/// Everything a command needs to know about where it is. Resolved once, at the top, because
/// two commands disagreeing about which repo they are in is the failure that loses reports.
pub struct Context {
    pub repo: RepoInfo,
    pub settings: Settings,
    pub resolved: config::Resolved,
}

/// Where we are, for a command addressing a hub: sending to it, listing its inbox, naming
/// it, bringing it forward.
pub fn context(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<Context, String> {
    context_of(resolve(repo_arg, hub_arg)?)
}

/// The same, for a command that *starts or registers* a hub rather than addressing one.
///
/// The difference is the worker record. A hub launched from inside a worktree, and a worker
/// registering in the worktree its tab was opened at, would both read a record that belongs
/// to somebody else — or, for the worker, the one it is a moment away from overwriting. See
/// `messaging::hub_id_told`.
fn context_as(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<Context, String> {
    context_of(repo::resolve(
        repo_arg,
        messaging::hub_id_told(hub_arg).as_deref(),
    )?)
}

/// The same, for a command that needs the settings and the checkout and no hub at all.
///
/// `ide` and `worktree-path` never read a hub field, and neither takes `--hub`. Sending them
/// through the addressing path would make an unreadable worker record stop them — and an
/// unreadable record is exactly the state of the worktree somebody is trying to open an
/// editor on. Strictness belongs where a wrong answer misroutes something.
fn context_without_hub(repo_arg: Option<&str>) -> Result<Context, String> {
    context_of(repo::resolve(repo_arg, None)?)
}

fn context_of(repo: RepoInfo) -> Result<Context, String> {
    // By `owner/name` and nothing else. The hub identifier moves the address; it must not
    // move the lookup, or asking for a second hub of a registered repository would answer
    // with an unregistered one — no task sources, no issue keys, no verify command.
    let resolved = config::resolve_config(&repo.nwo)?;
    Ok(Context {
        settings: resolved.settings.clone(),
        repo,
        resolved,
    })
}

/// Where we are, and which hub of it we are talking to.
///
/// Every subcommand that addresses a hub goes through here rather than calling
/// `repo::resolve` with whatever it was given: deciding between the flag, the environment
/// and the worktree is one rule, and a second copy of it is a second answer.
fn resolve(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<RepoInfo, String> {
    repo::resolve(repo_arg, messaging::hub_id(hub_arg, None)?.as_deref())
}

// ── hub-name ─────────────────────────────────────────────────────────

pub fn hub_name(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    as_json: bool,
) -> Result<(), String> {
    let info = resolve(repo_arg, hub_arg)?;
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "main": info.main,
                "nwo": info.nwo,
                "repo": info.repo,
                // Which hub of the repository this address belongs to, as it was resolved
                // — `null` for the repository's own. Printed because it is the only way to
                // see, from outside, which of the three answers won.
                "hub": info.hub,
                "slug": info.slug,
                "hubName": info.hub_name,
                "nwoSource": info.nwo_source,
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }
    if info.nwo_source == "dirname" {
        eprintln!(
            "adjutant: origin gave no repository name, using the directory name {}",
            info.nwo
        );
    }
    println!("{}", info.hub_name);
    Ok(())
}

// ── config ───────────────────────────────────────────────────────────

pub fn show_config(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<(), String> {
    let ctx = context(repo_arg, hub_arg)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "repo": ctx.repo.nwo,
            "main": ctx.repo.main,
            "hub": ctx.repo.hub,
            "hubName": ctx.repo.hub_name,
            "registered": ctx.resolved.registered,
            "configPath": ctx.resolved.config_path,
            "warnings": ctx.resolved.warnings,
            "settings": ctx.settings,
            "config": ctx.resolved.config,
        }))
        .unwrap_or_default()
    );
    Ok(())
}

// ── pending ──────────────────────────────────────────────────────────

pub struct PendingArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub path_only: bool,
    pub limit: usize,
    pub as_json: bool,
    pub read: Option<&'a str>,
    pub ack: Option<&'a str>,
}

pub fn pending(args: &PendingArgs<'_>) -> Result<(), String> {
    let info = resolve(args.repo, args.hub)?;
    let dir = messaging::inbox_dir(&info.slug);
    if args.path_only {
        // A caller asking for the path is about to write into it, so hand back a directory
        // that exists.
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        println!("{}", dir.display());
        return Ok(());
    }
    if let Some(name) = args.read {
        print!("{}", messaging::read(&info.slug, name)?);
        return Ok(());
    }
    if let Some(name) = args.ack {
        let moved = messaging::ack(&info.slug, name)?;
        println!("filed {} ({})", name, moved.display());
        return Ok(());
    }

    let entries = messaging::list(&info.slug);
    if args.as_json {
        let items: Vec<Value> = entries
            .iter()
            .map(|e| {
                json!({"name": e.name, "from": e.from, "worktree": e.worktree,
                       "kind": e.kind, "subject": e.subject})
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "hubName": info.hub_name,
                "dir": dir.to_string_lossy(),
                "count": items.len(),
                "messages": items,
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }
    // The path is printed even when nothing is waiting: an empty listing is the common case,
    // and it is also the one where the reader would otherwise have to guess the location.
    println!("dir: {}", dir.display());
    if entries.is_empty() {
        println!("(empty)");
        return Ok(());
    }
    for entry in entries.iter().take(args.limit) {
        println!(
            "{}  [{}] {} — {}",
            entry.name, entry.kind, entry.from, entry.subject
        );
    }
    if entries.len() > args.limit {
        println!("... and {} more", entries.len() - args.limit);
    }
    Ok(())
}

// ── send ─────────────────────────────────────────────────────────────

pub struct SendArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub from: Option<&'a str>,
    pub kind: &'a str,
    pub subject: Option<&'a str>,
    pub body: Option<&'a str>,
    pub quiet: bool,
}

/// What became of a message handed to the hub.
pub struct Delivered {
    pub delivery: messaging::Delivery,
    pub woken: bool,
}

/// Leave a message for the hub, poke its tab, and tell the person.
///
/// Shared by `adj send` and by the dashboard's hand-over, which is the whole reason it is a
/// function: the three steps are one rule, and a second copy of it is a second set of
/// conditions about when to wake and when to notify — drifting from the day it is written.
pub fn deliver_to_hub(ctx: &Context, message: &Message) -> Result<Delivered, String> {
    deliver_to_hub_announcing(ctx, message, true)
}

/// `deliver_to_hub`, choosing whether to tell the person. Not when they are the sender: a
/// decision made on the board a moment ago does not need a banner to say it was made.
pub fn deliver_to_hub_announcing(
    ctx: &Context,
    message: &Message,
    announce: bool,
) -> Result<Delivered, String> {
    let subject =
        messaging::header_value(&messaging::render_message(message), "subject").unwrap_or_default();
    let delivery = messaging::send(&ctx.repo.slug, &ctx.repo.hub_name, message)?;

    // A file appearing in a directory wakes nobody, so delivery has two follow-ups: poke the
    // hub if it is actually sitting there, and tell the person either way.
    let woken = match (
        delivery.present,
        messaging::hub_status(&ctx.repo.slug, &ctx.repo.hub_name).pid,
    ) {
        (true, Some(pid)) => terminal::wake(
            &ctx.settings.hub_wake,
            pid,
            &subject,
            terminal::HUB_WAKE_LINE,
            false,
        )
        .map(|done| done.ran)
        .unwrap_or(false),
        _ => false,
    };
    // Unconditionally, unlike `tell`, and the difference is the direction rather than an
    // oversight. This is a worker reporting to the hub, and the hub is the unattended half
    // — nobody is watching that tab, which is the premise the whole design rests on. A
    // report is also the thing a person most wants to hear about, so it is announced
    // whether or not the hub was poked. `tell` runs the other way, hub to worker: a worker
    // that was successfully woken needs no human, so there the notification is what happens
    // when waking did not.
    if announce
        && let Some(command) = notify::repo_command(&ctx.settings.notification, &ctx.repo, &subject)
    {
        let _ = terminal::run_shell(&command);
    }
    Ok(Delivered { delivery, woken })
}

pub fn send(args: &SendArgs<'_>) -> Result<(), String> {
    let ctx = context(args.repo, args.hub)?;
    let body = read_body(args.body)?;
    let message = Message {
        from: args.from.unwrap_or("unknown").to_string(),
        // Where this is being sent from, taken from the same directory the repository was
        // resolved in rather than from anything the sender says about itself.
        worktree: repo::current_worktree(None),
        kind: args.kind.to_string(),
        subject: args.subject.unwrap_or("").to_string(),
        body,
    };
    let Delivered { delivery, woken } = deliver_to_hub(&ctx, &message)?;

    if args.quiet {
        return Ok(());
    }
    println!(
        "delivered to {}: {}",
        ctx.repo.hub_name,
        delivery.path.display()
    );
    match (delivery.present, woken) {
        (true, true) => println!("Woke the hub; it will pick this up."),
        (true, false) => {
            println!("The hub is running; it will pick this up the next time it checks its inbox.")
        }
        (false, _) => {
            println!(
                "The hub is not running. Left in its inbox; it will be picked up the next time it starts."
            )
        }
    }
    Ok(())
}

// ── spawn / focus / close / work / ide ───────────────────────────────

/// The command that names a new tab from inside it, if one should.
///
/// A tab names itself rather than being named by the terminal's API, so that whatever
/// `terminal.title` is set to governs every tab the same way. `terminal::spawn` decides
/// whether it can be used at all: only a terminal taking a shell line can run it.
fn title_command(settings: &Settings, title: &str) -> Option<String> {
    if settings.terminal.title.is_off() || title.trim().is_empty() {
        return None;
    }
    // stdin closed: `adjutant title` reads a title of `-` from stdin, and in a new tab stdin is
    // the terminal — a task titled `-` would sit there waiting for input, and the worker after
    // it would never start.
    Some(format!(
        "{} < /dev/null",
        crate::template::sh_join(&[
            exe_path(),
            "title".to_string(),
            "--title".to_string(),
            title.to_string(),
        ])
    ))
}

pub fn spawn(
    repo_arg: Option<&str>,
    cwd: &str,
    title: &str,
    command: &[String],
    dry_run: bool,
) -> Result<(), String> {
    if command.is_empty() {
        return Err("pass the command to run after --".to_string());
    }
    let settings = settings_for(repo_arg);
    let name_it = title_command(&settings, title);
    let done = terminal::spawn(
        settings.terminal.spawn.as_deref(),
        &SpawnRequest {
            cwd: &config::expand_home(cwd).to_string_lossy(),
            title,
            command: &crate::template::sh_join(command),
            title_command: name_it.as_deref(),
        },
        dry_run,
    )?;
    if dry_run {
        println!("{}", done.script);
    } else {
        println!("{}", done.description);
    }
    Ok(())
}

/// Open a tab and start a worker agent in it. One command rather than two so the runner
/// template is read in exactly one place.
/// The environment a new tab has to be handed on its command line, because a terminal is
/// given a command line and nothing else.
///
/// `ADJUTANT_HUB` already travels as a flag, and `ADJUTANT_STARTUP_DASHBOARD` as one too.
/// These two have none, and losing them does not fail — it *splits*: the tab reads the
/// default config and the default state directory, so the agent it starts registers in one
/// world while the hub that dispatched it waits in another. The worker reports into an
/// inbox nobody is reading, and both halves look healthy from where they stand.
///
/// Forwarded only when this process was given them. A machine that never sets them gets the
/// command line it always had.
fn forwarded_env() -> Vec<String> {
    let set: Vec<String> = [config::CONFIG_ENV, messaging::STATE_DIR_ENV]
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| format!("{name}={value}"))
        })
        .collect();
    if set.is_empty() {
        return Vec::new();
    }
    let mut parts = vec!["env".to_string()];
    parts.extend(set);
    parts
}

/// What `adj work` exits with when `maxWorkers` is reached. Its own code rather than the
/// usual 1, because the caller is usually a hub, and "full, try later" is the one refusal it
/// should answer by leaving the task queued instead of reporting a failure.
pub const WORKER_LIMIT_EXIT: i32 = 3;

/// Refuse when `maxWorkers` workers are already running in this checkout, and otherwise mark
/// `worktree` as taken so the next dispatch in the same turn counts it.
///
/// Counted per checkout rather than per hub: two hubs on one repository share the machine
/// the limit is protecting. `Ok(Some(message))` is the refusal, kept apart from `Err` so the
/// caller can give it its own exit code.
fn claim_worker_slot(
    ctx: &Context,
    worktree: &std::path::Path,
    dry_run: bool,
) -> Result<Option<String>, String> {
    let Some(max) = ctx.settings.max_workers else {
        if !dry_run {
            messaging::mark_worker_starting(worktree)?;
        }
        return Ok(None);
    };
    let main = std::path::Path::new(&ctx.repo.main);
    messaging::with_dispatch_lock(main, || {
        // The main checkout too. It is where the hub sits and a worker is not meant to go,
        // but nothing stops `adj work` being pointed at it, and a worker running there
        // uncounted is one past the limit.
        let mut candidates = repo::linked_worktrees(&ctx.repo.main)?;
        candidates.push(ctx.repo.main.clone());
        let busy = messaging::busy_worktrees(&candidates, Some(worktree));
        if busy.len() >= max as usize {
            return Ok(Some(format!(
                "worker limit reached: {} of maxWorkers {max} are running ({}). \
                 Nothing was started; leave the task queued and dispatch it when one finishes",
                busy.len(),
                busy.join(", ")
            )));
        }
        if !dry_run {
            messaging::mark_worker_starting(worktree)?;
        }
        Ok(None)
    })?
}

/// Open the tab, and give the slot back if it never opened.
fn spawn_worker(
    ctx: &Context,
    worktree: &str,
    request: &SpawnRequest<'_>,
    dry_run: bool,
) -> Result<i32, String> {
    let done = terminal::spawn(ctx.settings.terminal.spawn.as_deref(), request, dry_run)
        .inspect_err(|_| {
            let _ = messaging::unmark_worker_starting(std::path::Path::new(worktree));
        })?;
    if dry_run {
        println!("{}", done.script);
    } else {
        println!("{}", done.description);
    }
    Ok(0)
}

pub struct WorkArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub worktree: &'a str,
    pub title: &'a str,
    /// The task record to take the title from, when `title` is empty.
    pub task: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub resume: bool,
    pub dry_run: bool,
}

pub fn work(args: &WorkArgs<'_>) -> Result<i32, String> {
    let WorkArgs {
        repo: repo_arg,
        hub: hub_arg,
        worktree,
        title,
        task: task_id,
        prompt,
        resume,
        dry_run,
    } = *args;
    if resume {
        return work_resumed(repo_arg, hub_arg, worktree, title, prompt, dry_run);
    }
    let prompt = prompt.unwrap_or(runner::WORKER_STARTUP_PROMPT);
    // The dispatching side: the identifier being handed to the new worker is this caller's
    // own, never one read out of some worktree it happens to be standing in.
    let ctx = context_as(repo_arg, hub_arg)?;
    // Named after the task's record rather than a title typed on the command line. The title
    // comes from an issue or a report, and quoted into the hub's shell it could close the
    // quote; the record's id is one this tool generated.
    let from_record;
    let title = match task_id {
        Some(id) if title.is_empty() => {
            from_record = crate::task::load(&task::dir(&ctx), id)?.title;
            from_record.as_str()
        }
        _ => title,
    };
    let worktree = config::expand_home(worktree).to_string_lossy().to_string();
    // Asked here and not left to the spawn, because marking the slot writes into the
    // worktree and would create the very directory the spawn checks for — a mistyped path
    // would then open a tab in an empty directory outside any repository.
    if !std::path::Path::new(&worktree).is_dir() {
        return Err(format!("no such directory: {worktree}"));
    }
    if let Some(refusal) = claim_worker_slot(&ctx, std::path::Path::new(&worktree), dry_run)? {
        eprintln!("adjutant: {refusal}");
        return Ok(WORKER_LIMIT_EXIT);
    }
    // The tab runs `adjutant worker`, not the agent directly. The agent is started by a
    // process that has already written down its own PID and then `exec`s itself away, which
    // is the only way anyone later gets to ask "is that worker still there".
    let mut parts = forwarded_env();
    parts.extend([
        exe_path(),
        "worker".to_string(),
        "--worktree".to_string(),
        worktree.clone(),
        "--title".to_string(),
        title.to_string(),
        "--prompt".to_string(),
        prompt.to_string(),
    ]);
    if let Some(repo) = repo_arg {
        parts.push("--repo".to_string());
        parts.push(repo.to_string());
    }
    // The *resolved* identifier rather than the flag, because a hub dispatching work runs
    // this as its own child and so usually passes no flag at all — it is carrying the
    // answer in its environment. That environment does not survive the trip: the tab is
    // opened by the terminal, which is handed a command line and nothing else. So the
    // answer goes onto the command line, or the worker registers under the wrong hub and
    // reports to an inbox nobody reads.
    // One argument rather than two: an identifier that starts with a dash reaches here from
    // `ADJUTANT_HUB`, where no flag parser has seen it, and as a separate word clap reads it
    // as the next option instead of as this one's value.
    if let Some(hub) = &ctx.repo.hub {
        parts.push(format!("--hub={hub}"));
    }
    let name_it = title_command(&ctx.settings, title);
    spawn_worker(
        &ctx,
        &worktree,
        &SpawnRequest {
            cwd: &worktree,
            title,
            command: &crate::template::sh_join(&parts),
            title_command: name_it.as_deref(),
        },
        dry_run,
    )
}

/// Open a tab that reopens the worker session saved in `worktree`.
///
/// Unlike a fresh dispatch, the hub is *not* this caller's: the worker goes back under
/// whichever hub dispatched it, which the saved session remembers and `adj worker --resume`
/// reads for itself. So nothing is forwarded unless it was said outright — forwarding the
/// caller's own identifier would re-file the worker under whoever happened to reopen it.
fn work_resumed(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    worktree: &str,
    title: &str,
    prompt: Option<&str>,
    dry_run: bool,
) -> Result<i32, String> {
    let ctx = context_without_hub(repo_arg)?;
    let worktree = worker_worktree(Some(worktree))?;
    // Refused here rather than in the tab, so the caller — often a hub — hears about it.
    let saved = saved_worker_session(&worktree)?;
    resume_template(
        ctx.settings.agent_resume_runner.as_deref(),
        "agentResumeRunner",
    )?;
    // A reopened worker is as much a process as a fresh one.
    if let Some(refusal) = claim_worker_slot(&ctx, &worktree, dry_run)? {
        eprintln!("adjutant: {refusal}");
        return Ok(WORKER_LIMIT_EXIT);
    }
    let worktree = worktree.to_string_lossy().to_string();
    let title = match title {
        "" => saved.title.as_deref().unwrap_or(""),
        given => given,
    };
    let mut parts = forwarded_env();
    parts.extend([
        exe_path(),
        "worker".to_string(),
        "--resume".to_string(),
        "--worktree".to_string(),
        worktree.clone(),
    ]);
    if !title.is_empty() {
        parts.push("--title".to_string());
        parts.push(title.to_string());
    }
    if let Some(prompt) = prompt {
        parts.push("--prompt".to_string());
        parts.push(prompt.to_string());
    }
    if let Some(repo) = repo_arg {
        parts.push("--repo".to_string());
        parts.push(repo.to_string());
    }
    if let Some(hub) = hub_arg.map(str::trim).filter(|hub| !hub.is_empty()) {
        parts.push(format!("--hub={hub}"));
    }
    let name_it = title_command(&ctx.settings, title);
    spawn_worker(
        &ctx,
        &worktree,
        &SpawnRequest {
            cwd: &worktree,
            title,
            command: &crate::template::sh_join(&parts),
            title_command: name_it.as_deref(),
        },
        dry_run,
    )
}

pub fn focus(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    quiet: bool,
    dry_run: bool,
) -> Result<bool, String> {
    let ctx = context(repo_arg, hub_arg)?;
    let status = messaging::hub_status(&ctx.repo.slug, &ctx.repo.hub_name);
    let Some(pid) = status.pid.filter(|_| status.present) else {
        if !quiet {
            println!("{} is not running", ctx.repo.hub_name);
        }
        return Ok(false);
    };
    let done = terminal::focus(
        ctx.settings.terminal.focus.as_deref(),
        pid,
        &ctx.repo.hub_name,
        dry_run,
    )?;
    if dry_run {
        println!("{}", done.script);
    } else if !quiet {
        println!("{} is already running (pid {pid})", ctx.repo.hub_name);
        if !done.ran {
            println!("({})", done.description);
        }
    }
    Ok(true)
}

/// How long to wait for a closed tab's worker to actually be gone, and how often to look.
///
/// Closing a tab hangs its session up and the process in it then unwinds, which is quick but
/// not instant — a single look straight afterwards would call a live worker gone. Two
/// seconds is far longer than an agent takes to die and far shorter than anyone would wait
/// for a cleanup step, and a worker still there after it is a worker that is not going.
const GONE_BUDGET: Duration = Duration::from_secs(2);
const GONE_POLL: Duration = Duration::from_millis(100);

/// Wait for a worker to be gone, and answer with what was actually seen.
///
/// Polled rather than slept through: a process that has already exited by the first look is
/// the common case, and a cleanup step that always cost the whole budget is a step people
/// stop running. `look` and `wait` are handed in for the reason `terminal::close_with` takes
/// its runner — this decision has to be testable without spending the budget in real time.
fn settled(
    mut look: impl FnMut() -> messaging::Liveness,
    mut wait: impl FnMut(Duration),
    budget: Duration,
    poll: Duration,
) -> messaging::Liveness {
    // One look before any waiting, then one more per interval until the budget is spent.
    // Counted rather than accumulated, so that a zero interval cannot spin here forever.
    let looks = 1 + budget.as_millis() / poll.as_millis().max(1);
    let mut answer = look();
    for _ in 1..looks {
        if answer == messaging::Liveness::Gone {
            return answer;
        }
        wait(poll);
        answer = look();
    }
    answer
}

/// Why a record was left where it was, said the way a person reads it.
///
/// `None` when it was cleared. The two reasons get a sentence each: announcing "somebody
/// else is working here" for a file that cannot be read sends a person looking for a worker
/// who was never there.
fn left_alone(cleared: &messaging::Cleared, worktree: &std::path::Path) -> Option<String> {
    match cleared {
        messaging::Cleared::Yes => None,
        messaging::Cleared::AnotherWorker => Some(format!(
            "another worker has registered in {} since",
            worktree.display()
        )),
        messaging::Cleared::Unreadable => Some(format!(
            "the record in {} can no longer be read",
            worktree.display()
        )),
    }
}

/// Close the tab the worker in a worktree is sitting in.
///
/// The hub's way of ending a session it started: it is the side that knows the task is
/// over, and a finished worker's tab otherwise stays open with nobody to close it.
///
/// Everything here is about **one** worker, read out of the record once and carried
/// through: the gate, the tab that gets closed, the process that has to be gone afterwards
/// and the record that may then be cleared. Asked separately, each of those questions can
/// be answered about a different worker — the next one registering in the same worktree —
/// and the answers then compose into a worktree that is deleted while somebody is using it.
///
/// `false` means a worker may still be sitting there. The caller is on its way to removing
/// this worktree, so that answer has to reach a shell as an exit code rather than as a
/// sentence in the output — and everything this cannot establish answers `false`, because
/// the cost of the two mistakes is not symmetric: a cleanup that stops is finished by hand,
/// a cleanup that carries on deletes work nobody can get back.
pub fn close(
    repo_arg: Option<&str>,
    worktree: &str,
    quiet: bool,
    dry_run: bool,
) -> Result<bool, String> {
    // Settings rather than a whole `Context`, like `spawn` and `title`: which config to
    // read is the only thing the repository is asked for here, and this is the command most
    // likely to be run while the repository it belongs to is being taken apart.
    let settings = settings_for(repo_arg);
    let worktree = config::expand_home(worktree);
    // No `is_dir` check, deliberately unlike `tell`: this runs during cleanup, so a
    // worktree that has already been removed is the ordinary way to arrive here twice
    // rather than a mistake worth failing over.
    let worker = match messaging::read_worker(&worktree) {
        // Nothing registered here is the job already done. A hub that calls this twice, or
        // calls it on a worker that stopped on its own, has to get on with the cleanup.
        messaging::WorkerRecord::Absent => {
            if !quiet {
                println!("no worker is running in {}", worktree.display());
            }
            return Ok(true);
        }
        messaging::WorkerRecord::Unreadable => {
            if !quiet {
                println!(
                    "the worker record in {} cannot be read as naming a worker, so nothing was cleared",
                    worktree.display()
                );
            }
            return Ok(false);
        }
        messaging::WorkerRecord::Named(worker) => worker,
    };
    let pid = worker.pid;
    match messaging::worker_liveness(&worker) {
        messaging::Liveness::Gone => {
            // The worker this record named is gone, so the record is the only thing left to
            // clear — and only while it is still that worker's.
            let cleared = match dry_run {
                true => messaging::Cleared::Yes,
                false => messaging::unregister_worker_if(&worktree, &worker)?,
            };
            if let Some(why) = left_alone(&cleared, &worktree) {
                if !quiet {
                    println!("the worker recorded here is gone, but {why}; nothing was cleared");
                }
                return Ok(false);
            }
            if !quiet {
                println!("no worker is running in {}", worktree.display());
                match dry_run {
                    true => println!("(a record is left behind; it would be cleared)"),
                    false => println!("(cleared the record it left behind)"),
                }
            }
            return Ok(true);
        }
        messaging::Liveness::CannotTell => {
            if !quiet {
                println!("cannot tell whether pid {pid} is still running, so nothing was cleared");
            }
            return Ok(false);
        }
        messaging::Liveness::Alive => {}
    }
    let done = terminal::close(
        &settings.terminal.close,
        pid,
        // The name the tab actually carries: `spawn` put the record's title through
        // `sanitise_title` with the directory name behind it, and a template that matches
        // a tab by name has to be handed the answer that got there.
        &terminal::sanitise_title(
            worker.title.as_deref().unwrap_or_default(),
            worktree
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default(),
        ),
        dry_run,
    )?;
    if dry_run {
        // The description when there is no script to show. The built-in path produces none
        // for a pid whose terminal cannot be found, and a blank line tells the reader less
        // than the sentence explaining why.
        println!(
            "{}",
            match done.script.is_empty() {
                true => &done.description,
                false => &done.script,
            }
        );
        // The exit code still answers about the worktree rather than about the plan — a
        // live worker was found a few lines above, and nothing has been closed. `focus`
        // sets the precedent: its dry run reports the state it looked at. Answering "safe
        // to remove" here is how `close --dry-run && git worktree remove` deletes a live
        // worker's checkout.
        return Ok(false);
    }
    if !done.ran {
        if !quiet {
            println!("{}", done.description);
        }
        return Ok(false);
    }
    // What the close command reported is not the question — `terminal::close` says what
    // little `ran` can mean. Neither is what the record says afterwards: a close command
    // that removed the record instead of the tab would leave a worktree that *looks* free.
    // Only the worker's own absence settles it.
    match settled(
        || messaging::worker_liveness(&worker),
        std::thread::sleep,
        GONE_BUDGET,
        GONE_POLL,
    ) {
        messaging::Liveness::Gone => {
            // The process went with its tab, so a record left behind would have `present`
            // lying to whoever asks next — including the next call to this. Conditional,
            // because the worktree may have been handed to a new worker while this one was
            // being closed, and that worker's record is not this call's to remove.
            let cleared = messaging::unregister_worker_if(&worktree, &worker)?;
            if let Some(why) = left_alone(&cleared, &worktree) {
                if !quiet {
                    println!("pid {pid} is gone, but {why}; nothing was cleared");
                }
                return Ok(false);
            }
            if !quiet {
                println!("{}", done.description);
            }
            Ok(true)
        }
        messaging::Liveness::Alive => {
            if !quiet {
                println!("the close command ran but pid {pid} is still there; nothing was cleared");
                println!(
                    "(a terminal that asks before closing a session with a process in it is waiting for an answer)"
                );
            }
            Ok(false)
        }
        messaging::Liveness::CannotTell => {
            if !quiet {
                println!(
                    "the close command ran but whether pid {pid} is gone cannot be established; nothing was cleared"
                );
            }
            Ok(false)
        }
    }
}

pub fn open_ide(repo_arg: Option<&str>, worktree: &str, dry_run: bool) -> Result<(), String> {
    let ctx = context_without_hub(repo_arg)?;
    let worktree = config::expand_home(worktree).to_string_lossy().to_string();
    let Some(command) = ide::open_command(ctx.settings.ide.as_deref(), &worktree) else {
        return Err("ide is not set: put your editor command in the config's ide key".to_string());
    };
    if dry_run {
        println!("{command}");
        return Ok(());
    }
    terminal::run_shell(&command)?;
    println!("opened {worktree}");
    Ok(())
}

/// Name the tab this process is sitting in. The hub calls it on itself at startup; nothing
/// else needs it, because a spawned tab is named at spawn time.
pub fn set_title(repo_arg: Option<&str>, title: &str, dry_run: bool) -> Result<(), String> {
    let settings = settings_for(repo_arg);
    let title = dash_is_stdin(title)?;
    let done = terminal::set_title(&settings.terminal.title, &title, dry_run)?;
    if dry_run {
        println!("{}", done.script);
    } else {
        println!("{}", done.description);
    }
    Ok(())
}

pub fn notify_user(
    repo_arg: Option<&str>,
    title: &str,
    message: &str,
    dry_run: bool,
) -> Result<(), String> {
    // Resolved once rather than left to `settings_for`, because a `{nwo}` template needs the
    // same answer the config was picked with — and because this command is the one that can
    // legitimately be run from outside a repository, where there is no answer at all.
    let nwo = match repo_arg {
        Some(arg) => Some(arg.to_string()),
        None => repo::resolve(None, None).ok().map(|info| info.nwo),
    };
    let settings = settings_for(nwo.as_deref());
    if nwo.is_none() && notify::needs_repo(&settings.notification) {
        eprintln!(
            "adjutant: the notification template asks for {{nwo}} but this is not a repository — pass --repo owner/name"
        );
    }
    let Some(command) = notify::command(
        &settings.notification,
        nwo.as_deref().unwrap_or_default(),
        title,
        message,
    ) else {
        // No notifier is a fact about the machine, not a failure of the thing being
        // announced. Say it on stderr and carry on.
        eprintln!("adjutant: no notifier is configured ({title}: {message})");
        return Ok(());
    };
    if dry_run {
        println!("{command}");
        return Ok(());
    }
    terminal::run_shell(&command)?;
    Ok(())
}

// ── worktree ─────────────────────────────────────────────────────────

pub struct WorktreeArgs<'a> {
    pub repo: Option<&'a str>,
    /// A branch you already know. Answers the path only.
    pub branch: Option<&'a str>,
    /// A task name (`app-1234`). Answers the branch *and* the path, as JSON — the two are
    /// always needed together, and deriving them in two places is how they drift apart.
    pub name: Option<&'a str>,
    pub user: Option<&'a str>,
    /// The selected task source's `branchPattern`, when it has one.
    pub pattern: Option<&'a str>,
}

pub fn worktree_path(args: &WorktreeArgs<'_>) -> Result<(), String> {
    let ctx = context_without_hub(args.repo)?;
    let layout = ctx
        .settings
        .worktree_pattern
        .as_deref()
        .unwrap_or(repo::DEFAULT_WORKTREE_PATTERN);

    if let Some(branch) = args.branch {
        println!(
            "{}",
            repo::worktree_fallback(layout, &ctx.repo.main, branch)?
        );
        return Ok(());
    }
    let Some(name) = args.name else {
        return Err("pass either --branch or --name".to_string());
    };
    let user = match args.user {
        Some(user) => user.to_string(),
        // Not guessed from the remote: the branch prefix people use is their forge login,
        // which is not always the local account name — so the caller passes it, and this is
        // only the last resort.
        None => std::env::var("USER").unwrap_or_else(|_| "worker".to_string()),
    };
    let branch = repo::branch_fallback(
        args.pattern.unwrap_or(repo::DEFAULT_BRANCH_PATTERN),
        &user,
        name,
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "branch": branch,
            "path": repo::worktree_fallback(layout, &ctx.repo.main, &branch)?,
            // Named for what it is: the checkout to run `git worktree add` *in*. It was
            // called `base`, and the procedure duly passed it where git wants a commit-ish
            // — which is a path, so every worktree creation failed with `fatal: invalid
            // reference`. What to branch *from* is the `baseBranch` rule, and that is a
            // question about the repository's branches rather than about this path.
            "main": ctx.repo.main,
        }))
        .unwrap_or_default()
    );
    Ok(())
}

// ── hub (the launcher) ─────────────────────────────────────────

/// Start this repository's hub, here, once.
///
/// Say that the hub is already up, and bring it forward. The answer to both "someone got
/// here first" and "it was already running when we looked".
fn go_to_running_hub(
    ctx: &Context,
    status: &messaging::HubStatus,
    dry_run: bool,
) -> Result<(), String> {
    println!(
        "{} is already running (pid {})",
        ctx.repo.hub_name,
        status.pid.unwrap_or(0)
    );
    if let Some(pid) = status.pid {
        let _ = terminal::focus(
            ctx.settings.terminal.focus.as_deref(),
            pid,
            &ctx.repo.hub_name,
            dry_run,
        );
    }
    Ok(())
}

/// The environment the hub's agent is started with.
///
/// `agentEnv` as configured, and then the two things this invocation was told that the agent
/// has no other way to learn: which hub it is, and whether it was asked to collect the
/// dashboard at startup. Appended rather than merged so that ours is the later assignment on
/// the `env` line and therefore the one that takes: a config naming either variable is
/// describing a default, not overruling the flag that was just typed.
///
/// Neither is added when it was not asked for, and that is deliberate rather than tidy: the
/// command line a plain `adj hub` prints has to stay exactly what it printed before, or every
/// existing dry run, doc and expectation of it is wrong.
fn hub_env(ctx: &Context, dashboard: Option<bool>) -> Vec<(String, String)> {
    let mut env = ctx.settings.agent_env.clone();
    if let Some(hub) = &ctx.repo.hub {
        env.push((messaging::HUB_ENV.to_string(), hub.clone()));
    }
    // Appended for the reason above, and absent when no flag was typed for the reason above
    // that: `--dashboard` and `--no-dashboard` are this invocation overruling the standing
    // `startupDashboard`, and a variable set unconditionally would make every hub's command
    // line carry an override nobody asked for.
    if let Some(on) = dashboard {
        env.push((
            config::STARTUP_DASHBOARD_ENV.to_string(),
            if on { "1" } else { "0" }.to_string(),
        ));
    }
    env
}

/// Open a tab and start this repository's hub in it, rather than becoming it here.
///
/// What the tab runs is `adj hub` — this same command without `--tab`. The claim is left to
/// it, and that is the whole reason the split exists: a claim records the claiming process's
/// PID, so claiming here would write down a launcher that is about to exit, for a hub that
/// is a different process in another tab. Every later liveness check would then be asking
/// about the wrong one, and the first `--hub` that answered "gone" would start a second hub
/// beside the live one. See the comment above the claim in `hub`.
///
/// The *resolved* identifier goes on the line rather than the flag, for the reason `work`
/// spells out: the caller most likely to open a tab for a hub is another hub, running this
/// as its own child with no flag at all and carrying the answer in its environment — and
/// that environment does not survive the trip through the terminal.
fn open_hub_tab(
    ctx: &Context,
    repo_arg: Option<&str>,
    extra: &[String],
    start: HubStart,
    dashboard: Option<bool>,
    dry_run: bool,
) -> Result<(), String> {
    let mut parts = forwarded_env();
    parts.extend([exe_path(), "hub".to_string()]);
    if let Some(repo) = repo_arg {
        parts.push("--repo".to_string());
        parts.push(repo.to_string());
    }
    // One argument rather than two, as in `work`: an identifier that starts with a dash
    // reaches here from `ADJUTANT_HUB`, where no flag parser has seen it.
    if let Some(hub) = &ctx.repo.hub {
        parts.push(format!("--hub={hub}"));
    }
    // Forwarded on the line, not through the environment: this route never reaches
    // `hub_env`, and the tab is opened by a terminal that is handed a command string and
    // nothing else. Left off, `adj hub --tab --no-dashboard` would open a tab running a
    // plain `adj hub` — the flag accepted, acknowledged, and silently dropped at the door.
    //
    // Above the separator, and that matters: everything after `--` is clap's trailing
    // argument at the far end, so a flag placed below here would be forwarded as an extra
    // argument to the *agent* rather than parsed by the `adj hub` that starts it.
    match dashboard {
        Some(true) => parts.push("--dashboard".to_string()),
        Some(false) => parts.push("--no-dashboard".to_string()),
        None => {}
    }
    // Above the separator for the same reason, and dropped just as silently if it were not
    // here: the tab would decide for itself what it had been told. `Auto` is left to it —
    // deciding is what a plain `adj hub` does.
    match start {
        HubStart::Resume => parts.push("--resume".to_string()),
        HubStart::New => parts.push("--new".to_string()),
        HubStart::Auto => {}
    }
    // The separator is put back because clap takes everything after it as the trailing
    // argument, and `strip_separator` at the far end takes it off again.
    if !extra.is_empty() {
        parts.push("--".to_string());
        parts.extend(extra.iter().cloned());
    }
    let name_it = title_command(&ctx.settings, &ctx.repo.hub_name);
    let done = terminal::spawn(
        ctx.settings.terminal.spawn.as_deref(),
        &SpawnRequest {
            // The main checkout, never a worktree: a hub that cannot cut worktrees is not a
            // hub, and this is the one thing `hub` moves to before it starts.
            cwd: &ctx.repo.main,
            title: &ctx.repo.hub_name,
            command: &crate::template::sh_join(&parts),
            title_command: name_it.as_deref(),
        },
        dry_run,
    )?;
    if dry_run {
        println!("{}", done.script);
    } else {
        println!("{}", done.description);
    }
    Ok(())
}

/// Three things go wrong when a person types the agent command by hand, and this exists to
/// take all three away: the session name has to match what a worker will look for, the hub
/// has to run in the main checkout or it cannot cut worktrees, and a second hub for the same
/// repo makes it luck which one a report reaches.
///
/// The process registers itself and then *replaces* itself with the agent, so the recorded
/// PID belongs to the live agent rather than to a launcher that has already exited.
///
/// `tab` opens a tab and starts it there instead, for a caller that is not a person sitting
/// at an empty one — nothing else about the decision changes, including which of the two
/// tabs claims the record.
/// `dashboard` is `--dashboard` / `--no-dashboard`, and `None` when neither was typed — the
/// standing `startupDashboard` then answers on its own. It is carried to the agent as an
/// environment variable rather than resolved here, because the thing that reads it is the
/// hub's procedure, which asks `adjutant_config` for the *resolved* settings.
pub fn hub(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    extra: &[String],
    tab: bool,
    start: HubStart,
    dashboard: Option<bool>,
    dry_run: bool,
) -> Result<(), String> {
    use std::os::unix::process::CommandExt;

    // `context_as`, not `context`: this command is run from anywhere in the repository,
    // worktrees included, and a hub that took its identity from whichever worktree it was
    // typed in would be a different hub every time.
    let ctx = context_as(repo_arg, hub_arg)?;
    if ctx.repo.nwo_source == "dirname" {
        eprintln!(
            "adjutant: origin gave no repository name, using the directory name {}",
            ctx.repo.nwo
        );
    }
    // The directory move comes first, and not only because the hub has to run there: a
    // relative `ADJUTANT_STATE_DIR` is resolved against the working directory, so looking or
    // claiming before moving reads and writes the record under wherever `adj hub` happened to
    // be typed, and the hub then goes looking for it somewhere else. It is above the check
    // rather than beside the claim because every route below this line reads that record: the
    // claim used to be the only one, and recovered a missed record by answering `Taken`, which
    // the tab route has no equivalent of — it would open a tab for a hub already running.
    // Nothing has been written at this point, so a failure here has nothing to undo.
    std::env::set_current_dir(&ctx.repo.main)
        .map_err(|e| format!("cannot change directory to {}: {e}", ctx.repo.main))?;

    // Asked before the command is even built, so the common "it is already up" case costs
    // nothing. It is not what *enforces* one hub per repository — the claim below is.
    let status = messaging::hub_status(&ctx.repo.slug, &ctx.repo.hub_name);
    if status.present {
        return go_to_running_hub(&ctx, &status, dry_run);
    }
    // Looked up before either route, so that asking for a session that is not there is
    // refused here, in the tab it was typed in — not in a tab opened to show the refusal.
    // The template is checked here too, for the same reason: on the tab route the refusal
    // would otherwise come from inside a tab this one had already reported as opened.
    let asked = match start {
        HubStart::Resume => {
            let saved = saved_hub_session(&ctx)?;
            resume_template(ctx.settings.hub_resume_runner.as_deref(), "hubResumeRunner")?;
            Some(saved)
        }
        HubStart::Auto | HubStart::New => None,
    };
    // Below the presence check, and deliberately: one hub per address is the invariant, and
    // opening a tab for one that is already up would break it in the one way nothing later
    // repairs — two sessions answering to the same name, with the record naming one of them.
    if tab {
        // `present: false` answers two different questions the same way: nobody is there,
        // and whether anybody is there could not be established — an unreadable record, or
        // a `ps` that would not run. Only the first is a reason to start a hub, and the
        // other route never has to tell them apart because its claim refuses the second in
        // exactly these words. This one leaves the claim to the tab it opens, so the
        // refusal happens here or nowhere — and nowhere means the caller this route exists
        // for, which is not a person, is told a hub was started in a new tab and handed
        // `exit 0`, for a hub whose own claim is about to refuse it.
        //
        // `Alive` is a hub running under a name the check above no longer matches on. The
        // tab's own claim would bring it forward, so the hub ends up in the same place
        // either way — but this side would have said it started one and exited 0 for a hub
        // that was already up, which is the same untruth told to the same non-human caller.
        // It is brought forward from here instead, and no tab is opened for it.
        match messaging::hub_liveness(&ctx.repo.slug) {
            messaging::Liveness::CannotTell => {
                return Err(messaging::hub_cannot_tell(&ctx.repo.slug));
            }
            messaging::Liveness::Alive => return go_to_running_hub(&ctx, &status, dry_run),
            messaging::Liveness::Gone => {}
        }
        return open_hub_tab(&ctx, repo_arg, extra, start, dashboard, dry_run);
    }
    // Only on this route: the tab route hands the question to the `adj hub` in the new tab,
    // which asks it a moment later with the same answer.
    let resumed = match start {
        HubStart::Auto => recent_hub_session(&ctx),
        _ => asked,
    };
    // The id a fresh hub is started into, when its runner has somewhere to put one. Written
    // down only once the claim is won, below: a launch that loses the claim started nothing,
    // and saving its id would point the next `--resume` at a conversation that never began.
    let session = match &resumed {
        Some(saved) => saved.session_id.clone(),
        None => messaging::new_session_id()?,
    };
    let records = resumed.is_some()
        || runner::records_session(
            ctx.settings.hub_runner.as_deref(),
            runner::DEFAULT_HUB_RUNNER,
        );
    let mut env = hub_env(&ctx, dashboard);
    // The session this hub runs as, for the MCP server the agent is about to start: it is
    // what keeps `lastAlive` current, and what the next plain `adj hub` reads to decide
    // whether to come back to this one. Absent for a runner that records no session, since
    // there would be nothing to come back to.
    if records {
        env.push((
            messaging::HUB_SESSION_ENV.to_string(),
            messaging::hub_session_env(&ctx.repo.slug, &session),
        ));
    }
    let mut command = match &resumed {
        Some(_) => runner::hub_resume_command(
            resume_template(ctx.settings.hub_resume_runner.as_deref(), "hubResumeRunner")?,
            &env,
            &ctx.repo.hub_name,
            &session,
            runner::HUB_RESUME_PROMPT,
        ),
        None => runner::hub_command(
            ctx.settings.hub_runner.as_deref(),
            &env,
            &ctx.repo.hub_name,
            &session,
            runner::HUB_STARTUP_PROMPT,
        ),
    };
    if !extra.is_empty() {
        command = format!("{command} {}", crate::template::sh_join(extra));
    }
    if dry_run {
        println!("cd {}", crate::template::sh_quote(&ctx.repo.main));
        println!("{command}");
        return Ok(());
    }

    // Only past the dry run: claiming the record is a write, and a dry run that cleared a
    // live hub's registration would make that hub permanently unreachable — nothing then
    // reports it as present, and every later launch starts another one beside it.
    //
    // Two launches can reach this line at the same time (a person and a wake-up, two tabs).
    // The claim is what decides between them; the loser is told who won, exactly as if it
    // had arrived a second later.
    // Whether the *rendered* command carries the name, not whether the template has a
    // `{name}` in it: a template that hardcodes the name works, and one that renders it
    // away does not, and only the finished line knows which.

    let named = command.contains(&ctx.repo.hub_name);
    match messaging::claim_hub(&ctx.repo.slug, &ctx.repo.hub_name, &ctx.repo.main, named)? {
        messaging::Claim::Ours => {}
        messaging::Claim::Taken(status) => return go_to_running_hub(&ctx, &status, dry_run),
    }
    // A hub that cannot be resumed later is still a hub, so failing to write this down is
    // said and then got past — refusing to start over it would trade a working hub for a
    // convenience.
    //
    // A fresh hub whose runner records no session replaces what was saved with nothing, so
    // that nothing can later reopen the conversation of the hub before it.
    if resumed.is_none() {
        let saved = match records {
            true => messaging::save_hub_session(
                &ctx.repo.slug,
                &ctx.repo.nwo,
                ctx.repo.hub.as_deref(),
                &ctx.repo.hub_name,
                &session,
            )
            .map(|_| ()),
            false => messaging::forget_hub_session(&ctx.repo.slug),
        };
        if let Err(e) = saved {
            eprintln!("adjutant: {e}; --resume may not reopen this hub");
        }
    }
    match &resumed {
        Some(saved) => println!(
            "resuming {} (session {}) in {}",
            ctx.repo.hub_name, saved.session_id, ctx.repo.main
        ),
        None => println!("starting {} in {}", ctx.repo.hub_name, ctx.repo.main),
    }

    // `exec` keeps the PID, which is the whole point: the record written a line ago has to
    // name the process a worker will later check for.
    // Removed and then set on the line itself, so the only value the agent — and so its MCP
    // server — can see is this hub's own, never one inherited from whatever started this.
    let error = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .env_remove(messaging::HUB_SESSION_ENV)
        .exec();
    // Only reachable if exec failed — otherwise this process no longer exists.
    let _ = messaging::unregister_hub(&ctx.repo.slug);
    Err(format!("cannot start the hub: {error}"))
}

/// How `adj hub` decides between a new session and the one it had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubStart {
    /// Resume when the last session ended within `hubAutoResumeHours`, start fresh otherwise.
    Auto,
    /// `--resume`: the saved session, or a refusal.
    Resume,
    /// `--new`: a fresh session whatever was saved.
    New,
}

/// The saved session a plain `adj hub` comes back to, if it ended recently enough.
///
/// "Ended" is the last time the hub's MCP server said the session was alive — it beats
/// every minute and once more as the agent closes it. Where nothing has said so (no MCP
/// server, a runner that is not the agent this knows, a session saved by an older version)
/// there is no answer, and no answer starts fresh: coming back uninvited to a conversation of
/// unknown age is worse than one clean start too many.
fn recent_hub_session(ctx: &Context) -> Option<messaging::SavedSession> {
    let window = ctx.settings.hub_auto_resume_hours;
    if window <= 0.0 {
        return None;
    }
    let saved = messaging::hub_session(&ctx.repo.slug)?;
    let last = messaging::hub_last_alive(&ctx.repo.slug, &saved.session_id)?;
    let age = messaging::now_secs().saturating_sub(last).max(0);
    if age as f64 > window * 3600.0 {
        return None;
    }
    // A resume template that cannot be told the session would make this refuse, and a
    // refusal is the wrong answer to a command that was not asked to resume anything.
    if resume_template(ctx.settings.hub_resume_runner.as_deref(), "hubResumeRunner").is_err() {
        eprintln!("adjutant: not resuming the last session: hubResumeRunner has no {{sessionId}}");
        return None;
    }
    // A hub started by a runner of its own would be reopened by the built-in one — without
    // whatever that runner added, or as another agent entirely. Asked for outright, that is
    // the person's call and `--resume` makes it; uninvited, it is not.
    if ctx.settings.hub_runner.is_some() && ctx.settings.hub_resume_runner.is_none() {
        eprintln!(
            "adjutant: not resuming the last session: hubRunner is your own and hubResumeRunner \
             is not set, so the built-in one would reopen it"
        );
        return None;
    }
    eprintln!(
        "adjutant: resuming the session that ended {} ago (within hubAutoResumeHours); \
         `adj hub --new` starts a fresh one instead",
        ago(age)
    );
    Some(saved)
}

/// "12 min", "2 h 5 min": how long ago, the way a person reads it.
fn ago(secs: i64) -> String {
    let minutes = secs / 60;
    match minutes {
        0 => "less than a minute".to_string(),
        1..=59 => format!("{minutes} min"),
        _ => format!("{} h {} min", minutes / 60, minutes % 60),
    }
}

/// The session `adj hub --resume` reopens, or a refusal that says what can be resumed.
///
/// Asking for a hub that has nothing saved is most often asking for the wrong one — the
/// repository's own hub when it was a parent task's, or the other way round — so the refusal
/// lists what this repository does have rather than only saying "no".
fn saved_hub_session(ctx: &Context) -> Result<messaging::SavedSession, String> {
    if let Some(saved) = messaging::hub_session(&ctx.repo.slug) {
        return Ok(saved);
    }
    let mut message = format!("{} has no saved session to resume.", ctx.repo.hub_name);
    let others = messaging::hub_sessions_for(&ctx.repo.nwo);
    if others.is_empty() {
        message.push_str(&format!(" No hub of {} has one.", ctx.repo.nwo));
    } else {
        message.push_str(" These can be resumed:");
        for other in others {
            let command = match &other.hub {
                Some(hub) => format!("adj hub --resume --hub {}", crate::template::sh_quote(hub)),
                None => "adj hub --resume".to_string(),
            };
            message.push_str(&format!("\n  {command}"));
        }
    }
    message.push_str(
        "\nA session is saved when a hub is started by a runner that takes {sessionId} \
         (the built-in one does). Start a new one with `adj hub`.",
    );
    Err(message)
}

/// The resume template to use, refusing one that has nowhere to put the session id.
///
/// Without `{sessionId}` the agent is not told which conversation to reopen, and opens
/// whichever one it would pick on its own — which for a hub in the main checkout is as likely
/// to be somebody's unrelated work. Refusing is the one answer that cannot be that.
fn resume_template<'a>(configured: Option<&'a str>, key: &str) -> Result<Option<&'a str>, String> {
    match configured {
        Some(template) if !runner::records_session(Some(template), "") => Err(format!(
            "{key} has no {{sessionId}}, so it cannot be told which session to reopen"
        )),
        other => Ok(other),
    }
}

/// Start the worker agent in the tab `work` just opened.
///
/// The mirror image of `hub`: write down who we are, then become the agent. Running the
/// agent as a child instead would record a PID that exits the moment the agent does
/// anything, and waking a dead launcher wakes nobody.
pub fn worker(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    worktree: Option<&str>,
    title: Option<&str>,
    prompt: Option<&str>,
    resume: bool,
    dry_run: bool,
) -> Result<(), String> {
    use std::os::unix::process::CommandExt;

    let worktree = worker_worktree(worktree)?;
    let resumed = match resume {
        true => Some(saved_worker_session(&worktree)?),
        false => None,
    };
    // This tab was opened *at* the worktree, so `context` would read the record this is
    // about to replace. A worker that crashed without being closed leaves one behind, and
    // re-dispatching that task would file the new worker under the hub that ran the old.
    //
    // A resumed worker goes back under the hub that dispatched it, which the saved session
    // remembers — ahead of `ADJUTANT_HUB`, because the tab someone types `--resume` into
    // may have inherited that from a different hub entirely. Only an explicit `--hub`
    // outranks it.
    let ctx = match &resumed {
        Some(saved) => {
            let told = hub_arg.map(str::trim).filter(|hub| !hub.is_empty());
            context_of(repo::resolve(repo_arg, told.or(saved.hub.as_deref()))?)?
        }
        None => context_as(repo_arg, hub_arg)?,
    };
    let status = messaging::worker_status(&worktree);
    if status.present {
        println!(
            "a worker is already running in this worktree (pid {})",
            status.pid.unwrap_or(0)
        );
        // `adj work` marked this worktree on the way here, and nobody is going to register
        // over it. Left, it would hold a second slot for the grace period after the running
        // worker ends.
        let _ = messaging::unmark_worker_starting(&worktree);
        return Ok(());
    }

    let worktree_text = worktree.to_string_lossy().to_string();
    let title = title
        .filter(|title| !title.is_empty())
        .or(resumed.as_ref().and_then(|saved| saved.title.as_deref()))
        .unwrap_or("")
        .to_string();
    let (command, fresh_session) = match &resumed {
        Some(saved) => {
            let template = resume_template(
                ctx.settings.agent_resume_runner.as_deref(),
                "agentResumeRunner",
            )?;
            let command = runner::worker_resume_command(
                template,
                &ctx.settings.agent_env,
                &saved.session_id,
                prompt.unwrap_or(runner::WORKER_RESUME_PROMPT),
                &worktree_text,
                &title,
            );
            (command, None)
        }
        None => {
            let session = messaging::new_session_id()?;
            let command = runner::worker_command(
                ctx.settings.agent_runner.as_deref(),
                &ctx.settings.agent_env,
                &session,
                prompt.unwrap_or(runner::WORKER_STARTUP_PROMPT),
                &worktree_text,
                &title,
            );
            let records = runner::records_session(
                ctx.settings.agent_runner.as_deref(),
                runner::DEFAULT_AGENT_RUNNER,
            );
            (command, records.then_some(session))
        }
    };
    if dry_run {
        println!("cd {}", crate::template::sh_quote(&worktree_text));
        println!("{command}");
        return Ok(());
    }

    std::env::set_current_dir(&worktree)
        .map_err(|e| format!("cannot change directory to {}: {e}", worktree.display()))?;
    // The address goes into the record here, at the last moment before this process stops
    // being a launcher. Everything the worker's agent later sends is addressed from it.
    messaging::register_worker(&worktree, &title, ctx.repo.hub.as_deref())?;
    // Said and got past, as for the hub: a worker that cannot be resumed still works. And as
    // for the hub, a fresh start with nothing to record clears what an earlier worker saved.
    if resumed.is_none() {
        let saved = match &fresh_session {
            Some(session) => {
                messaging::save_worker_session(&worktree, &title, ctx.repo.hub.as_deref(), session)
                    .map(|_| ())
            }
            None => messaging::forget_worker_session(&worktree),
        };
        if let Err(e) = saved {
            eprintln!("adjutant: {e}; --resume may not reopen this worker");
        }
    }

    // A worker is not a hub. A tab opened by a spawn command that passes its environment on
    // would otherwise hand the hub's session to this agent's MCP server, which would then
    // keep saying the hub is alive for as long as the worker runs.
    let error = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .env_remove(messaging::HUB_SESSION_ENV)
        .exec();
    let _ = messaging::unregister_worker(&worktree);
    Err(format!("cannot start the worker: {error}"))
}

/// The worktree a worker runs in: the one named, or — for `--resume`, typed by a person
/// standing in it — the one this command was run from.
fn worker_worktree(worktree: Option<&str>) -> Result<std::path::PathBuf, String> {
    let worktree = match worktree {
        Some(path) => config::expand_home(path),
        None => repo::current_worktree(None)
            .map(std::path::PathBuf::from)
            .ok_or("not inside a git worktree; pass --worktree")?,
    };
    if !worktree.is_dir() {
        return Err(format!("no such worktree: {}", worktree.display()));
    }
    Ok(worktree)
}

/// The session `--resume` reopens in `worktree`, or a refusal that says why there is none.
fn saved_worker_session(worktree: &std::path::Path) -> Result<messaging::SavedSession, String> {
    messaging::worker_session(worktree).ok_or_else(|| {
        format!(
            "no saved worker session in {}: a session is saved when a worker is started by \
             `adj work` with a runner that takes {{sessionId}} (the built-in one does)",
            worktree.display()
        )
    })
}

/// Leave a message for the worker in a worktree, and poke it if it is sitting there.
/// What became of a message left for a worker.
pub struct Told {
    pub path: std::path::PathBuf,
    pub present: bool,
    pub woken: bool,
}

/// Append to a worktree's outbox, poke the worker sitting in it, and tell the person when
/// poking was not possible.
///
/// Shared by `adj tell` and by a gate's answer, which is the point: both are the hub-to-
/// worker direction, and the rule about when to wake and when to notify is one rule. A
/// worker that was woken reads the message itself, so the notification is what happens
/// *instead* — unlike the other direction, where the hub is unattended and the person is
/// told either way.
pub fn deliver_to_worker(
    ctx: &Context,
    worktree: &std::path::Path,
    from: &str,
    subject: &str,
    body: &str,
) -> Result<Told, String> {
    let path = messaging::tell(worktree, from, subject, body)?;
    let status = messaging::worker_status(worktree);
    let woken = match (status.present, status.pid) {
        (true, Some(pid)) => terminal::wake(
            &ctx.settings.worker_wake,
            pid,
            subject,
            terminal::WORKER_WAKE_LINE,
            false,
        )
        .map(|done| done.ran)
        .unwrap_or(false),
        _ => false,
    };
    if !woken
        && let Some(command) = notify::repo_command(&ctx.settings.notification, &ctx.repo, subject)
    {
        let _ = terminal::run_shell(&command);
    }
    Ok(Told {
        path,
        present: status.present,
        woken,
    })
}

pub fn tell(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    worktree: &str,
    subject: &str,
    body: Option<&str>,
    from: Option<&str>,
    quiet: bool,
) -> Result<(), String> {
    let ctx = context(repo_arg, hub_arg)?;
    let worktree = config::expand_home(worktree);
    if !worktree.is_dir() {
        return Err(format!("no such worktree: {}", worktree.display()));
    }
    let body = read_body(body)?;
    let from = from.unwrap_or(&ctx.repo.hub_name).to_string();
    let Told {
        path,
        present,
        woken,
    } = deliver_to_worker(&ctx, &worktree, &from, subject, &body)?;

    if quiet {
        return Ok(());
    }
    println!("wrote {}", path.display());
    match (present, woken) {
        (true, true) => println!("Woke the worker."),
        (true, false) => {
            println!("The worker is running; it will read this the next time it checks its outbox.")
        }
        (false, _) => {
            println!("The worker is not running; it will read this the next time it starts.")
        }
    }
    Ok(())
}

/// Print one of the procedures.
///
/// The same text the MCP prompt and `adjutant_skill` serve. It exists as a subcommand
/// because a shell command is the one way in that every agent has: MCP prompt support
/// differs between agents, and a tool has to be loaded before it can be called — a woken
/// session that cannot reach its procedure is a session that does nothing.
pub fn skill(name: &str, arguments: &str, agent: Option<&str>) -> Result<(), String> {
    let prompt = crate::prompts::find(name).ok_or_else(|| {
        format!(
            "no such procedure: {name} ({})",
            crate::prompts::PROMPTS
                .iter()
                .map(|p| p.name)
                .collect::<Vec<_>>()
                .join(" / ")
        )
    })?;
    let runner = crate::mcp::resolve_runner_for(None, name);
    let resolved_agent = crate::prompts::resolve_agent(agent, None, runner.as_deref());
    print!(
        "{}",
        crate::prompts::render_for(prompt, arguments, resolved_agent)
    );
    Ok(())
}

/// What the hub has left for the worker in this worktree.
pub fn outbox(worktree: Option<&str>, clear: bool) -> Result<(), String> {
    let worktree = match worktree {
        Some(path) => config::expand_home(path),
        None => std::env::current_dir()
            .map_err(|e| format!("cannot determine the current directory: {e}"))?,
    };
    if clear {
        messaging::clear_outbox(&worktree)?;
        println!("cleared the outbox");
        return Ok(());
    }
    let text = messaging::read_outbox(&worktree);
    if text.trim().is_empty() {
        println!("(empty)");
        return Ok(());
    }
    print!("{text}");
    Ok(())
}

/// Remove this repo's hub record. For a hub shutting down cleanly, and for clearing a record
/// left behind by one that did not.
pub fn hub_stop(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<(), String> {
    let info = resolve(repo_arg, hub_arg)?;
    messaging::unregister_hub(&info.slug)?;
    println!("unregistered {}", info.hub_name);
    Ok(())
}

// ── shared ───────────────────────────────────────────────────────────

/// A value given as `-` is read from stdin, trailing newlines dropped.
///
/// For text that came from somewhere else — a task's title, the reason a start failed, a
/// comment typed on the board. On a command line it has to be quoted, and whatever quote is
/// chosen, the text can close it and run the rest as shell. Written to a file by the agent's
/// file tool and redirected in, it never passes through the shell at all.
fn dash_is_stdin(value: &str) -> Result<String, String> {
    if value != "-" {
        return Ok(value.to_string());
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("cannot read stdin: {e}"))?;
    Ok(buf.trim_end_matches(['\n', '\r']).to_string())
}

/// The message body, from the flag or from stdin. Long reports do not belong on a command
/// line, and the two ways in should behave identically.
fn read_body(body: Option<&str>) -> Result<String, String> {
    let body = match body {
        // `-` is stdin, as `task add --help` says and as most tools read it. Taken literally
        // it became a body of one dash, and the card's title with it.
        Some(body) if body != "-" => body.to_string(),
        _ => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| format!("cannot read stdin: {e}"))?;
            buf
        }
    };
    if body.trim().is_empty() {
        return Err("the body is empty: pass it with --body or on stdin".to_string());
    }
    Ok(body)
}

/// This binary, for commands that have to name themselves in a command line handed to a
/// terminal. The absolute path rather than `adjutant`, so a new tab whose PATH is not yet
/// loaded still finds it.
fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "adjutant".to_string())
}

/// Settings without insisting on a resolvable repository.
///
/// `spawn` and `notify` are useful from anywhere, including outside a checkout, and failing
/// them because `git` had nothing to say would be answering a question nobody asked.
fn settings_for(repo_arg: Option<&str>) -> Settings {
    let nwo = match repo_arg {
        Some(arg) => arg.to_string(),
        None => repo::resolve(None, None).map(|i| i.nwo).unwrap_or_default(),
    };
    config::resolve_config(&nwo)
        .map(|r| r.settings)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hub told where to read and write has to hand both to the tabs it opens. Losing
    /// them does not fail: the worker registers in the default world and reports into an
    /// inbox the hub is not watching, and both halves look healthy from where they stand.
    #[test]
    fn a_tab_is_handed_the_config_and_state_directory_it_must_not_lose() {
        let _sandbox = crate::testing::Sandbox::empty();
        let parts = forwarded_env();
        assert_eq!(parts.first().map(String::as_str), Some("env"));
        assert!(
            parts
                .iter()
                .any(|p| p.starts_with(&format!("{}=", config::CONFIG_ENV))),
            "{parts:?}"
        );
        assert!(
            parts
                .iter()
                .any(|p| p.starts_with(&format!("{}=", messaging::STATE_DIR_ENV))),
            "{parts:?}"
        );
    }

    /// A machine that never sets them gets the command line it always had — no `env`
    /// prefix, nothing to read past.
    #[test]
    fn nothing_is_forwarded_when_nothing_was_given() {
        let _sandbox = crate::testing::Sandbox::empty();
        unsafe {
            std::env::remove_var(config::CONFIG_ENV);
            std::env::remove_var(messaging::STATE_DIR_ENV);
        }
        assert!(forwarded_env().is_empty());
    }
    use crate::messaging::Liveness;

    /// A `look` that answers down a script and then keeps repeating its last word, so a
    /// test can say "still there twice, then gone" without owning a process to kill.
    fn answers(script: &[Liveness]) -> impl FnMut() -> Liveness + '_ {
        let mut n = 0;
        move || {
            let answer = script[n.min(script.len() - 1)];
            n += 1;
            answer
        }
    }

    /// What `close` acts on is the worker's own absence rather than anything the close
    /// command said. Waiting for that has to be bounded, must not read "cannot tell" as
    /// "gone", and must not cost a real two seconds every time it is tested.
    #[test]
    fn waiting_for_a_worker_to_go_looks_again_but_not_forever() {
        let budget = Duration::from_secs(2);
        let poll = Duration::from_millis(100);

        // Already gone at the first look: nothing is waited for at all, which is the common
        // case and the reason this polls instead of sleeping the budget.
        let mut waits = 0;
        assert_eq!(
            settled(answers(&[Liveness::Gone]), |_| waits += 1, budget, poll),
            Liveness::Gone
        );
        assert_eq!(waits, 0);

        // Still unwinding for the first two looks, then gone: it stops looking the moment
        // it has its answer.
        let mut waits = 0;
        assert_eq!(
            settled(
                answers(&[Liveness::Alive, Liveness::Alive, Liveness::Gone]),
                |_| waits += 1,
                budget,
                poll
            ),
            Liveness::Gone
        );
        assert_eq!(waits, 2);

        // Never goes: bounded by the budget, and the answer is what was seen rather than
        // what the caller was hoping for.
        let mut waits = 0;
        assert_eq!(
            settled(answers(&[Liveness::Alive]), |_| waits += 1, budget, poll),
            Liveness::Alive
        );
        assert_eq!(waits, 20);

        // "Cannot tell" is not "gone", however long it is asked. This is the answer that
        // gets folded into "nobody there" everywhere a message is being delivered, and
        // folding it here is what removes a live worker's worktree.
        assert_eq!(
            settled(answers(&[Liveness::CannotTell]), |_| {}, budget, poll),
            Liveness::CannotTell
        );

        // A zero interval is a caller's mistake, not a reason to spin forever.
        assert_eq!(
            settled(answers(&[Liveness::Alive]), |_| {}, budget, Duration::ZERO),
            Liveness::Alive
        );
    }
}
