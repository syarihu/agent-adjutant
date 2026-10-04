use super::*;
use crate::lifecycle::worker::WorkerClose;

/// Why a record was left where it was, said the way a person reads it.
///
/// `None` when it was cleared. The two reasons get a sentence each: announcing "somebody
/// else is working here" for a file that cannot be read sends a person looking for a worker
/// who was never there.
fn left_alone(cleared: &registry::Cleared, worktree: &std::path::Path) -> Option<String> {
    match cleared {
        registry::Cleared::Yes => None,
        registry::Cleared::AnotherWorker => Some(format!(
            "another worker has registered in {} since",
            worktree.display()
        )),
        registry::Cleared::Unreadable => Some(format!(
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
/// `false` is `WorkerClose::is_free` saying a worker may still be sitting there, which has
/// to reach a shell as an exit code rather than as a sentence in the output.
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
    let outcome = crate::lifecycle::worker::close(&settings, &worktree, dry_run)?;
    say(&outcome, &worktree, quiet);
    Ok(outcome.is_free())
}

/// What closing came to, in words. A dry run's plan is the one thing said under `--quiet`:
/// it is what the flag was asked for.
fn say(outcome: &WorkerClose, worktree: &std::path::Path, quiet: bool) {
    match outcome {
        WorkerClose::WouldClose(done) => {
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
        }
        _ if quiet => {}
        WorkerClose::NoWorker => println!("no worker is running in {}", worktree.display()),
        WorkerClose::Unreadable => println!(
            "the worker record in {} cannot be read as naming a worker, so nothing was cleared",
            worktree.display()
        ),
        WorkerClose::Gone { dry_run } => {
            println!("no worker is running in {}", worktree.display());
            match dry_run {
                true => println!("(a record is left behind; it would be cleared)"),
                false => println!("(cleared the record it left behind)"),
            }
        }
        WorkerClose::GoneLeftAlone(cleared) => {
            if let Some(why) = left_alone(cleared, worktree) {
                println!("the worker recorded here is gone, but {why}; nothing was cleared");
            }
        }
        WorkerClose::CannotTell { pid } => {
            println!("cannot tell whether pid {pid} is still running, so nothing was cleared");
        }
        WorkerClose::NotRun(done) | WorkerClose::Closed(done) => println!("{}", done.description),
        WorkerClose::ClosedLeftAlone { pid, cleared } => {
            if let Some(why) = left_alone(cleared, worktree) {
                println!("pid {pid} is gone, but {why}; nothing was cleared");
            }
        }
        WorkerClose::StillRunning { pid } => {
            println!("the close command ran but pid {pid} is still there; nothing was cleared");
            println!(
                "(a terminal that asks before closing a session with a process in it is waiting for an answer)"
            );
        }
        WorkerClose::GoneUnknown { pid } => println!(
            "the close command ran but whether pid {pid} is gone cannot be established; nothing was cleared"
        ),
    }
}
