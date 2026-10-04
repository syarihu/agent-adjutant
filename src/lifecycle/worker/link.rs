//! Joining a running worker session to a task, and to the hub that task belongs to.

use std::path::Path;

use crate::infra::paths::same_path;
use crate::registry::{self, Context};
use crate::task::{self, Executor, NewTask, Status, Task, TaskPatch};

/// What to join a session to, and the phase to enter it at.
pub struct LinkRequest {
    pub to: LinkTo,
    /// One of `registry::PHASES`. `None` keeps the record's phase, or enters `implement` when
    /// it has none (`relink_worker`).
    pub phase: Option<String>,
}

/// The task a session is joined to.
pub enum LinkTo {
    /// A task the hub already has, by id.
    Existing(String),
    /// A task made for this link. Its status and worktree are the link's to decide, whatever
    /// the caller set.
    New(NewTask),
}

/// What a link came to.
pub struct Linked {
    /// The task as the link wrote it.
    pub task: Task,
    /// Whether the link made the task rather than taking one that existed. Only a task made
    /// here may ask its hub to file an issue: an existing one already had its request handed
    /// over.
    pub made_here: bool,
}

/// A task write a link made, kept so that a worker record that cannot be written takes it
/// back. A link writes one task today; the list is what lets a second write be undone in
/// order without another shape.
pub enum Done {
    /// The record was made for this link: remove it.
    Created(String),
    /// The record existed: put these fields back.
    Updated(String, Box<TaskPatch>),
}

/// Join the worker running in `worktree` to a task of the hub `ctx` addresses, and to that
/// hub: the task directory a task is in is what says which hub it belongs to, and the
/// worker's record follows it (`ctx.repo.hub`, `None` for the repository's own hub).
///
/// The task is written first and the worker's record second. A task naming a worktree whose
/// worker still reports for nothing is the half-linked state to avoid, so a record that cannot
/// be written takes the task writes back. Telling the worker comes last and is not taken back:
/// the worker's process writes its own record (its phase) while it runs, and putting back the
/// record from before could drop a phase it wrote meanwhile.
pub fn link(ctx: &Context, worktree: &Path, request: LinkRequest) -> Result<Linked, String> {
    if !matches!(
        registry::read_worker_record(worktree),
        registry::Recorded::Found(_)
    ) {
        return Err("the session has not started yet".to_string());
    }
    // A task linked to a worker nobody is running would sit as `dispatched` in a worktree
    // that never reads the notice.
    if !registry::worker_status(worktree).present {
        return Err("the session has ended; resume it first".to_string());
    }
    // Checked before anything is written: a task made or changed for a phase the record would
    // then refuse is the half-linked state the undo exists for.
    let phase = request.phase.as_deref();
    if let Some(phase) = phase.filter(|p| !registry::PHASES.contains(p)) {
        return Err(format!(
            "no such phase: {phase} (one of {})",
            registry::PHASES.join(", ")
        ));
    }
    // Read as the board's session view reads it, so the session the board shows with a task
    // is the one refused here.
    let held = registry::worker_task(worktree);
    let path = worktree.to_string_lossy().to_string();
    let made_here = matches!(request.to, LinkTo::New(_));
    let mut done = Vec::new();
    let linked = match request.to {
        LinkTo::Existing(id) => take(ctx, &path, held.as_deref(), &id, &mut done)?,
        LinkTo::New(new) => make(ctx, &path, held.as_deref(), new, &mut done)?,
    };

    if let Err(e) = registry::relink_worker(worktree, ctx.repo.hub.as_deref(), &linked.id, phase) {
        return Err(match undo(ctx, done) {
            Ok(()) => e,
            Err(not_back) => format!("{e}; and {not_back}"),
        });
    }
    let filing = if made_here && linked.kind == task::Kind::FileAndStart {
        "The hub files the issue for this task; its URL reaches you as `[issue <id>] <url>` \
         and is recorded on the task.\n\n"
    } else if made_here && linked.kind == task::Kind::Start && linked.issue_url.is_none() {
        // `start` reads as "an issue that already exists" in the request below.
        "This task has no issue and none will be filed for it; the Kind line below does not \
         mean one exists.\n\n"
    } else {
        ""
    };
    let body = format!(
        "From now on your Task record is {} and your hub is {}. Fetch adj-worker \
         (`adj skill adj-worker`) and follow it from where the work stands.\n\n{filing}{}",
        linked.id,
        ctx.repo.hub_name,
        task::render_request(&linked)
    );
    crate::mail::deliver_to_worker(
        ctx,
        worktree,
        "dashboard",
        &format!("[linked {}] this session is now a task's worker", linked.id),
        &body,
        None,
    )
    .map_err(|e| {
        format!(
            "{} is linked, but the session could not be told: {e}",
            linked.id
        )
    })?;
    Ok(Linked {
        task: linked,
        made_here,
    })
}

/// Take back `done`, latest first, through the task module's own reverse operations, without
/// handing anything over. Each write is tried even when an earlier one fails, since each one
/// left standing names a worker that does not report for it. `Err` says which could not be put
/// back.
pub fn undo(ctx: &Context, done: Vec<Done>) -> Result<(), String> {
    let failed: Vec<String> = done
        .into_iter()
        .rev()
        .filter_map(|step| {
            let (id, result) = match step {
                Done::Created(id) => {
                    let result = task::remove(ctx, &id);
                    (id, result)
                }
                Done::Updated(id, before) => {
                    let result = task::update(ctx, &id, &before, false).map(|_| ());
                    (id, result)
                }
            };
            result
                .err()
                .map(|e| format!("{id} could not be put back: {e}"))
        })
        .collect();
    match failed.is_empty() {
        true => Ok(()),
        false => Err(failed.join("; and ")),
    }
}

/// Move the existing task `id` to the worker in `worktree`, as `dispatched` (or kept at `pr`),
/// recording in `done` the fields it changed.
fn take(
    ctx: &Context,
    worktree: &str,
    held: Option<&str>,
    id: &str,
    done: &mut Vec<Done>,
) -> Result<Task, String> {
    if !task::is_plain_id(id) {
        return Err(format!("no such task: {id}"));
    }
    if let Some(held) = held
        && held != id
    {
        return Err(format!("this session already has a task: {held}"));
    }
    // Read to choose the status; the checks that matter are made again under the lock.
    let status = match task::get(&ctx.state, &ctx.repo.slug, id)?.status {
        Status::Pr => Status::Pr,
        _ => Status::Dispatched,
    };
    let mut before = TaskPatch::default();
    let (updated, _) = task::update_checked(
        ctx,
        id,
        &TaskPatch {
            worktree: Some(Some(worktree.to_string())),
            status: Some(status),
            note: Some(None),
            ..TaskPatch::default()
        },
        false,
        |existing| {
            if matches!(existing.status, Status::Done | Status::Cancelled) {
                return Err(format!("{id} is finished"));
            }
            if existing.executor == Executor::Jules {
                return Err(format!("{id} is for Jules; a session cannot take it"));
            }
            if let Some(other) = existing
                .worktree
                .as_deref()
                .filter(|w| !same_path(w, worktree))
                && registry::worker_status(Path::new(other)).present
            {
                return Err(format!("{id} already has a worker running in {other}"));
            }
            // Every field the link writes, so an undo puts each one back, absent ones too.
            before = TaskPatch {
                worktree: Some(existing.worktree.clone()),
                status: Some(existing.status),
                note: Some(existing.note.clone()),
                ..TaskPatch::default()
            };
            Ok(())
        },
    )?;
    done.push(Done::Updated(id.to_string(), Box::new(before)));
    Ok(updated)
}

/// Make `new` as the task of the worker in `worktree`, already `dispatched`, recording it in
/// `done`.
fn make(
    ctx: &Context,
    worktree: &str,
    held: Option<&str>,
    mut new: NewTask,
    done: &mut Vec<Done>,
) -> Result<Task, String> {
    if let Some(held) = held {
        return Err(format!("this session already has a task: {held}"));
    }
    if new.executor == Executor::Jules {
        return Err("a session cannot take a task for Jules".to_string());
    }
    new.status = Status::Dispatched;
    new.worktree = Some(worktree.to_string());
    let (created, _) = task::create(ctx, new, false)?;
    done.push(Done::Created(created.id.clone()));
    Ok(created)
}
