use super::*;

// ── spawn / focus / close / work / ide ───────────────────────────────

/// The command that names a new tab from inside it, if one should.
///
/// A tab names itself rather than being named by the terminal's API, so that whatever
/// `terminal.title` is set to governs every tab the same way. `terminal::spawn` decides
/// whether it can be used at all: only a terminal taking a shell line can run it.
pub(super) fn title_command(settings: &Settings, title: &str) -> Option<String> {
    if settings.terminal.title.is_off() || title.trim().is_empty() {
        return None;
    }
    // stdin closed: `adjutant title` reads a title of `-` from stdin, and in a new tab stdin is
    // the terminal — a task titled `-` would sit there waiting for input, and the worker after
    // it would never start.
    Some(format!(
        "{} < /dev/null",
        crate::infra::template::sh_join(&[
            exe_path(),
            "title".to_string(),
            // One word, `--title=…`: a title starting with `--` given as the next word is read
            // by clap as an option of its own, and `--help` would print help instead.
            format!("--title={title}"),
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

/// Open a tab and start a worker agent in it. One command rather than two so the runner
/// template is read in exactly one place.
/// The environment a new tab has to be handed on its command line, because a terminal is
/// given a command line and nothing else.
///
/// `ADJUTANT_HUB` already travels as a flag, and `ADJUTANT_STARTUP_DASHBOARD` as one too.
/// `ADJUTANT_CONFIG`, `XDG_CONFIG_HOME` (which picks the config when `ADJUTANT_CONFIG` is
/// unset) and `ADJUTANT_STATE_DIR` have none, and losing them does not fail — it *splits*:
/// the tab reads the default config and the default state directory, so the agent it starts
/// registers in one world while the hub that dispatched it waits in another. The worker reports into an
/// inbox nobody is reading, and both halves look healthy from where they stand.
///
/// Forwarded only when this process was given them. A machine that never sets them gets the
/// command line it always had. The state directory is sent as `root`, the absolute one this
/// command read, because the tab would otherwise read a relative one against its own worktree.
pub(super) fn forwarded_env(root: &std::path::Path) -> Vec<String> {
    let set: Vec<String> = [
        crate::infra::env::CONFIG_ENV,
        crate::infra::env::XDG_CONFIG_HOME_ENV,
        crate::infra::env::STATE_DIR_ENV,
        crate::infra::env::TMUX_SOCKET_ENV,
        crate::infra::env::TMUX_SESSION_ENV,
    ]
    .iter()
    .filter_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.is_empty())
            .map(|value| {
                if *name == crate::infra::env::STATE_DIR_ENV {
                    format!("{name}={}", root.display())
                } else {
                    format!("{name}={value}")
                }
            })
    })
    .collect();
    if set.is_empty() {
        return Vec::new();
    }
    let mut parts = vec!["env".to_string()];
    parts.extend(set);
    parts
}

/// Whether two paths name one directory, for a task's stored worktree against the one git lists.
pub(super) fn same_path(a: &str, b: &str) -> bool {
    let resolved = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| p.into());
    resolved(a) == resolved(b)
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
    let main = std::path::Path::new(&ctx.repo.main);
    // Always under the lock, limit or not: the check that nobody else is starting a worker
    // here and the mark that says so is taken are one step, or two requests at once both pass
    // the check, and a cleanup's last look at the slots means nothing.
    registry::with_dispatch_lock(main, || {
        if registry::is_being_removed(main, worktree) {
            return Err(format!(
                "{} is being removed; nothing was started",
                worktree.display()
            ));
        }
        // Not on a dry run, which marks nothing and so races with nothing.
        if !dry_run && registry::is_starting(worktree, crate::infra::clock::now_secs()) {
            return Err(format!(
                "a worker is already starting in {}; nothing was started",
                worktree.display()
            ));
        }
        if let Some(max) = ctx.settings.max_workers {
            // The main checkout too. It is where the hub sits and a worker is not meant to go,
            // but nothing stops `adj work` being pointed at it, and a worker running there
            // uncounted is one past the limit.
            let mut candidates = identity::linked_worktrees(&ctx.repo.main)?;
            candidates.push(ctx.repo.main.clone());
            let busy = registry::busy_worktrees(&candidates, Some(worktree));
            if busy.len() >= max as usize {
                return Ok(Some(format!(
                    "worker limit reached: {} of maxWorkers {max} are running ({}). \
                     Nothing was started; leave the task queued and dispatch it when one finishes",
                    busy.len(),
                    busy.join(", ")
                )));
            }
        }
        if !dry_run {
            registry::mark_worker_starting(worktree)?;
        }
        Ok(None)
    })?
}

/// Open the tab, and give the slot back if it never opened.
fn open_worker_tab(
    ctx: &Context,
    worktree: &str,
    request: &SpawnRequest<'_>,
    dry_run: bool,
) -> Result<terminal::Performed, String> {
    terminal::spawn(&ctx.settings.terminal, request, dry_run).inspect_err(|_| {
        let _ = registry::unmark_worker_starting(std::path::Path::new(worktree));
    })
}

/// `open_worker_tab`, said aloud: what the command line prints.
fn spawn_worker(
    ctx: &Context,
    worktree: &str,
    request: &SpawnRequest<'_>,
    dry_run: bool,
) -> Result<i32, String> {
    let done = open_worker_tab(ctx, worktree, request, dry_run)?;
    print_performed(&done, dry_run);
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
            from_record = crate::task::get(&ctx.state, &ctx.repo.slug, id)?.title;
            from_record.as_str()
        }
        _ => title,
    };
    let worktree = crate::infra::paths::expand_home(worktree)
        .to_string_lossy()
        .to_string();
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
    let mut parts = forwarded_env(&ctx.state);
    parts.extend([
        exe_path(),
        "worker".to_string(),
        "--worktree".to_string(),
        worktree.clone(),
        // `=` rather than a separate word, as in `title_command`: a title from an issue that
        // starts with `--` would otherwise be parsed as an option and the worker never start.
        format!("--title={title}"),
        format!("--prompt={prompt}"),
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
    if let Some(task) = task_id {
        parts.push(format!("--task={task}"));
    }
    let name_it = title_command(&ctx.settings, title);
    spawn_worker(
        &ctx,
        &worktree,
        &SpawnRequest {
            cwd: &worktree,
            title,
            command: &crate::infra::template::sh_join(&parts),
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
    match resume_worker(&ctx, repo_arg, hub_arg, worktree, title, prompt, dry_run)? {
        Resumed::Full(refusal) => {
            eprintln!("adjutant: {refusal}");
            Ok(WORKER_LIMIT_EXIT)
        }
        Resumed::Opened(done) => {
            print_performed(&done, dry_run);
            Ok(0)
        }
    }
}

/// What reopening a worker came to.
pub(super) enum Resumed {
    /// The tab was opened (or, on a dry run, would be).
    Opened(terminal::Performed),
    /// `maxWorkers` is reached; the refusal says so. Kept apart from `Err` because the command
    /// line gives it its own exit code.
    Full(String),
}

/// Reopen the worker session saved in `worktree` in a new tab, without saying anything: the
/// command line prints what came of it and the board puts it in a reply.
///
/// `ctx` is the caller's to build. It has to address no hub of its own (see `work_resumed`),
/// and this does not look at the process's directory: the resident server runs nowhere near
/// the repository.
pub(super) fn resume_worker(
    ctx: &Context,
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    worktree: &str,
    title: &str,
    prompt: Option<&str>,
    dry_run: bool,
) -> Result<Resumed, String> {
    let worktree = worker_worktree(Some(worktree))?;
    // Refused here rather than in the tab, so the caller — often a hub — hears about it.
    let saved = saved_worker_session(&worktree)?;
    resume_template(
        ctx.settings.agent_resume_runner.as_deref(),
        "agentResumeRunner",
    )?;
    // A reopened worker is as much a process as a fresh one.
    if let Some(refusal) = claim_worker_slot(ctx, &worktree, dry_run)? {
        return Ok(Resumed::Full(refusal));
    }
    let worktree = worktree.to_string_lossy().to_string();
    let title = match title {
        "" => saved.title.as_deref().unwrap_or(""),
        given => given,
    };
    let mut parts = forwarded_env(&ctx.state);
    parts.extend([
        exe_path(),
        "worker".to_string(),
        "--resume".to_string(),
        "--worktree".to_string(),
        worktree.clone(),
    ]);
    if !title.is_empty() {
        parts.push(format!("--title={title}"));
    }
    if let Some(prompt) = prompt {
        parts.push(format!("--prompt={prompt}"));
    }
    if let Some(repo) = repo_arg {
        parts.push("--repo".to_string());
        parts.push(repo.to_string());
    }
    if let Some(hub) = hub_arg.map(str::trim).filter(|hub| !hub.is_empty()) {
        parts.push(format!("--hub={hub}"));
    }
    let name_it = title_command(&ctx.settings, title);
    open_worker_tab(
        ctx,
        &worktree,
        &SpawnRequest {
            cwd: &worktree,
            title,
            command: &crate::infra::template::sh_join(&parts),
            title_command: name_it.as_deref(),
        },
        dry_run,
    )
    .map(Resumed::Opened)
}

pub fn focus(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    quiet: bool,
    dry_run: bool,
) -> Result<bool, String> {
    let ctx = context(repo_arg, hub_arg)?;
    let status = registry::hub_status(&ctx.state, &ctx.repo.slug, &ctx.repo.hub_name);
    let Some(pid) = status.pid.filter(|_| status.present) else {
        if !quiet {
            println!("{} is not running", ctx.repo.hub_name);
        }
        return Ok(false);
    };
    let done = terminal::focus(&ctx.settings.terminal, pid, &ctx.repo.hub_name, dry_run)?;
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

/// Bring the tab of the worker in `worktree` to the front, through `terminal.focus` like the
/// hub's. `Ok(false)` when no worker is running there — nothing to raise.
pub fn focus_worker(
    settings: &Settings,
    worktree: &std::path::Path,
    dry_run: bool,
) -> Result<Option<terminal::Performed>, String> {
    let status = registry::worker_status(worktree);
    let Some(pid) = status.pid.filter(|_| status.present) else {
        return Ok(None);
    };
    // The name the tab actually carries, as `close` hands it over: `spawn` put the record's
    // title through `sanitise_title`, and a template that finds a tab by name needs that.
    let title = terminal::sanitise_title(
        status.title.as_deref().unwrap_or_default(),
        worktree
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
    );
    terminal::focus(&settings.terminal, pid, &title, dry_run).map(Some)
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
        // Resumed under another hub than the session remembers: say so there too, or the
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

/// The shell the agent is started through, by `hub` and `worker` alike.
///
/// The agent gets the caller's environment, minus what would make it answer for something it
/// is not. The adjutant variables are set on the command line itself when they apply, so an
/// inherited one could only name another hub. The git variables in
/// `crate::infra::git::REPOSITORY_LOCATION_ENV` would point every git command the agent runs at another
/// repository, while adjutant — which ignores them — works on the one it was started in.
pub(super) fn agent_command(command: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .env_remove(crate::infra::env::HUB_SESSION_ENV)
        .env_remove(crate::infra::env::HUB_SERVE_ENV)
        .env_remove(crate::infra::env::HUB_ENV);
    for name in crate::infra::git::REPOSITORY_LOCATION_ENV {
        cmd.env_remove(name);
    }
    cmd
}

/// The worktree a worker runs in: the one named, or — for `--resume`, typed by a person
/// standing in it — the one this command was run from.
fn worker_worktree(worktree: Option<&str>) -> Result<std::path::PathBuf, String> {
    let worktree = match worktree {
        Some(path) => crate::infra::paths::expand_home(path),
        None => identity::current_worktree(None)
            .map(std::path::PathBuf::from)
            .ok_or("not inside a git worktree; pass --worktree")?,
    };
    if !worktree.is_dir() {
        return Err(format!("no such worktree: {}", worktree.display()));
    }
    Ok(worktree)
}

/// The session `--resume` reopens in `worktree`, or a refusal that says why there is none.
pub(super) fn saved_worker_session(
    worktree: &std::path::Path,
) -> Result<registry::SavedSession, String> {
    registry::worker_session(worktree).ok_or_else(|| {
        format!(
            "no saved worker session in {}: a session is saved when a worker is started by \
             `adj work` with a runner that takes {{sessionId}} (the built-in one does)",
            worktree.display()
        )
    })
}
