use crate::infra::terminal::{self, SpawnRequest};
use crate::kernel::identity;
use crate::registry::{self, Context};

/// Refuse when `maxWorkers` workers are already running in this checkout, and otherwise mark
/// `worktree` as taken so the next dispatch in the same turn counts it.
///
/// Counted per checkout rather than per hub: two hubs on one repository share the machine
/// the limit is protecting. `Ok(Some(message))` is the refusal, kept apart from `Err` so the
/// caller can give it its own exit code.
pub fn claim_worker_slot(
    ctx: &Context,
    worktree: &std::path::Path,
    dry_run: bool,
) -> Result<Option<String>, String> {
    let main = std::path::Path::new(&ctx.repo.main);
    // Always under the lock, limit or not: the check that nobody else is starting a worker
    // here and the mark that says so is taken are one step, or two requests at once both pass
    // the check, and a cleanup's last look at the slots means nothing.
    registry::with_dispatch_lock(main, || {
        if registry::is_being_removed(main, worktree) {
            return Err(format!(
                "{} is being removed; nothing was started",
                worktree.display()
            ));
        }
        // Not on a dry run, which marks nothing and so races with nothing.
        if !dry_run && registry::is_starting(worktree, crate::infra::clock::now_secs()) {
            return Err(format!(
                "a worker is already starting in {}; nothing was started",
                worktree.display()
            ));
        }
        if let Some(max) = ctx.settings.max_workers {
            // The main checkout too. It is where the hub sits and a worker is not meant to go,
            // but nothing stops `adj work` being pointed at it, and a worker running there
            // uncounted is one past the limit.
            let mut candidates = identity::linked_worktrees(&ctx.repo.main)?;
            candidates.push(ctx.repo.main.clone());
            let busy = registry::busy_worktrees(&candidates, Some(worktree));
            if busy.len() >= max as usize {
                return Ok(Some(format!(
                    "worker limit reached: {} of maxWorkers {max} are running ({}). \
                     Nothing was started; leave the task queued and dispatch it when one finishes",
                    busy.len(),
                    busy.join(", ")
                )));
            }
        }
        if !dry_run {
            registry::mark_worker_starting(worktree)?;
        }
        Ok(None)
    })?
}

/// Open the tab, and give the slot back if it never opened or the worker died at once (#707).
///
/// With the built-in tmux backend the window is watched for about three seconds, as the hub's is
/// (#182), so a worker that exits at once is an error rather than `started`. A worker that dies
/// after the watch ends is still reported as started; a terminal that cannot be watched
/// (iTerm2, a `terminal.spawn` template) is never checked at all.
pub fn open_worker_tab(
    ctx: &Context,
    worktree: &str,
    request: &SpawnRequest<'_>,
    dry_run: bool,
) -> Result<terminal::Performed, String> {
    let terminal = &ctx.settings.terminal;
    open_worker_tab_with(
        worktree,
        request,
        dry_run,
        &|request, dry_run| terminal::spawn_held(terminal, request, dry_run),
        &|pane| terminal::tmux_watch_start(terminal.tmux_socket(), pane),
    )
}

type Spawned = Result<(terminal::Performed, Option<String>), String>;

/// `open_worker_tab`, with the spawn and the watch passed in so a test can say what they found.
pub fn open_worker_tab_with(
    worktree: &str,
    request: &SpawnRequest<'_>,
    dry_run: bool,
    spawn: &dyn Fn(&SpawnRequest<'_>, bool) -> Spawned,
    watch: &dyn Fn(&str) -> Result<terminal::StartWatch, String>,
) -> Result<terminal::Performed, String> {
    let give_back = || {
        let _ = registry::unmark_worker_starting(std::path::Path::new(worktree));
    };
    let (mut performed, pane) = spawn(request, dry_run).inspect_err(|_| give_back())?;
    let Some(pane) = pane else {
        return Ok(performed);
    };
    // A failed check says nothing about the worker, which may be running: the mark stays, and
    // expires by itself if it is not. Not an error either, for the reason given for status 0
    // below: the tab did open, so it is reported as a terminal that cannot be watched is.
    let watched = match watch(&pane) {
        Ok(watched) => watched,
        Err(e) => {
            performed.description = format!(
                "{}; the tmux window could not be checked ({e}), so the worker may not have stayed up",
                performed.description
            );
            return Ok(performed);
        }
    };
    match watched {
        terminal::StartWatch::Alive => Ok(performed),
        // `adjutant worker` exits 0 when one is already running here. That is not a failed
        // start, and the hub treats any failing `adj work` as "could not start" and
        // re-queues the task, which would put a second copy of a task that is being worked.
        terminal::StartWatch::Exited {
            status: Some(0),
            screen,
        } => {
            give_back();
            performed.description = match screen.as_deref() {
                Some("") | None => {
                    "the worker exited at once without starting anything new".to_string()
                }
                Some(screen) => {
                    format!("the worker exited at once without starting anything new: {screen}")
                }
            };
            Ok(performed)
        }
        terminal::StartWatch::Exited { status, screen } => {
            give_back();
            Err(exited_message(status, screen.as_deref(), worktree))
        }
        terminal::StartWatch::Gone => {
            give_back();
            Err(
                "the worker's window closed at once, before what it printed could be read; \
                 nothing was started"
                    .to_string(),
            )
        }
    }
}

/// What `adj work` says of a worker that exited at once. A screen with nothing on it would
/// leave a sentence ending in a colon, so that case points at running it by hand, as does a
/// screen that could not be read.
pub(super) fn exited_message(status: Option<i32>, screen: Option<&str>, worktree: &str) -> String {
    let status = status.map(|s| format!(" (status {s})")).unwrap_or_default();
    match screen {
        Some("") => format!(
            "the worker exited at once{status} without printing anything; run `adj worker --worktree {worktree}` there to see why"
        ),
        Some(screen) => format!("the worker exited at once{status}: {screen}; nothing was started"),
        None => format!(
            "the worker exited at once{status}, but what it printed could not be read; run `adj worker --worktree {worktree}` there to see why"
        ),
    }
}
