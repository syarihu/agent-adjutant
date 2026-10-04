//! Closing a gate without delivering an answer.

use super::store::{archive, exists, load, lock, stamp};
use super::*;

use crate::registry::Context;

/// Archive a gate without delivering an answer to the worker's outbox.
///
/// Used when the conversation happened directly in a terminal tab, or when a gate was
/// rendered moot. It leaves the gate in `answered/` with decision "closed", or "terminal"
/// when the person answered it in the worker's terminal, so the record survives, but skips
/// the delivery and the wake.
///
/// If the gate was already archived (answered on the board, or closed by the board's sweep),
/// returns the archived gate rather than failing: the caller can inspect `decision`.
pub fn close(
    ctx: &Context,
    id: &str,
    comment: Option<&str>,
    terminal: bool,
) -> Result<Gate, String> {
    if !exists(&ctx.state, &ctx.repo.slug, Shelf::Open, id) {
        return load(&ctx.state, &ctx.repo.slug, Shelf::Answered, id)
            .map_err(|_| format!("no open gate: {id}"));
    }
    let _lock = lock(ctx, Shelf::Open, id)?;
    // Read under the lock: an answer or a sweep that got there first has archived it.
    let mut gate = match load(&ctx.state, &ctx.repo.slug, Shelf::Open, id) {
        Ok(g) => g,
        Err(e) => {
            return load(&ctx.state, &ctx.repo.slug, Shelf::Answered, id).map_err(|_| e);
        }
    };
    gate.decision = Some(if terminal { TERMINAL } else { CLOSED }.to_string());
    gate.comment = comment
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    let at = stamp();
    gate.answered_at = Some(at.clone());
    archive(ctx, &gate)?;
    note_answered(ctx, gate.task.as_deref(), &at);
    Ok(gate)
}
