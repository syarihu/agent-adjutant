use crate::infra::clock::now_secs;
use crate::lifecycle::hub::hub_startable;
use crate::others::{self, Gh, HungReader, ReadMode, Requested, TaskLink, TrackError};
use crate::registry::{self, Context, Liveness};
use crate::task;
use crate::task::PrRef;

use super::{AskError, Asked, dispatch, request_taking_over, stop_reader};

/// Why asking for a read of a task's PR was refused. The transports word it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskAskError {
    /// There is no such task.
    NoTask,
    /// The task's record is there and cannot be read, and why.
    TaskUnreadable(String),
    /// The task has no PR yet.
    NoPr,
    /// The task's PR is not one this can read: not on github.com, or a bare number with no
    /// remote to read it against. Holds the PR as the task names it.
    NotReadable(String),
    /// A sync is running, which also writes the PR's record: nothing was written; ask again once it
    /// ends.
    Syncing,
    /// The task's PR changed while GitHub was read: nothing was queued; ask again.
    PrChanged,
    /// The read was refused as a request's is, or GitHub could not be read.
    Refused(AskError),
}

/// Ask for a read-through of the PR of task `task_id` of the hub `ctx` is for, and start the
/// readers that are next in line. The PR's record is the one a request makes, linked to the task:
/// it waits in the same line of three, and is refused while the code-review hub is not up.
pub fn request_for_task(
    ctx: &Context,
    task_id: &str,
    mode: ReadMode,
) -> Result<Asked, TaskAskError> {
    let root = &ctx.state;
    let review =
        registry::review_context(root).map_err(|e| TaskAskError::Refused(AskError::Failed(e)))?;
    let hub_up =
        registry::hub_status(&review.state, &review.repo.slug, &review.repo.hub_name).present;
    let startable = hub_startable(&review.settings.terminal);
    let requested = request_for_task_with(
        ctx,
        task_id,
        mode,
        now_secs(),
        hub_up,
        startable,
        &registry::hub_process_liveness,
        &|reader| stop_reader(&review, reader),
        &|args, deadline| crate::infra::gh::run(None, args, deadline),
    )?;
    // Outside the record's lock, which `request_read` has let go of.
    let queue_error = dispatch(root).error;
    // The record as it is after the dispatch, which may have started this very read.
    let id = match &requested {
        Requested::Queued(record) | Requested::Already(record) => record.id.clone(),
    };
    let now = others::list(root).into_iter().find(|r| r.id == id);
    let requested = match (requested, now) {
        (Requested::Queued(_), Some(record)) => Requested::Queued(record),
        (Requested::Already(_), Some(record)) => Requested::Already(record),
        (asked, None) => asked,
    };
    Ok(Asked {
        requested,
        queue_error,
    })
}

/// `request_for_task`, with what it asked the machine (is the hub up, can a reader be opened, is
/// the old reader alive) and the way to run `gh` handed in, and without the dispatch. Every refusal
/// that can be known without GitHub comes before the first `gh` call and the first write.
#[allow(clippy::too_many_arguments)]
pub(crate) fn request_for_task_with(
    ctx: &Context,
    task_id: &str,
    mode: ReadMode,
    now: i64,
    hub_up: bool,
    startable: bool,
    alive: &dyn Fn(u32, Option<&str>) -> Liveness,
    stop: &dyn Fn(&HungReader) -> Result<(), String>,
    gh: Gh,
) -> Result<Requested, TaskAskError> {
    let root = &ctx.state;
    let (found, pr) = task_pr(ctx, task_id)?;
    if !hub_up {
        return Err(TaskAskError::Refused(AskError::HubStopped));
    }
    if !startable {
        return Err(TaskAskError::Refused(AskError::NotStartable));
    }
    let link = TaskLink {
        board: ctx.repo.slug.clone(),
        id: found.id.clone(),
        extra: Default::default(),
    };
    // The repository as GitHub spells it: the board's own when it is this one, else as written.
    let repo = if ctx.repo.nwo.eq_ignore_ascii_case(&pr.nwo()) {
        ctx.repo.nwo.clone()
    } else {
        pr.nwo()
    };
    others::track_task_pr_with(root, &pr, &link, &repo, now, gh).map_err(|e| match e {
        TrackError::Syncing => TaskAskError::Syncing,
        TrackError::Failed(why) => TaskAskError::Refused(AskError::Failed(why)),
    })?;
    // The task may have been given another PR while GitHub was read: this one is not the task's now,
    // and is not queued for it.
    if task_pr(ctx, task_id)?.1 != pr {
        return Err(TaskAskError::PrChanged);
    }
    request_taking_over(root, &pr, now, mode, hub_up, startable, alive, stop)
        .map_err(TaskAskError::Refused)
}

/// The task and the PR it names now, or why it names none this can read. A task record that is
/// there and cannot be read is not "no such task".
fn task_pr(ctx: &Context, task_id: &str) -> Result<(task::Task, PrRef), TaskAskError> {
    let root = &ctx.state;
    let slug = &ctx.repo.slug;
    let found = match task::get(root, slug, task_id) {
        Ok(found) => found,
        Err(why) => {
            return Err(match task::exists(root, slug, task_id) {
                Ok(false) => TaskAskError::NoTask,
                _ => TaskAskError::TaskUnreadable(why),
            });
        }
    };
    let text = found.pr.clone().ok_or(TaskAskError::NoPr)?;
    let pr = task::pr_refs(ctx, std::slice::from_ref(&found))
        .into_iter()
        .next()
        .flatten()
        .filter(|pr| pr.host == "github.com")
        .ok_or(TaskAskError::NotReadable(text))?;
    Ok((found, pr))
}
