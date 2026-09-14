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
    let subject = messaging::header_value(&messaging::render_message(&message), "subject")
        .unwrap_or_default();
    let delivery = messaging::send(&ctx.repo.slug, &ctx.repo.hub_name, &message)?;

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
    if let Some(command) = notify::repo_command(&ctx.settings.notification, &ctx.repo, &subject) {
        let _ = terminal::run_shell(&command);
    }

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
    Some(crate::template::sh_join(&[
        exe_path(),
        "title".to_string(),
        "--title".to_string(),
        title.to_string(),
    ]))
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
pub fn work(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    worktree: &str,
    title: &str,
    prompt: &str,
    dry_run: bool,
) -> Result<(), String> {
    // The dispatching side: the identifier being handed to the new worker is this caller's
    // own, never one read out of some worktree it happens to be standing in.
    let ctx = context_as(repo_arg, hub_arg)?;
    let worktree = config::expand_home(worktree).to_string_lossy().to_string();
    // The tab runs `adjutant worker`, not the agent directly. The agent is started by a
    // process that has already written down its own PID and then `exec`s itself away, which
    // is the only way anyone later gets to ask "is that worker still there".
    let mut parts = vec![
        exe_path(),
        "worker".to_string(),
        "--worktree".to_string(),
        worktree.clone(),
        "--title".to_string(),
        title.to_string(),
        "--prompt".to_string(),
        prompt.to_string(),
    ];
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
    let done = terminal::spawn(
        ctx.settings.terminal.spawn.as_deref(),
        &SpawnRequest {
            cwd: &worktree,
            title,
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
    let done = terminal::set_title(&settings.terminal.title, title, dry_run)?;
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
/// `agentEnv` as configured, and then the hub identifier when there is one. Appended rather
/// than merged so that ours is the later assignment on the `env` line and therefore the one
/// that takes: a config naming this variable is describing a default, not overruling the
/// `--hub` that was just typed.
///
/// Nothing is added when there is no identifier, and that is deliberate rather than tidy:
/// the command line a plain `adj hub` prints has to stay exactly what it printed before, or
/// every existing dry run, doc and expectation of it is wrong.
fn hub_env(ctx: &Context) -> Vec<(String, String)> {
    let mut env = ctx.settings.agent_env.clone();
    if let Some(hub) = &ctx.repo.hub {
        env.push((messaging::HUB_ENV.to_string(), hub.clone()));
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
    dry_run: bool,
) -> Result<(), String> {
    let mut parts = vec![exe_path(), "hub".to_string()];
    if let Some(repo) = repo_arg {
        parts.push("--repo".to_string());
        parts.push(repo.to_string());
    }
    // One argument rather than two, as in `work`: an identifier that starts with a dash
    // reaches here from `ADJUTANT_HUB`, where no flag parser has seen it.
    if let Some(hub) = &ctx.repo.hub {
        parts.push(format!("--hub={hub}"));
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
pub fn hub(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    extra: &[String],
    tab: bool,
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
    // Below the presence check, and deliberately: one hub per address is the invariant, and
    // opening a tab for one that is already up would break it in the one way nothing later
    // repairs — two sessions answering to the same name, with the record naming one of them.
    if tab {
        return open_hub_tab(&ctx, repo_arg, extra, dry_run);
    }
    let mut command = runner::hub_command(
        ctx.settings.hub_runner.as_deref(),
        &hub_env(&ctx),
        &ctx.repo.hub_name,
        runner::HUB_STARTUP_PROMPT,
    );
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
    println!("starting {} in {}", ctx.repo.hub_name, ctx.repo.main);

    // `exec` keeps the PID, which is the whole point: the record written a line ago has to
    // name the process a worker will later check for.
    let error = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .exec();
    // Only reachable if exec failed — otherwise this process no longer exists.
    let _ = messaging::unregister_hub(&ctx.repo.slug);
    Err(format!("cannot start the hub: {error}"))
}

/// Start the worker agent in the tab `work` just opened.
///
/// The mirror image of `hub`: write down who we are, then become the agent. Running the
/// agent as a child instead would record a PID that exits the moment the agent does
/// anything, and waking a dead launcher wakes nobody.
pub fn worker(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    worktree: &str,
    title: &str,
    prompt: &str,
    dry_run: bool,
) -> Result<(), String> {
    use std::os::unix::process::CommandExt;

    let worktree = config::expand_home(worktree);
    if !worktree.is_dir() {
        return Err(format!("no such worktree: {}", worktree.display()));
    }
    // This tab was opened *at* the worktree, so `context` would read the record this is
    // about to replace. A worker that crashed without being closed leaves one behind, and
    // re-dispatching that task would file the new worker under the hub that ran the old.
    let ctx = context_as(repo_arg, hub_arg)?;
    let status = messaging::worker_status(&worktree);
    if status.present {
        println!(
            "a worker is already running in this worktree (pid {})",
            status.pid.unwrap_or(0)
        );
        return Ok(());
    }

    let command = runner::worker_command(
        ctx.settings.agent_runner.as_deref(),
        &ctx.settings.agent_env,
        prompt,
        &worktree.to_string_lossy(),
        title,
    );
    if dry_run {
        println!(
            "cd {}",
            crate::template::sh_quote(&worktree.to_string_lossy())
        );
        println!("{command}");
        return Ok(());
    }

    std::env::set_current_dir(&worktree)
        .map_err(|e| format!("cannot change directory to {}: {e}", worktree.display()))?;
    // The address goes into the record here, at the last moment before this process stops
    // being a launcher. Everything the worker's agent later sends is addressed from it.
    messaging::register_worker(&worktree, title, ctx.repo.hub.as_deref())?;

    let error = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .exec();
    let _ = messaging::unregister_worker(&worktree);
    Err(format!("cannot start the worker: {error}"))
}

/// Leave a message for the worker in a worktree, and poke it if it is sitting there.
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
    let path = messaging::tell(&worktree, &from, subject, &body)?;

    let status = messaging::worker_status(&worktree);
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
    if quiet {
        return Ok(());
    }
    println!("wrote {}", path.display());
    match (status.present, woken) {
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
pub fn skill(name: &str, arguments: &str) -> Result<(), String> {
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
    print!("{}", crate::prompts::render(prompt, arguments));
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

/// The message body, from the flag or from stdin. Long reports do not belong on a command
/// line, and the two ways in should behave identically.
fn read_body(body: Option<&str>) -> Result<String, String> {
    let body = match body {
        Some(body) => body.to_string(),
        None => {
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
