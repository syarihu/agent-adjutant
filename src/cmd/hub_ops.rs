use super::*;
use crate::lifecycle::hub::{
    AutoResume, Claimed, HubRequest, Planned, Skip, claim_launch, exec_launch, plan_launch,
};

// ── hub (the launcher) ─────────────────────────────────────────

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

/// Start this repository's hub, here, once.
///
/// `tab` opens a tab and starts it there instead, for a caller that is not a person sitting
/// at an empty one — nothing else about the decision changes, including which of the two
/// tabs claims the record.
/// `dashboard` is `--dashboard` / `--no-dashboard`, and `None` when neither was typed — the
/// standing `startupDashboard` then answers on its own.
pub fn hub(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    extra: &[String],
    tab: bool,
    start: HubStart,
    dashboard: Option<bool>,
    dry_run: bool,
) -> Result<(), String> {
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

    // Before planning, because the auto-resume decision belongs to the `adj hub` in the new
    // tab, and `hub_in_tab` makes the presence check itself before opening anything: one hub
    // per address is the invariant, and opening a tab for one that is already up would break
    // it in the one way nothing later repairs — two sessions answering to the same name, with
    // the record naming one of them.
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
    let request = HubRequest {
        start,
        dashboard,
        extra: extra.to_vec(),
    };
    let launch = match plan_launch(&ctx, &request)? {
        Planned::Running(status) => return go_to_running_hub(&ctx, &status, dry_run),
        Planned::Launch(launch) => launch,
    };
    match launch.auto {
        Some(AutoResume::Resuming { ended_secs_ago }) => eprintln!(
            "adjutant: resuming the session that ended {} ago (within hubAutoResumeHours); \
             `adj hub --new` starts a fresh one instead",
            ago(ended_secs_ago)
        ),
        Some(AutoResume::Skipped(Skip::ResumeTemplateHasNoSessionId)) => eprintln!(
            "adjutant: not resuming the last session: hubResumeRunner has no {{sessionId}}"
        ),
        Some(AutoResume::Skipped(Skip::OwnRunner)) => eprintln!(
            "adjutant: not resuming the last session: hubRunner is your own and hubResumeRunner \
             is not set, so the built-in one would reopen it"
        ),
        None => {}
    }
    if dry_run {
        println!("cd {}", crate::infra::template::sh_quote(&ctx.repo.main));
        println!("{}", launch.command);
        return Ok(());
    }

    // Only past the dry run: claiming the record is a write, and a dry run that cleared a
    // live hub's registration would make that hub permanently unreachable — nothing then
    // reports it as present, and every later launch starts another one beside it.
    match claim_launch(&ctx, &launch)? {
        Claimed::Taken(status) => return go_to_running_hub(&ctx, &status, dry_run),
        Claimed::Ours { session_saved } => {
            if let Err(e) = session_saved {
                eprintln!("adjutant: {e}; --resume may not reopen this hub");
            }
        }
    }
    match &launch.resumed {
        Some(saved) => println!(
            "resuming {} (session {}) in {}",
            ctx.repo.hub_name, saved.session_id, ctx.repo.main
        ),
        None => println!("starting {} in {}", ctx.repo.hub_name, ctx.repo.main),
    }
    Err(exec_launch(&ctx, &launch))
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
