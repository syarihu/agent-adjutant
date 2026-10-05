use super::Launch;
use crate::infra::terminal;
use crate::registry::{self, Context};

/// Who got the hub's record.
#[derive(Debug)]
pub enum Claimed {
    /// This launch holds the record. `session_saved` is what became of writing the session
    /// down, for the caller to say if it failed.
    Ours {
        session_saved: Result<(), String>,
    },
    Taken(registry::HubStatus),
}

/// Claim the hub's record for `launch`, tell a resident server where the repository is, and
/// write down the session the hub runs as (or that there is none).
///
/// A failed save is returned in `session_saved` rather than raised: the claim is won either
/// way. The claim and its undo, `exec_launch`, live side by side.
pub fn claim_launch(ctx: &Context, launch: &Launch) -> Result<Claimed, String> {
    // Two launches can reach this line at the same time (a person and a wake-up, two tabs).
    // The claim is what decides between them; the loser is told who won, exactly as if it
    // had arrived a second later.
    // Whether the *rendered* command carries the name, not whether the template has a
    // `{name}` in it: a template that hardcodes the name works, and one that renders it
    // away does not, and only the finished line knows which.
    let named = launch.command.contains(&ctx.repo.hub_name);
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
        registry::Claim::Taken(status) => return Ok(Claimed::Taken(*status)),
    }
    // Where this repository is, for a resident server that may serve its board without
    // being told anything else. Only the address: nothing here needs the server to be up.
    registry::note_board(&ctx.state, &ctx.repo);
    // A hub that cannot be resumed later is still a hub, so failing to write this down is
    // said (the caller says it) and then got past — refusing to start over it would trade a
    // working hub for a convenience.
    //
    // A fresh hub whose runner records no session replaces what was saved with nothing, so
    // that nothing can later reopen the conversation of the hub before it.
    let session_saved = match &launch.resumed {
        Some(_) => Ok(()),
        None => match launch.records {
            true => registry::save_hub_session(
                &ctx.state,
                &ctx.repo.slug,
                &ctx.repo.nwo,
                ctx.repo.hub.as_deref(),
                &ctx.repo.hub_name,
                &launch.session,
            )
            .map(|_| ()),
            false => registry::forget_hub_session(&ctx.state, &ctx.repo.slug),
        },
    };
    Ok(Claimed::Ours { session_saved })
}
