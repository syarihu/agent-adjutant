use super::*;

// ── hub (the launcher) ─────────────────────────────────────────

/// Start this repository's hub, here, once.
///
/// Say that the hub is already up, and bring it forward. The answer to both "someone got
/// here first" and "it was already running when we looked".
fn go_to_running_hub(
    ctx: &Context,
    status: &registry::HubStatus,
    dry_run: bool,
) -> Result<(), String> {
    println!(
        "{} is already running (pid {})",
        ctx.repo.hub_name,
        status.pid.unwrap_or(0)
    );
    // Raising is a courtesy: the hub is up either way, and a tab that cannot be raised is not
    // this command failing.
    let _ = crate::lifecycle::hub::focus(ctx, dry_run);
    Ok(())
}

pub(super) fn print_performed(done: &terminal::Performed, dry_run: bool) {
    if dry_run {
        println!("{}", done.script);
    } else {
        println!("{}", done.description);
    }
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
    let status = registry::hub_status(&ctx.state, &ctx.repo.slug, &ctx.repo.hub_name);
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
        None => registry::new_session_id()?,
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
            registry::hub_session_env(&ctx.repo.slug, &session),
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
    match registry::claim_hub(
        &ctx.state,
        &ctx.repo.slug,
        &ctx.repo.hub_name,
        &ctx.repo.main,
        named,
        ctx.repo.hub.as_deref(),
        Some(&terminal::own_location(&ctx.settings.terminal)),
    )? {
        registry::Claim::Ours => {}
        registry::Claim::Taken(status) => return go_to_running_hub(&ctx, &status, dry_run),
    }
    // Where this repository is, for a resident server that may serve its board without
    // being told anything else. Only the address: nothing here needs the server to be up.
    crate::registry::note_board(&ctx.state, &ctx.repo);
    // A hub that cannot be resumed later is still a hub, so failing to write this down is
    // said and then got past — refusing to start over it would trade a working hub for a
    // convenience.
    //
    // A fresh hub whose runner records no session replaces what was saved with nothing, so
    // that nothing can later reopen the conversation of the hub before it.
    if resumed.is_none() {
        let saved = match records {
            true => registry::save_hub_session(
                &ctx.state,
                &ctx.repo.slug,
                &ctx.repo.nwo,
                ctx.repo.hub.as_deref(),
                &ctx.repo.hub_name,
                &session,
            )
            .map(|_| ()),
            false => registry::forget_hub_session(&ctx.state, &ctx.repo.slug),
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
    let _ = registry::unregister_hub(&ctx.state, &ctx.repo.slug);
    Err(format!("cannot start the hub: {error}"))
}

/// The saved session a plain `adj hub` comes back to, if it ended recently enough.
///
/// "Ended" is the last time the hub's MCP server said the session was alive — it beats
/// every minute and once more as the agent closes it. Where nothing has said so (no MCP
/// server, a runner that is not the agent this knows, a session saved by an older version)
/// there is no answer, and no answer starts fresh: coming back uninvited to a conversation of
/// unknown age is worse than one clean start too many.
fn recent_hub_session(ctx: &Context) -> Option<registry::SavedSession> {
    let window = ctx.settings.hub_auto_resume_hours;
    if window <= 0.0 {
        return None;
    }
    let saved = registry::hub_session(&ctx.state, &ctx.repo.slug)?;
    let last = registry::hub_last_alive(&ctx.state, &ctx.repo.slug, &saved.session_id)?;
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

/// Remove this repo's hub record. For a hub shutting down cleanly, and for clearing a record
/// left behind by one that did not.
pub fn hub_stop(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<(), String> {
    let info = resolve(repo_arg, hub_arg)?;
    registry::unregister_hub(
        &crate::registry::state_root(Some(std::path::Path::new(&info.main))),
        &info.slug,
    )?;
    println!("unregistered {}", info.hub_name);
    Ok(())
}

/// Close a parent-task hub whose workers are all gone: clear its record and take it off the
/// board's address book, so it drops out of the list. Like `hub-stop` it ends no process: it
/// is for the hub itself, or a hub that is no longer running, and refuses a running one.
/// Its saved session, tasks, gates and inbox stay, and starting the same key with `--resume` picks them up again.
pub fn hub_close(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<(), String> {
    let info = resolve(repo_arg, hub_arg)?;
    let root = crate::registry::state_root(Some(std::path::Path::new(&info.main)));
    let hub = mail::all_repo_hubs(&root, &info)
        .into_iter()
        .find(|h| h.slug == info.slug)
        .unwrap_or_else(|| crate::mail::RepoHub {
            id: format!("hub-{}", info.slug),
            parent: info.hub.is_some(),
            key: info.hub.clone(),
            name: info.hub_name.clone(),
            title: None,
            slug: info.slug.clone(),
            state: crate::mail::RepoHubState {
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
    let recorded = registry::read_hub_record(&root, &hub.slug);
    let named = match &recorded {
        registry::Recorded::Found(r) => r.pid.map(|pid| (pid as u32, r.ps_started.clone())),
        _ => None,
    };
    if let Some((pid, started)) = &named {
        let pid = *pid;
        match registry::hub_process_liveness(pid, started.as_deref()) {
            registry::Liveness::Gone => {}
            registry::Liveness::CannotTell => {
                return Err(registry::hub_cannot_tell(&root, &hub.slug));
            }
            registry::Liveness::Alive => {
                if !registry::is_self_or_descendant_of(pid) {
                    return Err(format!(
                        "{} is still running (pid {pid}); close it from the board, or stop it first \
                         (`adj hub-stop` from inside it, or the board's stop) and then close it",
                        hub.name
                    ));
                }
            }
        }
    } else if matches!(recorded, registry::Recorded::Unreadable) {
        // A record that is there and cannot be read is asked the way `stop_hub` asks, and
        // nobody can be told to be the hub itself. A readable one that names no process is
        // `Gone` to `hub_liveness`, so it goes straight on to be cleared.
        match registry::hub_liveness(&root, &hub.slug) {
            registry::Liveness::Gone => {}
            registry::Liveness::CannotTell => {
                return Err(registry::hub_cannot_tell(&root, &hub.slug));
            }
            registry::Liveness::Alive => {
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
            registry::unregister_hub_if(&root, &hub.slug, *pid, started.as_deref())?
        }
        None => registry::unregister_hub_if_unnamed(&root, &hub.slug)?,
    };
    if !removed {
        return Err(format!("{} changed while it was being closed", hub.name));
    }
    crate::registry::forget_board(&root, &hub.slug)?;
    println!("closed {}", hub.name);
    if hub.inbox_count > 0 {
        println!(
            "{} unread message(s) remain for it; starting the same key shows them",
            hub.inbox_count
        );
    }
    Ok(())
}
