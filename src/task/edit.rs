//! Changing one task record under its lock, for a caller that has done something outside it.

use super::*;

use crate::registry::Context;

/// What `edit`'s closure decided about the record it was given.
#[derive(Debug)]
pub enum Edit<T> {
    /// Leave the record as it is.
    Keep(T),
    /// Stamp the record and save it.
    Write(T),
}

/// What `edit` did: the closure's value, and whether the save it asked for worked.
///
/// The save comes back beside the value rather than in place of it. The closure may already
/// have done something outside the record by then — created a session, posted a message — and
/// the caller still has to act on that, or say which one it was.
#[must_use]
pub struct Edited<T> {
    pub value: T,
    pub saved: Result<(), String>,
}

/// Change task `id` under its lock: load the record, let `change` look at it and alter it,
/// and save it whole if `change` says `Write`, with `updatedAt` stamped.
///
/// The lock is held across `change`, so what it checks is still true when the record is
/// written, and is let go before this returns, so the caller can do the slow work (waking the
/// hub) after. An `Err` from `change` is returned as it is and nothing is written.
pub fn edit<T>(
    ctx: &Context,
    id: &str,
    change: impl FnOnce(&mut Task) -> Result<Edit<T>, String>,
) -> Result<Edited<T>, String> {
    let _lock = store::lock(ctx, id)?;
    let mut task = store::load(&ctx.state, &ctx.repo.slug, id)?;
    Ok(match change(&mut task)? {
        Edit::Keep(value) => Edited {
            value,
            saved: Ok(()),
        },
        Edit::Write(value) => {
            task.updated_at = store::stamp();
            Edited {
                value,
                saved: store::save(ctx, &task).map(|_| ()),
            }
        }
    })
}
