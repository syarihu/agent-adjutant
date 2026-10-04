use super::resume::saved_hub_session;
use crate::infra::paths::exe_path;
use crate::infra::terminal::{self, SpawnRequest};
use crate::lifecycle::{forwarded_env, resume_template, title_command};
use crate::registry::{self, Context, agent_env};

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
pub fn hub_env(ctx: &Context, dashboard: Option<bool>) -> Vec<(String, String)> {
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
    AlreadyRunning(registry::HubStatus),
    Opened(terminal::Performed),
}

/// The saved session `--resume` names, or a refusal — and, for the template it will run,
/// the same. Looked up before either route, so that asking for a session that is not there is
/// refused here, in the tab it was typed in — not in a tab opened to show the refusal. The
/// template is checked here too, for the same reason: on the tab route the refusal would
/// otherwise come from inside a tab this one had already reported as opened.
pub fn asked_session(
    ctx: &Context,
    start: HubStart,
) -> Result<Option<registry::SavedSession>, String> {
    match start {
        HubStart::Resume => {
            let saved = saved_hub_session(ctx)?;
            resume_template(ctx.settings.hub_resume_runner.as_deref(), "hubResumeRunner")?;
            Ok(Some(saved))
        }
        HubStart::Auto | HubStart::New => Ok(None),
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
    let status = registry::hub_status(&ctx.state, &ctx.repo.slug, &ctx.repo.hub_name);
    if status.present {
        return Ok(TabOutcome::AlreadyRunning(status));
    }
    asked_session(ctx, start)?;
    match registry::hub_liveness(&ctx.state, &ctx.repo.slug) {
        registry::Liveness::CannotTell => {
            Err(registry::hub_cannot_tell(&ctx.state, &ctx.repo.slug))
        }
        registry::Liveness::Alive => Ok(TabOutcome::AlreadyRunning(status)),
        registry::Liveness::Gone => {
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

/// Open a tab and start this repository's hub in it, rather than becoming it here.
///
/// What the tab runs is `adj hub` — this same command without `--tab`. The claim is left to
/// it, and that is the whole reason the split exists: a claim records the claiming process's
/// PID, so claiming here would write down a launcher that is about to exit, for a hub that
/// is a different process in another tab. Every later liveness check would then be asking
/// about the wrong one, and the first `--hub` that answered "gone" would start a second hub
/// beside the live one. See the comment above the claim in `claim_launch`.
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
