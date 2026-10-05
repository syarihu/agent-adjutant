use std::path::Path;

use super::start::{hub_context, start_if_stopped};
use crate::board::view::find_session;
use crate::board::{Server, settings_now};
use crate::lifecycle::worker::{LinkRequest, LinkTo, Linked};
use crate::mail::{DeliveryOutcome, Message};
use crate::task::{self, Task};

/// What the person asked for when linking a session to a task.
pub struct LinkSession {
    /// The `hubs[].id` whose task directory the task is in or made in; this board's own when none.
    pub hub: Option<String>,
    pub to: LinkTo,
    pub phase: Option<String>,
}

/// What a link came to.
pub struct SessionLinked {
    pub task: Task,
    /// The id of the session that was linked.
    pub session: String,
    /// The `hubs[].id` the session now reports to.
    pub hub: String,
    /// `None` when no issue was to be filed; otherwise whether the hub was told to file one.
    pub file_issue: Option<Result<IssueAsked, String>>,
}

/// What asking a hub to file the issue of a task this link made came to.
pub struct IssueAsked {
    pub delivered: DeliveryOutcome,
    pub hub_started: Result<bool, String>,
}

/// Join the session `id` to a task, and to the hub that task belongs to.
///
/// The task is either an existing one or a new one, and is looked up or made in the task
/// directory of the hub named by `hub` — that directory is what says which hub a task belongs
/// to, and the worker's record follows it. The link itself is `lifecycle::worker::link`'s; what
/// is here is finding the session and the hub, and the issue the hub is asked to file.
pub fn link(server: &Server, id: &str, request: LinkSession) -> Result<SessionLinked, String> {
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    if session.kind != "worker" {
        return Err("only a worker session can be linked to a task".to_string());
    }
    let (hub, ctx) = hub_context(server, request.hub.as_deref(), settings)?;
    let worktree = session.worktree.as_str();
    let Linked {
        task: linked,
        made_here,
    } = crate::lifecycle::worker::link(
        &ctx,
        Path::new(worktree),
        LinkRequest {
            to: request.to,
            phase: request.phase,
        },
    )?;
    // Only a task this link made: an existing one may carry `file-and-start` from its own
    // request, which was already handed to a hub.
    let files_issue = made_here && linked.kind == task::Kind::FileAndStart;
    let file_issue = if files_issue {
        // The link stands whatever happens here: the worker already has its task, and a hub that
        // could not be told is something to say, not to undo.
        let message = Message {
            from: "dashboard".to_string(),
            worktree: None,
            kind: "file-issue".to_string(),
            subject: format!("[file {}] {}", linked.id, linked.title),
            body: format!(
                "{}\n## Worker running in {worktree}\n",
                task::render_request(&linked)
            ),
        };
        Some(match crate::mail::deliver_to_hub(&ctx, &message) {
            Ok(delivered) => {
                let hub_started = start_if_stopped(server, &ctx, &delivered);
                Ok(IssueAsked {
                    delivered,
                    hub_started,
                })
            }
            Err(e) => {
                // The worker was told the hub would file it: say it will not, in the words it
                // already handles, so it does not wait for a URL.
                let _ = crate::mail::deliver_to_worker(
                    &ctx,
                    Path::new(worktree),
                    "dashboard",
                    &format!("[issue {}] not filed: the hub could not be told", linked.id),
                    "Nothing will file an issue for this task automatically. Carry on without one.",
                    None,
                );
                Err(e)
            }
        })
    } else {
        None
    };
    Ok(SessionLinked {
        task: linked,
        session: session.id.clone(),
        hub: hub.id,
        file_issue,
    })
}
