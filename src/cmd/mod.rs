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

pub fn context(repo_arg: Option<&str>) -> Result<Context, String> {
    let repo = repo::resolve(repo_arg)?;
    let resolved = config::resolve_config(&repo.nwo)?;
    Ok(Context {
        settings: resolved.settings.clone(),
        repo,
        resolved,
    })
}

// ── hub-name ─────────────────────────────────────────────────────────

pub fn hub_name(repo_arg: Option<&str>, as_json: bool) -> Result<(), String> {
    let info = repo::resolve(repo_arg)?;
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "main": info.main,
                "nwo": info.nwo,
                "repo": info.repo,
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

pub fn show_config(repo_arg: Option<&str>) -> Result<(), String> {
    let ctx = context(repo_arg)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "repo": ctx.repo.nwo,
            "main": ctx.repo.main,
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
    pub path_only: bool,
    pub limit: usize,
    pub as_json: bool,
    pub read: Option<&'a str>,
    pub ack: Option<&'a str>,
}

pub fn pending(args: &PendingArgs<'_>) -> Result<(), String> {
    let info = repo::resolve(args.repo)?;
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
            .map(|e| json!({"name": e.name, "from": e.from, "kind": e.kind, "subject": e.subject}))
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
    pub from: Option<&'a str>,
    pub kind: &'a str,
    pub subject: Option<&'a str>,
    pub body: Option<&'a str>,
    pub quiet: bool,
}

pub fn send(args: &SendArgs<'_>) -> Result<(), String> {
    let ctx = context(args.repo)?;
    let body = read_body(args.body)?;
    let message = Message {
        from: args.from.unwrap_or("unknown").to_string(),
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
    worktree: &str,
    title: &str,
    prompt: &str,
    dry_run: bool,
) -> Result<(), String> {
    let ctx = context(repo_arg)?;
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

pub fn focus(repo_arg: Option<&str>, quiet: bool, dry_run: bool) -> Result<bool, String> {
    let ctx = context(repo_arg)?;
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
        settings.terminal.close.as_deref(),
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
    let ctx = context(repo_arg)?;
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
        None => repo::resolve(None).ok().map(|info| info.nwo),
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
    let ctx = context(args.repo)?;
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

/// Three things go wrong when a person types the agent command by hand, and this exists to
/// take all three away: the session name has to match what a worker will look for, the hub
/// has to run in the main checkout or it cannot cut worktrees, and a second hub for the same
/// repo makes it luck which one a report reaches.
///
/// The process registers itself and then *replaces* itself with the agent, so the recorded
/// PID belongs to the live agent rather than to a launcher that has already exited.
pub fn hub(repo_arg: Option<&str>, extra: &[String], dry_run: bool) -> Result<(), String> {
    use std::os::unix::process::CommandExt;

    let ctx = context(repo_arg)?;
    if ctx.repo.nwo_source == "dirname" {
        eprintln!(
            "adjutant: origin gave no repository name, using the directory name {}",
            ctx.repo.nwo
        );
    }
    // Asked before the command is even built, so the common "it is already up" case costs
    // nothing. It is not what *enforces* one hub per repository — the claim below is.
    let status = messaging::hub_status(&ctx.repo.slug, &ctx.repo.hub_name);
    if status.present {
        return go_to_running_hub(&ctx, &status, dry_run);
    }
    let mut command = runner::hub_command(
        ctx.settings.hub_runner.as_deref(),
        &ctx.settings.agent_env,
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
    // The directory move comes first, and not only because the hub has to run there: a
    // relative `ADJUTANT_STATE_DIR` is resolved against the working directory, so claiming
    // before moving wrote the record under wherever `adj hub` happened to be typed, and the
    // hub then went looking for it somewhere else. Nothing has been written at this point,
    // so a failure here has nothing to undo.
    std::env::set_current_dir(&ctx.repo.main)
        .map_err(|e| format!("cannot change directory to {}: {e}", ctx.repo.main))?;

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
    let ctx = context(repo_arg)?;
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
    messaging::register_worker(&worktree, title)?;

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
    worktree: &str,
    subject: &str,
    body: Option<&str>,
    from: Option<&str>,
    quiet: bool,
) -> Result<(), String> {
    let ctx = context(repo_arg)?;
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
pub fn hub_stop(repo_arg: Option<&str>) -> Result<(), String> {
    let info = repo::resolve(repo_arg)?;
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
        None => repo::resolve(None).map(|i| i.nwo).unwrap_or_default(),
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
