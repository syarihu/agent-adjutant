//! Handing a task to the hub: the request in its inbox.

use super::*;

use crate::mail::{DeliveryOutcome, Message};
use crate::registry::Context;

/// Put the task in the hub's inbox and poke its tab — the same delivery `adj send` performs,
/// through the same code, so waking and notifying cannot drift between the two callers.
pub fn hand_over(ctx: &Context, task: &Task) -> Result<DeliveryOutcome, String> {
    let message = Message {
        from: "dashboard".to_string(),
        // Deliberately none. The sender is a person at a browser, not a worktree, and a
        // `worktree:` header here would name whichever directory the server was started in
        // — which the hub would then act on as if a worker had reported from it.
        worktree: None,
        kind: "request".to_string(),
        subject: task.title.clone(),
        body: render_request(task),
    };
    crate::mail::deliver_to_hub(ctx, &message)
}
