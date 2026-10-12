//! The PRs that asked for your review, as the page reads them.

use std::path::Path;

use serde::Serialize;

use crate::infra::clock::now_secs;
use crate::kernel::identity::review_hub_at;
use crate::lifecycle::hub::hub_startable;
use crate::others::{LastSync, MAX_READING, Record, last_sync, list, record};
use crate::registry::{self, Context};

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
///
/// A task's PR that nobody requested of you is left out of `records`, which is the list; its
/// reader still counts in `hub`, which says what the hub is doing.
pub fn others(root: &Path) -> OthersState {
    let all = list(root);
    let hub = Some(review_hub(root, &all));
    OthersState {
        now: now_secs(),
        last_sync: last_sync(root),
        records: all.into_iter().filter(Record::on_review_list).collect(),
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

/// The read-through of a task's PR, as `GET /api/tasks/<id>/ai` sends it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRead {
    /// The task's PR as `owner/repo#N`; `None` when the task has none, or one this cannot read.
    pub pr: Option<String>,
    /// The PR's record: `None` when it was never asked to be read and is not on the list either.
    /// Found through the task's current PR, so a task that gets another PR shows that one's.
    pub record: Option<Record>,
    /// The code-review hub: whether a read can be asked for now, and how busy its places are.
    pub hub: ReviewHub,
}

/// What the task `task_id` of the hub `ctx` is for has of a read: its PR, the record of it (whether
/// the record is linked to the task or only requested of you) and the hub. Read-only. Takes the
/// `Context` and not a root and a slug only because a task's bare `#N` is read against its
/// repository's origin.
pub fn task_read(ctx: &Context, task_id: &str) -> Result<TaskRead, String> {
    let root = &ctx.state;
    let task = crate::task::get(root, &ctx.repo.slug, task_id)?;
    let pr = crate::task::pr_refs(ctx, std::slice::from_ref(&task))
        .into_iter()
        .next()
        .flatten()
        // The records are keyed by owner and repository only: a PR on another host would find another's.
        .filter(|pr| pr.host == "github.com");
    let record = match &pr {
        Some(pr) => record(root, pr)?,
        None => None,
    };
    Ok(TaskRead {
        pr: pr.as_ref().map(|pr| format!("{}#{}", pr.nwo(), pr.number)),
        record,
        hub: review_hub(root, &list(root)),
    })
}
