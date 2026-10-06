use crate::infra::terminal;
use crate::registry::{self, Context};

/// The hub's tab, brought forward: the pid it was found under and what `terminal.focus` did.
#[derive(Debug)]
pub struct Raised {
    pub pid: u32,
    pub done: terminal::Performed,
}

/// Bring the tab of the hub `ctx` addresses to the front, through `terminal.focus` like a
/// worker's. `Ok(None)` when no hub is running — nothing to raise.
pub fn focus(ctx: &Context, dry_run: bool) -> Result<Option<Raised>, String> {
    let status = registry::hub_status(&ctx.state, &ctx.repo.slug, &ctx.repo.hub_name);
    let Some(pid) = status.pid.filter(|_| status.present) else {
        return Ok(None);
    };
    let done = terminal::focus(
        &ctx.settings.terminal,
        status.terminal.as_ref(),
        pid,
        &ctx.repo.hub_name,
        dry_run,
    )?;
    Ok(Some(Raised { pid, done }))
}
