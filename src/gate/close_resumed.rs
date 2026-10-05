//! Closing the waiting gates whose worker has visibly moved on.

use std::path::Path;

use super::store::{archive, load, try_lock};
use super::*;

use crate::registry::{self, Context};

/// Close the waiting gates whose worker has visibly moved on, as answered in the terminal.
///
/// The fallback for a worker that was answered in its terminal and did not close the gate:
/// a worker that has changed phase, or opened another gate, since it opened this one is not
/// waiting on it any more, and the board should not go on asking for an answer nobody is
/// waiting for. Only gates a worker opened are considered (`gate::resumed_at` says which),
/// and the same worker: the worktree's worker record must have started before the
/// gate was opened. Best effort, and silent: this runs on the board's gate sweep, and in a hub
/// process whose stdout is the MCP stream.
pub fn close_resumed(ctx: &Context) -> Vec<Gate> {
    let open = list(&ctx.state, &ctx.repo.slug, Shelf::Open);
    if open.iter().all(|g| !g.wait || g.answered_by_hub()) {
        return Vec::new();
    }
    let signals = resume_signals(&ctx.state, &ctx.repo.slug, &open);
    let mut closed = Vec::new();
    for g in &open {
        if !g.wait || g.answered_by_hub() {
            continue;
        }
        let record = match registry::read_worker_record(Path::new(&g.worktree)) {
            registry::Recorded::Found(record) => Some(record),
            _ => None,
        };
        let started = record.as_ref().and_then(|r| r.started_at.as_deref());
        let phase_at = record
            .as_ref()
            .and_then(|r| r.phase_at)
            .map(crate::infra::clock::utc_stamp);
        let later = signals
            .iter()
            .filter(|s| s.worktree == g.worktree && s.id != g.id && s.opened_at > g.opened_at)
            .min_by(|a, b| a.opened_at.cmp(&b.opened_at));
        let Some((at, signal)) = resumed_at(
            g,
            started,
            phase_at.as_deref(),
            later.map(|s| s.opened_at.as_str()),
        ) else {
            continue;
        };
        let comment = match signal {
            Signal::Phase => format!(
                "closed by the board: the worker moved on to phase {} at {at}",
                record
                    .as_ref()
                    .and_then(|r| r.phase.as_deref())
                    .unwrap_or("unknown")
            ),
            Signal::Gate => format!(
                "closed by the board: the worker opened gate {}",
                later.map_or("", |s| s.id.as_str())
            ),
        };
        // Skipped when somebody is deciding it right now: whoever holds the lock is
        // answering or closing it, and that is the decision that stands.
        let Ok(Some(_lock)) = try_lock(ctx, Shelf::Open, &g.id) else {
            continue;
        };
        let Ok(mut fresh) = load(&ctx.state, &ctx.repo.slug, Shelf::Open, &g.id) else {
            continue;
        };
        fresh.decision = Some(TERMINAL.to_string());
        fresh.comment = Some(comment);
        fresh.answered_at = Some(at.clone());
        if archive(ctx, &fresh).is_err() {
            continue;
        }
        note_answered(ctx, fresh.task.as_deref(), &at);
        closed.push(fresh);
    }
    closed
}
