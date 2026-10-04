use super::*;

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
        let _ = terminal::focus(&ctx.settings.terminal, pid, &ctx.repo.hub_name, dry_run);
    }
    Ok(())
}

/// The environment the hub's agent is started with.
///
/// `agent_env`, and then whether this invocation was asked to collect the dashboard at
/// startup — the one other thing the agent has no way to learn. Appended so that ours is the
/// later assignment on the `env` line and therefore the one that takes: a config naming the
/// variable is describing a default, not overruling the flag that was just typed.
///
/// Neither the hub nor the dashboard is added when it was not asked for, and that is
/// deliberate rather than tidy: the command line a plain `adj hub` prints has to stay exactly
/// what it printed before, or every existing dry run, doc and expectation of it is wrong.
fn hub_env(ctx: &Context, dashboard: Option<bool>) -> Vec<(String, String)> {
    let mut env = agent_env(ctx);
    // Appended for the reason above, and absent when no flag was typed for the reason above
    // that: `--dashboard` and `--no-dashboard` are this invocation overruling the standing
    // `startupDashboard`, and a variable set unconditionally would make every hub's command
    // line carry an override nobody asked for.
    if let Some(on) = dashboard {
        env.push((
            crate::infra::env::STARTUP_DASHBOARD_ENV.to_string(),
            if on { "1" } else { "0" }.to_string(),
        ));
    }
    env
}

/// What starting a hub in a tab came to.
pub enum TabOutcome {
    /// A hub is up under this name already, so no tab was opened.
    AlreadyRunning(messaging::HubStatus),
    Opened(terminal::Performed),
}

/// The saved session `--resume` names, or a refusal — and, for the template it will run,
/// the same. Looked up before either route, so that asking for a session that is not there is
/// refused here, in the tab it was typed in — not in a tab opened to show the refusal. The
/// template is checked here too, for the same reason: on the tab route the refusal would
/// otherwise come from inside a tab this one had already reported as opened.
fn asked_session(
    ctx: &Context,
    start: HubStart,
) -> Result<Option<messaging::SavedSession>, String> {
    match start {
        HubStart::Resume => {
            let saved = saved_hub_session(ctx)?;
            resume_template(ctx.settings.hub_resume_runner.as_deref(), "hubResumeRunner")?;
            Ok(Some(saved))
        }
        HubStart::Auto | HubStart::New => Ok(None),
    }
}

/// Everything a restart of the hub `ctx` addresses has to know can be resumed, checked before
/// the hub is stopped: a restart that stops the hub and only then finds there is nothing to
/// come back to has cost the person the session it was meant to keep.
///
/// Beyond what `--resume` itself refuses, a `hubRunner` of the person's own with no
/// `hubResumeRunner` is refused too. Typed out, `--resume` is the person's call to have the
/// built-in runner reopen it; a button that stops a running hub is not asking anyone.
pub(super) fn hub_resume_check(ctx: &Context) -> Result<messaging::SavedSession, String> {
    let saved = saved_hub_session(ctx)?;
    resume_template(ctx.settings.hub_resume_runner.as_deref(), "hubResumeRunner")?;
    if let Some(refusal) = own_hub_runner_refusal(&ctx.settings) {
        return Err(refusal);
    }
    Ok(saved)
}

/// Why a hub started by a runner of the person's own cannot be reopened by the built-in one:
/// `hubRunner` is set and `hubResumeRunner` is not. `None` when that is not the case.
pub(super) fn own_hub_runner_refusal(settings: &config::Settings) -> Option<String> {
    (settings.hub_runner.is_some() && settings.hub_resume_runner.is_none()).then(|| {
        "hubRunner is your own and hubResumeRunner is not set, so the built-in runner \
         would reopen the session instead of yours"
            .to_string()
    })
}

pub(super) fn print_performed(done: &terminal::Performed, dry_run: bool) {
    if dry_run {
        println!("{}", done.script);
    } else {
        println!("{}", done.description);
    }
}

/// Start the hub `ctx` addresses in a new tab under `terminal`, unless it is already up.
///
/// The route `adj hub --tab` takes, and the one the board's start button takes: neither
/// claims anything here, since the claim belongs to the `adj hub` the tab runs (see
/// `open_hub_tab`), so everything that can be refused is refused before a tab is opened.
///
/// `present: false` answers two different questions the same way: nobody is there, and
/// whether anybody is there could not be established — an unreadable record, or a `ps` that
/// would not run. Only the first is a reason to start a hub, and the other route never has to
/// tell them apart because its claim refuses the second in exactly these words. This one
/// leaves the claim to the tab it opens, so the refusal happens here or nowhere — and nowhere
/// means the caller this route exists for, which is not a person, is told a hub was started
/// in a new tab and handed `exit 0`, for a hub whose own claim is about to refuse it.
///
/// `Alive` is a hub running under a name the presence check no longer matches on. The tab's
/// own claim would bring it forward, so the hub ends up in the same place either way — but
/// this side would have said it started one and exited 0 for a hub that was already up, which
/// is the same untruth told to the same non-human caller.
pub fn hub_in_tab(
    ctx: &Context,
    repo_arg: Option<&str>,
    extra: &[String],
    start: HubStart,
    dashboard: Option<bool>,
    terminal: &crate::infra::terminal::TerminalSettings,
    dry_run: bool,
) -> Result<TabOutcome, String> {
    let status = messaging::hub_status(&ctx.state, &ctx.repo.slug, &ctx.repo.hub_name);
    if status.present {
        return Ok(TabOutcome::AlreadyRunning(status));
    }
    asked_session(ctx, start)?;
    match messaging::hub_liveness(&ctx.state, &ctx.repo.slug) {
        messaging::Liveness::CannotTell => {
            Err(messaging::hub_cannot_tell(&ctx.state, &ctx.repo.slug))
        }
        messaging::Liveness::Alive => Ok(TabOutcome::AlreadyRunning(status)),
        messaging::Liveness::Gone => {
            open_hub_tab(ctx, repo_arg, extra, start, dashboard, terminal, dry_run)
                .map(TabOutcome::Opened)
        }
    }
}

/// Start a hub from the board: a new tmux window running `adj hub`, exactly as `adj hub --tab`
/// would open it. Refused unless that is what the settings mean by a tab — a custom `spawn`
/// template runs whatever the person wrote, and the built-in terminal is not one this process
/// can be sure to reach from a server that has no terminal of its own.
pub fn start_hub(ctx: &Context, start: HubStart) -> Result<TabOutcome, String> {
    if !hub_startable(&ctx.settings.terminal) {
        return Err("starting a hub from the board needs terminal.preset \"tmux\"".to_string());
    }
    hub_in_tab(ctx, None, &[], start, None, &ctx.settings.terminal, false)
}

/// Whether the board may start a hub under these settings.
pub fn hub_startable(terminal: &crate::infra::terminal::TerminalSettings) -> bool {
    crate::infra::terminal::backend_name(terminal) == "tmux"
}

/// Stop the hub `ctx` addresses by closing the tmux pane it runs in. `Ok(true)` when a hub
/// was running and is gone, `Ok(false)` when there was none (a record it left is cleared).
///
/// Everything is about **one** hub, read out of the record once: the pane that is closed,
/// the process that has to be gone afterwards, and the record that may then be cleared. Asked
/// separately, each could be answered about a different hub — one that started in the
/// meantime — and the record cleared would be its.
pub fn stop_hub(ctx: &Context) -> Result<bool, String> {
    let slug = &ctx.repo.slug;
    let root = &ctx.state;
    let read = messaging::read_hub_record(root, slug);
    let record = match &read {
        messaging::Recorded::Found(r) => Some(r),
        _ => None,
    };
    // The start time as recorded, not `recorded_anchor`: a blank one is refused below by
    // name rather than read as no anchor.
    let named = record.and_then(|r| Some((r.pid? as u32, r.ps_started.clone())));
    let Some((pid, started)) = named else {
        // No record, or none that names a process: no hub, and nothing that is safe to clear.
        return match messaging::hub_liveness(root, slug) {
            messaging::Liveness::CannotTell => Err(messaging::hub_cannot_tell(root, slug)),
            _ => Ok(false),
        };
    };
    // Before anything is looked up or closed: the start time is what tells this hub from
    // whatever inherited its pid, and closing a pane on the strength of the pid alone could
    // close somebody else's.
    if started.as_deref().is_none_or(|s| s.trim().is_empty()) {
        return Err("the hub record carries no start time, so the process cannot be told apart from a reused pid; stop it where it runs".to_string());
    }
    match messaging::hub_process_liveness(pid, started.as_deref()) {
        messaging::Liveness::Gone => {
            messaging::unregister_hub_if(root, slug, pid, started.as_deref())?;
            return Ok(false);
        }
        messaging::Liveness::CannotTell => return Err(messaging::hub_cannot_tell(root, slug)),
        messaging::Liveness::Alive => {}
    }
    let terminal_recorded = record.and_then(|r| r.terminal.clone());
    // Only a hub known to sit in tmux is looked for there: asking tmux about a hub that runs
    // anywhere else would start by talking to whichever server is the default.
    let in_tmux = match &terminal_recorded {
        Some(t) => t.backend == "tmux",
        None => hub_startable(&ctx.settings.terminal),
    };
    let socket = terminal_recorded
        .as_ref()
        .and_then(|t| t.socket.clone())
        .or_else(|| ctx.settings.terminal.tmux_socket().map(str::to_string));
    if !in_tmux {
        return Err(format!(
            "the hub is not in a tmux pane on socket {}; stop it where it runs",
            socket.as_deref().unwrap_or("default")
        ));
    }
    let panes = terminal::list_tmux_panes(socket.as_deref())?;
    let pane = terminal::find_matching_pane(&panes, Some(pid), terminal::tty_of(pid).as_deref())
        .ok_or_else(|| {
            format!(
                "the hub is not in a tmux pane on socket {}; stop it where it runs",
                socket.as_deref().unwrap_or("default")
            )
        })?;
    terminal::run_shell(&terminal::tmux_kill_pane_script(
        socket.as_deref(),
        &pane.pane_id,
    ))?;
    match settled(
        || messaging::hub_process_liveness(pid, started.as_deref()),
        std::thread::sleep,
        GONE_BUDGET,
        GONE_POLL,
    ) {
        messaging::Liveness::Gone => {
            messaging::unregister_hub_if(root, slug, pid, started.as_deref())?;
            Ok(true)
        }
        _ => Err(format!(
            "closed the pane, but the hub (pid {pid}) is still running"
        )),
    }
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
    terminal: &crate::infra::terminal::TerminalSettings,
    dry_run: bool,
) -> Result<terminal::Performed, String> {
    let mut parts = forwarded_env(&ctx.state);
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
    terminal::spawn(
        terminal,
        &SpawnRequest {
            // The main checkout, never a worktree: a hub that cannot cut worktrees is not a
            // hub, and this is the one thing `hub` moves to before it starts.
            cwd: &ctx.repo.main,
            title: &ctx.repo.hub_name,
            command: &crate::infra::template::sh_join(&parts),
            title_command: name_it.as_deref(),
        },
        dry_run,
    )
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
    // The directory move comes first: the hub runs in the main checkout.
    // `ctx.state` is taken against the main checkout, so the hub's own records are the
    // same wherever this was typed. Nothing has been written at this point, so a failure here
    // has nothing to undo.
    std::env::set_current_dir(&ctx.repo.main)
        .map_err(|e| format!("cannot change directory to {}: {e}", ctx.repo.main))?;

    // Asked before the command is even built, so the common "it is already up" case costs
    // nothing. It is not what *enforces* one hub per repository — the claim below is.
    let status = messaging::hub_status(&ctx.state, &ctx.repo.slug, &ctx.repo.hub_name);
    if status.present {
        return go_to_running_hub(&ctx, &status, dry_run);
    }
    // Below the presence check, and deliberately: one hub per address is the invariant, and
    // opening a tab for one that is already up would break it in the one way nothing later
    // repairs — two sessions answering to the same name, with the record naming one of them.
    if tab {
        return match hub_in_tab(
            &ctx,
            repo_arg,
            extra,
            start,
            dashboard,
            &ctx.settings.terminal,
            dry_run,
        )? {
            TabOutcome::AlreadyRunning(status) => go_to_running_hub(&ctx, &status, dry_run),
            TabOutcome::Opened(done) => {
                print_performed(&done, dry_run);
                Ok(())
            }
        };
    }
    let asked = asked_session(&ctx, start)?;
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
            crate::infra::env::HUB_SESSION_ENV.to_string(),
            messaging::hub_session_env(&ctx.repo.slug, &session),
        ));
    }
    // The board this hub is served by, for the same MCP server: it lives exactly as long as
    // the session, so the board stops when the hub does and nothing has to watch for that.
    // `adj hub` itself cannot, since it `exec`s the agent and is gone.
    if ctx.settings.hub_serve {
        env.push((
            crate::infra::env::HUB_SERVE_ENV.to_string(),
            ctx.repo.slug.clone(),
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
        command = format!("{command} {}", crate::infra::template::sh_join(extra));
    }
    if dry_run {
        println!("cd {}", crate::infra::template::sh_quote(&ctx.repo.main));
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
    match messaging::claim_hub(
        &ctx.state,
        &ctx.repo.slug,
        &ctx.repo.hub_name,
        &ctx.repo.main,
        named,
        ctx.repo.hub.as_deref(),
        Some(&terminal::own_location(&ctx.settings.terminal)),
    )? {
        messaging::Claim::Ours => {}
        messaging::Claim::Taken(status) => return go_to_running_hub(&ctx, &status, dry_run),
    }
    // Where this repository is, for a resident server that may serve its board without
    // being told anything else. Only the address: nothing here needs the server to be up.
    serve::note_board(&ctx.state, &ctx.repo);
    // A hub that cannot be resumed later is still a hub, so failing to write this down is
    // said and then got past — refusing to start over it would trade a working hub for a
    // convenience.
    //
    // A fresh hub whose runner records no session replaces what was saved with nothing, so
    // that nothing can later reopen the conversation of the hub before it.
    if resumed.is_none() {
        let saved = match records {
            true => messaging::save_hub_session(
                &ctx.state,
                &ctx.repo.slug,
                &ctx.repo.nwo,
                ctx.repo.hub.as_deref(),
                &ctx.repo.hub_name,
                &session,
            )
            .map(|_| ()),
            false => messaging::forget_hub_session(&ctx.state, &ctx.repo.slug),
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
    let error = agent_command(&command).exec();
    // Only reachable if exec failed — otherwise this process no longer exists.
    let _ = messaging::unregister_hub(&ctx.state, &ctx.repo.slug);
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
    let saved = messaging::hub_session(&ctx.state, &ctx.repo.slug)?;
    let last = messaging::hub_last_alive(&ctx.state, &ctx.repo.slug, &saved.session_id)?;
    let age = crate::infra::clock::now_secs().saturating_sub(last).max(0);
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
    if own_hub_runner_refusal(&ctx.settings).is_some() {
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
pub(super) fn ago(secs: i64) -> String {
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
    if let Some(saved) = messaging::hub_session(&ctx.state, &ctx.repo.slug) {
        return Ok(saved);
    }
    let mut message = format!("{} has no saved session to resume.", ctx.repo.hub_name);
    let others = messaging::hub_sessions_for(&ctx.state, &ctx.repo.nwo);
    if others.is_empty() {
        message.push_str(&format!(" No hub of {} has one.", ctx.repo.nwo));
    } else {
        message.push_str(" These can be resumed:");
        for other in others {
            let command = match &other.hub {
                Some(hub) => format!(
                    "adj hub --resume --hub {}",
                    crate::infra::template::sh_quote(hub)
                ),
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
pub(super) fn resume_template<'a>(
    configured: Option<&'a str>,
    key: &str,
) -> Result<Option<&'a str>, String> {
    match configured {
        Some(template) if !runner::records_session(Some(template), "") => Err(format!(
            "{key} has no {{sessionId}}, so it cannot be told which session to reopen"
        )),
        other => Ok(other),
    }
}

/// Remove this repo's hub record. For a hub shutting down cleanly, and for clearing a record
/// left behind by one that did not.
pub fn hub_stop(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<(), String> {
    let info = resolve(repo_arg, hub_arg)?;
    messaging::unregister_hub(
        &crate::registry::state_root(Some(std::path::Path::new(&info.main))),
        &info.slug,
    )?;
    println!("unregistered {}", info.hub_name);
    Ok(())
}

/// Why `hub` cannot be closed, or `Ok` when it can. Only a parent-task hub none of whose
/// checkouts report to it any more is closable: the repository hub is always there, and a
/// hub with workers is still in use. Unread messages, open tasks and gates do not stop it —
/// they are kept, and starting the same key again finds them.
pub(super) fn closable_check(repo: &RepoInfo, hub: &crate::session::RepoHub) -> Result<(), String> {
    if !hub.parent {
        return Err("the repository hub can only be stopped, not closed; use hub-stop".to_string());
    }
    // The list is lenient about checkouts it cannot read; closing must not be.
    identity::linked_worktrees(&repo.main)
        .map_err(|e| format!("cannot tell which checkouts report to {}: {e}", hub.name))?;
    if hub.children > 0 {
        return Err(format!(
            "{} checkout(s) still report to {}; use hub-stop to stop it, or clean them up first",
            hub.children, hub.name
        ));
    }
    Ok(())
}

/// Close a parent-task hub whose workers are all gone: clear its record and take it off the
/// board's address book, so it drops out of the list. Like `hub-stop` it ends no process: it
/// is for the hub itself, or a hub that is no longer running, and refuses a running one.
/// Its saved session, tasks, gates and inbox stay, and starting the same key with `--resume` picks them up again.
pub fn hub_close(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<(), String> {
    let info = resolve(repo_arg, hub_arg)?;
    let root = crate::registry::state_root(Some(std::path::Path::new(&info.main)));
    let hub = messaging::all_repo_hubs(&root, &info)
        .into_iter()
        .find(|h| h.slug == info.slug)
        .unwrap_or_else(|| crate::session::RepoHub {
            id: format!("hub-{}", info.slug),
            parent: info.hub.is_some(),
            key: info.hub.clone(),
            name: info.hub_name.clone(),
            title: None,
            slug: info.slug.clone(),
            state: crate::session::RepoHubState {
                present: false,
                stale: false,
                pid: None,
                started_at: None,
            },
            inbox_count: 0,
            inbox: Vec::new(),
            children: 0,
        });
    closable_check(&info, &hub)?;
    // This ends no process, so a hub that is still running would be left running with no
    // record, and the next `adj hub` would start a second one beside it. Only the hub itself
    // may clear its own record; from anywhere else it has to be stopped first.
    let recorded = messaging::read_hub_record(&root, &hub.slug);
    let named = match &recorded {
        messaging::Recorded::Found(r) => r.pid.map(|pid| (pid as u32, r.ps_started.clone())),
        _ => None,
    };
    if let Some((pid, started)) = &named {
        let pid = *pid;
        match messaging::hub_process_liveness(pid, started.as_deref()) {
            messaging::Liveness::Gone => {}
            messaging::Liveness::CannotTell => {
                return Err(messaging::hub_cannot_tell(&root, &hub.slug));
            }
            messaging::Liveness::Alive => {
                if !messaging::is_self_or_descendant_of(pid) {
                    return Err(format!(
                        "{} is still running (pid {pid}); close it from the board, or stop it first \
                         (`adj hub-stop` from inside it, or the board's stop) and then close it",
                        hub.name
                    ));
                }
            }
        }
    } else if matches!(recorded, messaging::Recorded::Unreadable) {
        // A record that is there and cannot be read is asked the way `stop_hub` asks, and
        // nobody can be told to be the hub itself. A readable one that names no process is
        // `Gone` to `hub_liveness`, so it goes straight on to be cleared.
        match messaging::hub_liveness(&root, &hub.slug) {
            messaging::Liveness::Gone => {}
            messaging::Liveness::CannotTell => {
                return Err(messaging::hub_cannot_tell(&root, &hub.slug));
            }
            messaging::Liveness::Alive => {
                return Err(format!(
                    "{} is still running; close it from the board, or stop it first \
                     (`adj hub-stop` from inside it, or the board's stop) and then close it",
                    hub.name
                ));
            }
        }
    }
    // Under the claim lock and only while the record is still the one that was looked at:
    // a hub that registered since is not this call's to unregister.
    let removed = match &named {
        Some((pid, started)) => {
            messaging::unregister_hub_if(&root, &hub.slug, *pid, started.as_deref())?
        }
        None => messaging::unregister_hub_if_unnamed(&root, &hub.slug)?,
    };
    if !removed {
        return Err(format!("{} changed while it was being closed", hub.name));
    }
    serve::forget_board(&root, &hub.slug)?;
    println!("closed {}", hub.name);
    if hub.inbox_count > 0 {
        println!(
            "{} unread message(s) remain for it; starting the same key shows them",
            hub.inbox_count
        );
    }
    Ok(())
}
