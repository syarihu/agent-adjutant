use serde_json::{Value, json};

use crate::board::{Server, hub_start_of, input_of, settings_now, text};
use crate::lifecycle::hub::{TabOutcome, start_hub};
use crate::registry::Context;

// ── start a parent-task hub ──────────────────────────────────────────

/// Start the hub for the parent-task `key`, as `adj hub --hub KEY` does. The other way to
/// start a parent-task hub is by the id in `hubs[]`, which only exists once something points
/// at it.
pub fn start_parent_hub(server: &Server, body: &[u8]) -> Result<Value, String> {
    let input = input_of(body)?;
    let key = text(&input, "key")?.ok_or("a key is required")?;
    let start = hub_start_of(&input)?;
    let settings = settings_now(server);
    let ctx = Context {
        repo: server.ctx.repo.clone().addressed(Some(key))?,
        resolved: server.ctx.resolved.clone(),
        state: server.ctx.state.clone(),
        settings,
    };
    let hub = json!({ "id": format!("hub-{key}"), "slug": ctx.repo.slug });
    match start_hub(&ctx, start)? {
        TabOutcome::Opened(done) => Ok(json!({
            "started": true,
            "description": done.description,
            "hub": hub,
        })),
        TabOutcome::AlreadyRunning(status) => Ok(json!({
            "alreadyRunning": true,
            "pid": status.pid,
            "hub": hub,
        })),
    }
}
