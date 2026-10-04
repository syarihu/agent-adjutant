//! Noting on a task that its gate was answered.

use super::*;

use crate::registry::Context;

/// Write the answer's time onto the gate's task `id`, for the board's stuck badge. Best
/// effort: the answer has been delivered and archived by now, and a task record that is
/// missing or unwritable must not turn that into a failure.
pub fn note_gate_answered(ctx: &Context, id: &str, at: &str) {
    // Under the task's lock, like `task::update`: a whole-record write racing another would
    // undo whichever landed first.
    let Ok(_lock) = store::lock(ctx, id) else {
        return;
    };
    if let Ok(mut task) = store::load(&ctx.state, &ctx.repo.slug, id) {
        task.gate_answered_at = Some(at.to_string());
        let _ = store::save(ctx, &task);
    }
}
