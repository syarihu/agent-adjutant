use super::*;
use crate::lifecycle::worker::{
    Planned, SessionNote, WorkerRequest, exec_launch, plan_launch, register_launch,
};

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
        &settings.terminal,
        &SpawnRequest {
            cwd: &crate::infra::paths::expand_home(cwd).to_string_lossy(),
            title,
            command: &crate::infra::template::sh_join(command),
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

/// What `adj work` exits with when `maxWorkers` is reached. Its own code rather than the
/// usual 1, because the caller is usually a hub, and "full, try later" is the one refusal it
/// should answer by leaving the task queued instead of reporting a failure.
pub const WORKER_LIMIT_EXIT: i32 = 3;

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
    // The dispatching side: the identifier being handed to the new worker is this caller's
    // own, never one read out of some worktree it happens to be standing in.
    let ctx = context_as(repo_arg, hub_arg)?;
    let request = StartRequest {
        worktree: worktree.to_string(),
        title: title.to_string(),
        task: task_id.map(str::to_string),
        prompt: prompt.map(str::to_string),
        repo: repo_arg.map(str::to_string),
    };
    match crate::lifecycle::worker::start(&ctx, &request, dry_run)? {
        Started::Full(refusal) => {
            eprintln!("adjutant: {refusal}");
            Ok(WORKER_LIMIT_EXIT)
        }
        Started::Opened(done) => {
            print_performed(&done, dry_run);
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
            Ok(0)
        }
    }
}

pub fn focus(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    quiet: bool,
    dry_run: bool,
) -> Result<bool, String> {
    let ctx = context(repo_arg, hub_arg)?;
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

pub fn focus_worker_cmd(
    repo_arg: Option<&str>,
    worktree: &str,
    quiet: bool,
    dry_run: bool,
) -> Result<bool, String> {
    let settings = settings_for(repo_arg);
    let worktree = crate::infra::paths::expand_home(worktree);
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
pub fn phase(worktree: Option<&str>, set: Option<&str>) -> Result<(), String> {
    let worktree = worker_worktree(worktree)?;
    match set {
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

/// What `adj worker` was typed with.
pub struct WorkerArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub worktree: Option<&'a str>,
    pub title: Option<&'a str>,
    pub task: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub resume: bool,
    pub dry_run: bool,
}

pub fn worker(args: &WorkerArgs<'_>) -> Result<(), String> {
    let request = WorkerRequest {
        repo: args.repo.map(str::to_string),
        hub: args.hub.map(str::to_string),
        worktree: args.worktree.map(str::to_string),
        title: args.title.map(str::to_string),
        task: args.task.map(str::to_string),
        prompt: args.prompt.map(str::to_string),
        resume: args.resume,
    };
    let launch = match plan_launch(&request)? {
        Planned::Running { pid } => {
            println!(
                "a worker is already running in this worktree (pid {})",
                pid.unwrap_or(0)
            );
            return Ok(());
        }
        Planned::Launch(launch) => launch,
    };
    if args.dry_run {
        println!(
            "cd {}",
            crate::infra::template::sh_quote(&launch.worktree.to_string_lossy())
        );
        println!("{}", launch.command);
        return Ok(());
    }
    match register_launch(&launch)?.session {
        Some(SessionNote::NotSaved(e)) => {
            eprintln!("adjutant: {e}; --resume may not reopen this worker");
        }
        Some(SessionNote::StillUnderOldHub(e)) => {
            eprintln!("adjutant: {e}; the worker may still be counted for its old hub");
        }
        None => {}
    }
    Err(exec_launch(&launch))
}
