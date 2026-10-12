//! The PRs that asked for your review, as the page reads them.

use std::path::Path;

use serde::Serialize;

use crate::infra::clock::now_secs;
use crate::kernel::identity::review_hub_at;
use crate::lifecycle::hub::hub_startable;
use crate::others::{LastSync, MAX_READING, Record, last_sync, list};
use crate::registry;

/// The document `GET /api/others` sends.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OthersState {
    pub now: i64,
    /// `None` before the first sync.
    pub last_sync: Option<LastSync>,
    pub records: Vec<Record>,
    /// The code-review hub, whose row comes first in the list. Always there on the resident.
    pub hub: Option<ReviewHub>,
}

/// The code-review hub as its row shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewHub {
    /// `running` or `stopped`.
    pub state: &'static str,
    pub slug: String,
    /// The saved session of the hub, when it has one.
    pub session_id: Option<String>,
    /// Reads being made now.
    pub running: usize,
    /// Reads waiting for a place.
    pub queued: usize,
    /// How many may be made at once.
    pub max: usize,
    /// Whether the board may start the hub: it is stopped, and the terminal is one a hub can be
    /// opened in.
    pub startable: bool,
    /// Whether a read can be started now: the hub is running, and the terminal is one a reader
    /// can be opened in (`terminal.preset` is tmux). A running hub with another terminal is not
    /// `startable` either way, and answers `request` with `not-startable`.
    pub can_read: bool,
}

/// Read-only: a sync is the only thing that writes these records. The hub's state is a read
/// too.
pub fn others(root: &Path) -> OthersState {
    let records = list(root);
    let hub = Some(review_hub(root, &records));
    OthersState {
        now: now_secs(),
        last_sync: last_sync(root),
        records,
        hub,
    }
}

fn review_hub(root: &Path, records: &[Record]) -> ReviewHub {
    let info = review_hub_at(root);
    let count = |status: &str| {
        records
            .iter()
            .filter(|r| r.ai.as_ref().is_some_and(|ai| ai.status == status))
            .count()
    };
    let up = registry::hub_status(root, &info.slug, &info.hub_name).present;
    // A config that cannot be read is a terminal nothing can be opened in.
    let tmux =
        registry::review_context(root).is_ok_and(|ctx| hub_startable(&ctx.settings.terminal));
    let startable = !up && tmux;
    ReviewHub {
        state: if up { "running" } else { "stopped" },
        session_id: registry::hub_session(root, &info.slug).map(|saved| saved.session_id),
        slug: info.slug,
        running: count("running"),
        queued: count("queued"),
        max: MAX_READING,
        startable,
        can_read: up && tmux,
    }
}
