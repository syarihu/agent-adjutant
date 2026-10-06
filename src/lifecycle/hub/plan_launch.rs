use super::{HubStart, asked_session, hub_env, own_hub_runner_refusal};
use crate::kernel::runner;
use crate::lifecycle::{Hooks, agent_hooks_for, resume_template};
use crate::registry::{self, Context};

/// What `adj hub` was asked to start.
#[derive(Debug, Clone)]
pub struct HubRequest {
    pub start: HubStart,
    pub dashboard: Option<bool>,
    pub extra: Vec<String>,
}

/// What starting the hub came to.
#[derive(Debug)]
pub enum Planned {
    Running(registry::HubStatus),
    Launch(Launch),
}

/// The line to run, and the session it runs as.
#[derive(Debug, Clone)]
pub struct Launch {
    pub command: String,
    pub session: String,
    pub resumed: Option<registry::SavedSession>,
    pub records: bool,
    pub auto: Option<AutoResume>,
    /// What came of the hook settings the runner may take; `Skipped` is for the caller to say.
    pub hooks: Hooks,
}

/// What a plain `adj hub` made of the session it had, for the caller to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoResume {
    Resuming { ended_secs_ago: i64 },
    Skipped(Skip),
}

/// Why a plain `adj hub` did not come back to a session that ended recently enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    ResumeTemplateHasNoSessionId,
    OwnRunner,
}

/// What starting the hub `ctx` addresses comes to — already running, or the line to run and
/// the session it runs as — decided without writing a record. The tab route does not come
/// here: it hands the question to the `adj hub` in the new tab.
///
/// Three things go wrong when a person types the agent command by hand, and this exists to
/// take all three away: the session name has to match what a worker will look for, the hub
/// has to run in the main checkout or it cannot cut worktrees, and a second hub for the same
/// repo makes it luck which one a report reaches.
///
/// `request.dashboard` is `--dashboard` / `--no-dashboard`, and `None` when neither was typed
/// — the standing `startupDashboard` then answers on its own. It is carried to the agent as an
/// environment variable rather than resolved here, because the thing that reads it is the
/// hub's procedure, which asks `adjutant_config` for the *resolved* settings.
pub fn plan_launch(ctx: &Context, request: &HubRequest) -> Result<Planned, String> {
    // Asked before the command is even built, so the common "it is already up" case costs
    // nothing. It is not what *enforces* one hub per repository — `claim_launch` is.
    let status = registry::hub_status(&ctx.state, &ctx.repo.slug, &ctx.repo.hub_name);
    if status.present {
        return Ok(Planned::Running(status));
    }
    let asked = asked_session(ctx, request.start)?;
    // Only on this route: the tab route hands the question to the `adj hub` in the new tab,
    // which asks it a moment later with the same answer.
    let (resumed, auto) = match request.start {
        HubStart::Auto => recent_hub_session(ctx),
        _ => (asked, None),
    };
    // The id a fresh hub is started into, when its runner has somewhere to put one. Written
    // down only once the claim is won, in `claim_launch`: a launch that loses the claim
    // started nothing, and saving its id would point the next `--resume` at a conversation
    // that never began.
    let session = match &resumed {
        Some(saved) => saved.session_id.clone(),
        None => registry::new_session_id()?,
    };
    let records = resumed.is_some()
        || runner::records_session(
            ctx.settings.hub_runner.as_deref(),
            runner::DEFAULT_HUB_RUNNER,
        );
    let mut env = hub_env(ctx, request.dashboard);
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
    // The hook settings are a file, not a record: written on a dry run too, and the same
    // bytes every time for one binary.
    let (template, default) = match &resumed {
        Some(_) => (
            resume_template(ctx.settings.hub_resume_runner.as_deref(), "hubResumeRunner")?,
            runner::DEFAULT_HUB_RESUME_RUNNER,
        ),
        None => (
            ctx.settings.hub_runner.as_deref(),
            runner::DEFAULT_HUB_RUNNER,
        ),
    };
    let hooks = agent_hooks_for(&ctx.state, template.unwrap_or(default));
    let settings = hooks.path().map(|path| path.to_string_lossy());
    let settings = settings.as_deref();
    let mut command = match &resumed {
        Some(_) => runner::hub_resume_command(
            template,
            &env,
            &ctx.repo.hub_name,
            &session,
            runner::HUB_RESUME_PROMPT,
            settings,
        ),
        None => runner::hub_command(
            template,
            &env,
            &ctx.repo.hub_name,
            &session,
            runner::HUB_STARTUP_PROMPT,
            settings,
        ),
    };
    if !request.extra.is_empty() {
        command = format!(
            "{command} {}",
            crate::infra::template::sh_join(&request.extra)
        );
    }
    Ok(Planned::Launch(Launch {
        command,
        session,
        resumed,
        records,
        auto,
        hooks,
    }))
}

/// The saved session a plain `adj hub` comes back to, if it ended recently enough, and what
/// the caller should say about it.
///
/// "Ended" is the last time the hub's MCP server said the session was alive — it beats
/// every minute and once more as the agent closes it. Where nothing has said so (no MCP
/// server, a runner that is not the agent this knows, a session saved by an older version)
/// there is no answer, and no answer starts fresh: coming back uninvited to a conversation of
/// unknown age is worse than one clean start too many.
fn recent_hub_session(ctx: &Context) -> (Option<registry::SavedSession>, Option<AutoResume>) {
    let window = ctx.settings.hub_auto_resume_hours;
    if window <= 0.0 {
        return (None, None);
    }
    let Some(saved) = registry::hub_session(&ctx.state, &ctx.repo.slug) else {
        return (None, None);
    };
    let Some(last) = registry::hub_last_alive(&ctx.state, &ctx.repo.slug, &saved.session_id) else {
        return (None, None);
    };
    let age = crate::infra::clock::now_secs().saturating_sub(last).max(0);
    if age as f64 > window * 3600.0 {
        return (None, None);
    }
    // A resume template that cannot be told the session would make this refuse, and a
    // refusal is the wrong answer to a command that was not asked to resume anything.
    if resume_template(ctx.settings.hub_resume_runner.as_deref(), "hubResumeRunner").is_err() {
        return (
            None,
            Some(AutoResume::Skipped(Skip::ResumeTemplateHasNoSessionId)),
        );
    }
    // A hub started by a runner of its own would be reopened by the built-in one — without
    // whatever that runner added, or as another agent entirely. Asked for outright, that is
    // the person's call and `--resume` makes it; uninvited, it is not.
    if own_hub_runner_refusal(&ctx.settings).is_some() {
        return (None, Some(AutoResume::Skipped(Skip::OwnRunner)));
    }
    (
        Some(saved),
        Some(AutoResume::Resuming {
            ended_secs_ago: age,
        }),
    )
}
