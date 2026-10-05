use super::*;

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

/// Start the worker agent in the tab `work` just opened.
///
/// The mirror image of `hub`: write down who we are, then become the agent. Running the
/// agent as a child instead would record a PID that exits the moment the agent does
/// anything, and waking a dead launcher wakes nobody.
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
    use std::os::unix::process::CommandExt;

    let WorkerArgs {
        repo: repo_arg,
        hub: hub_arg,
        worktree,
        title,
        task,
        prompt,
        resume,
        dry_run,
    } = *args;

    let worktree = worker_worktree(worktree)?;
    let resumed = match resume {
        true => Some(saved_worker_session(&worktree)?),
        false => None,
    };
    let task = task.or_else(|| resumed.as_ref().and_then(|saved| saved.task.as_deref()));
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
            context_of(identity::resolve(repo_arg, told.or(saved.hub.as_deref()))?)?
        }
        None => context_as(repo_arg, hub_arg)?,
    };
    let status = registry::worker_status(&worktree);
    if status.present {
        println!(
            "a worker is already running in this worktree (pid {})",
            status.pid.unwrap_or(0)
        );
        // `adj work` marked this worktree on the way here, and nobody is going to register
        // over it. Left, it would hold a second slot for the grace period after the running
        // worker ends.
        let _ = registry::unmark_worker_starting(&worktree);
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
                &agent_env(&ctx),
                &saved.session_id,
                prompt.unwrap_or(runner::WORKER_RESUME_PROMPT),
                &worktree_text,
                &title,
            );
            (command, None)
        }
        None => {
            let session = registry::new_session_id()?;
            let command = runner::worker_command(
                ctx.settings.agent_runner.as_deref(),
                &agent_env(&ctx),
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
        println!("cd {}", crate::infra::template::sh_quote(&worktree_text));
        println!("{command}");
        return Ok(());
    }

    std::env::set_current_dir(&worktree)
        .map_err(|e| format!("cannot change directory to {}: {e}", worktree.display()))?;
    // The address goes into the record here, at the last moment before this process stops
    // being a launcher. Everything the worker's agent later sends is addressed from it.
    let location = terminal::own_location(&ctx.settings.terminal);
    registry::register_worker(
        &worktree,
        &title,
        ctx.repo.hub.as_deref(),
        task,
        Some(&location),
    )?;
    // Said and got past, as for the hub: a worker that cannot be resumed still works. And as
    // for the hub, a fresh start with nothing to record clears what an earlier worker saved.
    if resumed.is_none() {
        let saved = match &fresh_session {
            Some(session) => registry::save_worker_session(
                &worktree,
                &title,
                ctx.repo.hub.as_deref(),
                task,
                session,
            )
            .map(|_| ()),
            None => registry::forget_worker_session(&worktree),
        };
        if let Err(e) = saved {
            eprintln!("adjutant: {e}; --resume may not reopen this worker");
        }
    } else if let Some(saved) = &resumed {
        // Reopened under another hub than the session remembers: say so there too, or the
        // worker would count for the old hub once it has ended and its record is gone.
        let slug_of = |hub: Option<&str>| identity::slug_for(&ctx.repo.nwo, hub);
        if slug_of(ctx.repo.hub.as_deref()) != slug_of(saved.hub.as_deref())
            && let Err(e) = registry::save_worker_session(
                &worktree,
                &title,
                ctx.repo.hub.as_deref(),
                task,
                &saved.session_id,
            )
        {
            eprintln!("adjutant: {e}; the worker may still be counted for its old hub");
        }
    }

    // A worker is not a hub. A tab opened by a spawn command that passes its environment on
    // would otherwise hand the hub's session to this agent's MCP server, which would then
    // keep saying the hub is alive for as long as the worker runs — and the hub's board
    // marker, which would have it try to serve the hub's board.
    //
    // Nor is it whichever hub opened the tab. `ADJUTANT_HUB` outranks the record written
    // above, so an inherited one would send every report to the hub that dispatched the
    // tab rather than the one this worker registered under; the line carries the right one.
    let error = agent_command(&command).exec();
    let _ = registry::unregister_worker(&worktree);
    Err(format!("cannot start the worker: {error}"))
}
