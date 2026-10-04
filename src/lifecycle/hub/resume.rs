use crate::kernel::config;
use crate::lifecycle::resume_template;
use crate::registry::{self, Context};

/// Everything a restart of the hub `ctx` addresses has to know can be resumed, checked before
/// the hub is stopped: a restart that stops the hub and only then finds there is nothing to
/// come back to has cost the person the session it was meant to keep.
///
/// Beyond what `--resume` itself refuses, a `hubRunner` of the person's own with no
/// `hubResumeRunner` is refused too. Typed out, `--resume` is the person's call to have the
/// built-in runner reopen it; a button that stops a running hub is not asking anyone.
pub fn hub_resume_check(ctx: &Context) -> Result<registry::SavedSession, String> {
    let saved = saved_hub_session(ctx)?;
    resume_template(ctx.settings.hub_resume_runner.as_deref(), "hubResumeRunner")?;
    if let Some(refusal) = own_hub_runner_refusal(&ctx.settings) {
        return Err(refusal);
    }
    Ok(saved)
}

/// Why a hub started by a runner of the person's own cannot be reopened by the built-in one:
/// `hubRunner` is set and `hubResumeRunner` is not. `None` when that is not the case.
pub fn own_hub_runner_refusal(settings: &config::Settings) -> Option<String> {
    (settings.hub_runner.is_some() && settings.hub_resume_runner.is_none()).then(|| {
        "hubRunner is your own and hubResumeRunner is not set, so the built-in runner \
         would reopen the session instead of yours"
            .to_string()
    })
}

/// The session `adj hub --resume` reopens, or a refusal that says what can be resumed.
///
/// Asking for a hub that has nothing saved is most often asking for the wrong one — the
/// repository's own hub when it was a parent task's, or the other way round — so the refusal
/// lists what this repository does have rather than only saying "no".
pub(super) fn saved_hub_session(ctx: &Context) -> Result<registry::SavedSession, String> {
    if let Some(saved) = registry::hub_session(&ctx.state, &ctx.repo.slug) {
        return Ok(saved);
    }
    let mut message = format!("{} has no saved session to resume.", ctx.repo.hub_name);
    let others = registry::hub_sessions_for(&ctx.state, &ctx.repo.nwo);
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
