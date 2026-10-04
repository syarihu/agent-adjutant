//! Asking the hub to start the next queued task.

use crate::mail::{DeliveryOutcome, Message};
use crate::registry::Context;

/// Ask the hub to start the next queued task if a worker slot is free.
///
/// For the one case the hub's own procedure cannot see: a worker that died without sending
/// `done`. Its slot came free and nothing woke the hub to say so. A message rather than a
/// bare wake, because a hub that is woken and finds its inbox empty goes straight back to
/// waiting — and one that is not running should find this waiting when it starts.
pub fn nudge(ctx: &Context) -> Result<DeliveryOutcome, String> {
    let message = Message {
        from: "dashboard".to_string(),
        // None, for the reason `hand_over` gives.
        worktree: None,
        kind: "next".to_string(),
        subject: "start the next queued task if a worker slot is free".to_string(),
        body: String::new(),
    };
    crate::mail::deliver_to_hub(ctx, &message)
}
