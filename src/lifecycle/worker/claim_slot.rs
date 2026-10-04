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

/// Open the tab, and give the slot back if it never opened.
pub fn open_worker_tab(
    ctx: &Context,
    worktree: &str,
    request: &SpawnRequest<'_>,
    dry_run: bool,
) -> Result<terminal::Performed, String> {
    terminal::spawn(&ctx.settings.terminal, request, dry_run).inspect_err(|_| {
        let _ = registry::unmark_worker_starting(std::path::Path::new(worktree));
    })
}
