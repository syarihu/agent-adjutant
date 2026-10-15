use super::args::{FocusArgs, PhaseArgs, SpawnArgs, WorkArgs, WorkerArgs};
use super::*;
use crate::lifecycle::worker::{
    Planned, SessionNote, StartRequest, Started, WorkerRequest, exec_launch, focus_worker,
    plan_launch, register_launch, resume_worker, worker_worktree,
};
use crate::lifecycle::{Hooks, title_command};

pub fn spawn(args: &SpawnArgs) -> Result<(), String> {
    let command = args.command();
    if command.is_empty() {
        return Err("pass the command to run after --".to_string());
    }
    let settings = settings_for(args.repo.as_deref());
    let name_it = title_command(&settings, &args.title);
    let done = terminal::spawn(
        &settings.terminal,
        &SpawnRequest {
            cwd: &crate::infra::paths::expand_home(&args.cwd).to_string_lossy(),
            title: &args.title,
            command: &crate::infra::template::sh_join(&command),
            title_command: name_it.as_deref(),
        },
        args.dry_run,
    )?;
    if args.dry_run {
        println!("{}", done.script);
    } else {
        println!("{}", done.description);
    }
    Ok(())
}

/// What `adj work` exits with when `maxWorkers` is reached. Its own code rather than the
/// usual 1, because the caller is usually a hub, and "full, try later" is the one refusal it
/// should answer by leaving the task queued instead of reporting a failure.
pub const WORKER_LIMIT_EXIT: i32 = 3;

/// Open a tab and start a worker on a worktree. `--task` names the record the title is taken
/// from when `--title` is empty.
pub fn work(args: &WorkArgs) -> Result<i32, String> {
    let repo_arg = args.repo.as_deref();
    let hub_arg = args.hub.as_deref();
    if args.resume {
        return work_resumed(
            repo_arg,
            hub_arg,
            &args.worktree,
            &args.title,
            args.prompt.as_deref(),
            args.dry_run,
        );
    }
    // The dispatching side: the identifier being handed to the new worker is this caller's
    // own, never one read out of some worktree it happens to be standing in.
    let ctx = context_as(repo_arg, hub_arg)?;
    let request = StartRequest {
        worktree: args.worktree.clone(),
        title: args.title.clone(),
        task: args.task.clone(),
        prompt: args.prompt.clone(),
        repo: args.repo.clone(),
    };
    match crate::lifecycle::worker::start(&ctx, &request, args.dry_run)? {
        Started::Full(refusal) => {
            eprintln!("adjutant: {refusal}");
            Ok(WORKER_LIMIT_EXIT)
        }
        Started::Opened(done) => {
            print_performed(&done, args.dry_run);
            say_unwatched(&ctx, args.dry_run);
            Ok(0)
        }
    }
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
    match resume_worker(&ctx, repo_arg, hub_arg, worktree, title, prompt, dry_run)? {
        Started::Full(refusal) => {
            eprintln!("adjutant: {refusal}");
            Ok(WORKER_LIMIT_EXIT)
        }
        Started::Opened(done) => {
            print_performed(&done, dry_run);
            say_unwatched(&ctx, dry_run);
            Ok(0)
        }
    }
}

/// A terminal that cannot be watched announces a start it has no way to check, so say so.
fn say_unwatched(ctx: &Context, dry_run: bool) {
    if !dry_run && !terminal::watches_start(&ctx.settings.terminal) {
        println!(
            "(this terminal gives no way to check that the worker stayed up; it appears on the board once it registers)"
        );
    }
}

/// `adj focus`: bring the hub, or with `--worktree` the worker in that worktree, to the front.
pub fn focus(args: &FocusArgs) -> Result<bool, String> {
    match args.worktree.as_deref() {
        Some(worktree) => focus_worker_cmd(args, worktree),
        None => focus_hub(args),
    }
}

fn focus_hub(args: &FocusArgs) -> Result<bool, String> {
    let (quiet, dry_run) = (args.quiet, args.dry_run);
    let ctx = context(args.repo.as_deref(), args.hub.as_deref())?;
    let Some(raised) = crate::lifecycle::hub::focus(&ctx, dry_run)? else {
        if !quiet {
            println!("{} is not running", ctx.repo.hub_name);
        }
        return Ok(false);
    };
    if dry_run {
        println!("{}", raised.done.script);
    } else if !quiet {
        println!(
            "{} is already running (pid {})",
            ctx.repo.hub_name, raised.pid
        );
        if !raised.done.ran {
            println!("({})", raised.done.description);
        }
    }
    Ok(true)
}

fn focus_worker_cmd(args: &FocusArgs, worktree: &str) -> Result<bool, String> {
    let (quiet, dry_run) = (args.quiet, args.dry_run);
    let settings = settings_for(args.repo.as_deref());
    let worktree = crate::infra::paths::absolute(worktree);
    let Some(done) = focus_worker(&settings, &worktree, dry_run)? else {
        if !quiet {
            println!("no worker is running in {}", worktree.display());
        }
        return Ok(false);
    };
    if dry_run && !done.script.is_empty() {
        println!("{}", done.script);
    } else if !quiet && (dry_run || !done.ran) {
        // Nothing to show as a script means nothing would run: say why rather than print a
        // blank line.
        println!("({})", done.description);
    }
    Ok(true)
}

/// `adj phase`: set the step the worker here is in, or say which it is.
pub fn phase(args: &PhaseArgs) -> Result<(), String> {
    let worktree = worker_worktree(args.worktree.as_deref())?;
    match args.set.as_deref() {
        Some(phase) => {
            registry::set_worker_phase(&worktree, phase)?;
            println!("phase: {phase}");
        }
        None => match registry::worker_status(&worktree).phase {
            Some(phase) => println!("{phase}"),
            None => println!("(none)"),
        },
    }
    Ok(())
}

pub fn worker(args: &WorkerArgs) -> Result<(), String> {
    let request = WorkerRequest {
        repo: args.repo.clone(),
        hub: args.hub.clone(),
        worktree: args.worktree.clone(),
        title: args.title.clone(),
        task: args.task.clone(),
        prompt: args.prompt.clone(),
        resume: args.resume,
    };
    // Resolved here as well, so a failure below can give the slot back. A worktree that cannot
    // be resolved is a refusal before anything was marked.
    let worktree = worker_worktree(args.worktree.as_deref())?;
    // Harmless once registered, since registering already gave it back. It is for the
    // terminals that cannot be watched (iTerm2, a template), where nobody else notices a
    // worker that never started and its slot would be held for the rest of the grace period.
    // Not on a dry run, which `adj work` did not mark for and which must not clear the mark of
    // one in flight.
    let give_back = |e: String| {
        if !args.dry_run {
            let _ = registry::unmark_worker_starting(&worktree);
        }
        e
    };
    let launch = match plan_launch(&request).map_err(give_back)? {
        Planned::Running { pid } => {
            println!(
                "a worker is already running in this worktree (pid {})",
                pid.unwrap_or(0)
            );
            return Ok(());
        }
        Planned::Launch(launch) => launch,
    };
    if let Hooks::Skipped(why) = &launch.hooks {
        eprintln!("adjutant: not passing hooks to the agent: {why}");
    }
    if args.dry_run {
        println!(
            "cd {}",
            crate::infra::template::sh_quote(&launch.worktree.to_string_lossy())
        );
        println!("{}", launch.command);
        return Ok(());
    }
    match register_launch(&launch).map_err(give_back)?.session {
        Some(SessionNote::NotSaved(e)) => {
            eprintln!("adjutant: {e}; --resume may not reopen this worker");
        }
        Some(SessionNote::StillUnderOldHub(e)) => {
            eprintln!("adjutant: {e}; the worker may still be counted for its old hub");
        }
        None => {}
    }
    Err(give_back(exec_launch(&launch)))
}
