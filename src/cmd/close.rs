use super::*;

/// How long to wait for a closed tab's worker to actually be gone, and how often to look.
///
/// Closing a tab hangs its session up and the process in it then unwinds, which is not
/// instant — a single look straight afterwards would call a live worker gone. How long the
/// unwinding takes is the agent's business: a Claude Code session routinely needs more than
/// two seconds, and a budget shorter than that reports a worker whose tab is already closed
/// as still there. The budget only costs anything when the process really does stay, and a
/// terminal waiting for someone to confirm the close does not answer sooner for being
/// given less time, so it is set well past how long an agent takes rather than close to it.
/// Polling keeps the common case — gone at the first few looks — as quick as before.
pub(super) const GONE_BUDGET: Duration = Duration::from_secs(10);
pub(super) const GONE_POLL: Duration = Duration::from_millis(100);

/// Wait for a worker to be gone, and answer with what was actually seen.
///
/// Polled rather than slept through: a process that has already exited by the first look is
/// the common case, and a cleanup step that always cost the whole budget is a step people
/// stop running. `look` and `wait` are handed in for the reason `terminal::close_with` takes
/// its runner — this decision has to be testable without spending the budget in real time.
pub(super) fn settled(
    mut look: impl FnMut() -> messaging::Liveness,
    mut wait: impl FnMut(Duration),
    budget: Duration,
    poll: Duration,
) -> messaging::Liveness {
    // One look before any waiting, then one more per interval until the budget is spent.
    // Counted rather than accumulated, so that a zero interval cannot spin here forever.
    let looks = 1 + budget.as_millis() / poll.as_millis().max(1);
    let mut answer = look();
    for _ in 1..looks {
        if answer == messaging::Liveness::Gone {
            return answer;
        }
        wait(poll);
        answer = look();
    }
    answer
}

/// Why a record was left where it was, said the way a person reads it.
///
/// `None` when it was cleared. The two reasons get a sentence each: announcing "somebody
/// else is working here" for a file that cannot be read sends a person looking for a worker
/// who was never there.
fn left_alone(cleared: &messaging::Cleared, worktree: &std::path::Path) -> Option<String> {
    match cleared {
        messaging::Cleared::Yes => None,
        messaging::Cleared::AnotherWorker => Some(format!(
            "another worker has registered in {} since",
            worktree.display()
        )),
        messaging::Cleared::Unreadable => Some(format!(
            "the record in {} can no longer be read",
            worktree.display()
        )),
    }
}

/// Close the tab the worker in a worktree is sitting in.
///
/// The hub's way of ending a session it started: it is the side that knows the task is
/// over, and a finished worker's tab otherwise stays open with nobody to close it.
///
/// Everything here is about **one** worker, read out of the record once and carried
/// through: the gate, the tab that gets closed, the process that has to be gone afterwards
/// and the record that may then be cleared. Asked separately, each of those questions can
/// be answered about a different worker — the next one registering in the same worktree —
/// and the answers then compose into a worktree that is deleted while somebody is using it.
///
/// `false` means a worker may still be sitting there. The caller is on its way to removing
/// this worktree, so that answer has to reach a shell as an exit code rather than as a
/// sentence in the output — and everything this cannot establish answers `false`, because
/// the cost of the two mistakes is not symmetric: a cleanup that stops is finished by hand,
/// a cleanup that carries on deletes work nobody can get back.
pub fn close(
    repo_arg: Option<&str>,
    worktree: &str,
    quiet: bool,
    dry_run: bool,
) -> Result<bool, String> {
    // Settings rather than a whole `Context`, like `spawn` and `title`: which config to
    // read is the only thing the repository is asked for here, and this is the command most
    // likely to be run while the repository it belongs to is being taken apart.
    let settings = settings_for(repo_arg);
    let worktree = crate::infra::paths::expand_home(worktree);
    // No `is_dir` check, deliberately unlike `tell`: this runs during cleanup, so a
    // worktree that has already been removed is the ordinary way to arrive here twice
    // rather than a mistake worth failing over.
    let worker = match messaging::read_worker(&worktree) {
        // Nothing registered here is the job already done. A hub that calls this twice, or
        // calls it on a worker that stopped on its own, has to get on with the cleanup.
        messaging::Recorded::Absent => {
            if !quiet {
                println!("no worker is running in {}", worktree.display());
            }
            return Ok(true);
        }
        messaging::Recorded::Unreadable => {
            if !quiet {
                println!(
                    "the worker record in {} cannot be read as naming a worker, so nothing was cleared",
                    worktree.display()
                );
            }
            return Ok(false);
        }
        messaging::Recorded::Found(worker) => worker,
    };
    let pid = worker.pid;
    match messaging::worker_liveness(&worker) {
        messaging::Liveness::Gone => {
            // The worker this record named is gone, so the record is the only thing left to
            // clear — and only while it is still that worker's.
            let cleared = match dry_run {
                true => messaging::Cleared::Yes,
                false => messaging::unregister_worker_if(&worktree, &worker)?,
            };
            if let Some(why) = left_alone(&cleared, &worktree) {
                if !quiet {
                    println!("the worker recorded here is gone, but {why}; nothing was cleared");
                }
                return Ok(false);
            }
            if !quiet {
                println!("no worker is running in {}", worktree.display());
                match dry_run {
                    true => println!("(a record is left behind; it would be cleared)"),
                    false => println!("(cleared the record it left behind)"),
                }
            }
            return Ok(true);
        }
        messaging::Liveness::CannotTell => {
            if !quiet {
                println!("cannot tell whether pid {pid} is still running, so nothing was cleared");
            }
            return Ok(false);
        }
        messaging::Liveness::Alive => {}
    }
    let done = terminal::close(
        &settings.terminal,
        pid,
        // The name the tab actually carries: `spawn` put the record's title through
        // `sanitise_title` with the directory name behind it, and a template that matches
        // a tab by name has to be handed the answer that got there.
        &terminal::sanitise_title(
            worker.title.as_deref().unwrap_or_default(),
            worktree
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default(),
        ),
        dry_run,
    )?;
    if dry_run {
        // The description when there is no script to show. The built-in path produces none
        // for a pid whose terminal cannot be found, and a blank line tells the reader less
        // than the sentence explaining why.
        println!(
            "{}",
            match done.script.is_empty() {
                true => &done.description,
                false => &done.script,
            }
        );
        // The exit code still answers about the worktree rather than about the plan — a
        // live worker was found a few lines above, and nothing has been closed. `focus`
        // sets the precedent: its dry run reports the state it looked at. Answering "safe
        // to remove" here is how `close --dry-run && git worktree remove` deletes a live
        // worker's checkout.
        return Ok(false);
    }
    if !done.ran {
        if !quiet {
            println!("{}", done.description);
        }
        return Ok(false);
    }
    // What the close command reported is not the question — `terminal::close` says what
    // little `ran` can mean. Neither is what the record says afterwards: a close command
    // that removed the record instead of the tab would leave a worktree that *looks* free.
    // Only the worker's own absence settles it.
    match settled(
        || messaging::worker_liveness(&worker),
        std::thread::sleep,
        GONE_BUDGET,
        GONE_POLL,
    ) {
        messaging::Liveness::Gone => {
            // The process went with its tab, so a record left behind would have `present`
            // lying to whoever asks next — including the next call to this. Conditional,
            // because the worktree may have been handed to a new worker while this one was
            // being closed, and that worker's record is not this call's to remove.
            let cleared = messaging::unregister_worker_if(&worktree, &worker)?;
            if let Some(why) = left_alone(&cleared, &worktree) {
                if !quiet {
                    println!("pid {pid} is gone, but {why}; nothing was cleared");
                }
                return Ok(false);
            }
            if !quiet {
                println!("{}", done.description);
            }
            Ok(true)
        }
        messaging::Liveness::Alive => {
            if !quiet {
                println!("the close command ran but pid {pid} is still there; nothing was cleared");
                println!(
                    "(a terminal that asks before closing a session with a process in it is waiting for an answer)"
                );
            }
            Ok(false)
        }
        messaging::Liveness::CannotTell => {
            if !quiet {
                println!(
                    "the close command ran but whether pid {pid} is gone cannot be established; nothing was cleared"
                );
            }
            Ok(false)
        }
    }
}
