use super::args::TellArgs;
use super::*;

pub fn tell(args: &TellArgs) -> Result<(), String> {
    let ctx = context(args.repo.as_deref(), args.hub.as_deref())?;
    let worktree = crate::infra::paths::expand_home(&args.worktree);
    if !worktree.is_dir() {
        return Err(format!("no such worktree: {}", worktree.display()));
    }
    let body = read_body(args.body.as_deref())?;
    let from = args
        .from
        .as_deref()
        .unwrap_or(&ctx.repo.hub_name)
        .to_string();
    let outcome = deliver_to_worker(&ctx, &worktree, &from, &args.subject, &body, args.wake())?;

    if args.quiet {
        return Ok(());
    }
    println!("wrote {}", outcome.path.display());
    match outcome.reached {
        Reached::Woken => println!("Woke the worker."),
        Reached::Running {
            wake: NotWoken::NotNeeded,
        } => {
            println!(
                "The worker is running; wake skipped (no action needed). It will read this the next time it checks its outbox."
            )
        }
        Reached::Running {
            wake: NotWoken::Held { why },
        } => {
            println!(
                "The worker is running; it will read this the next time it checks its outbox."
            );
            if let Some(note) = why {
                println!("{}", wake_note_sentence(&note));
            }
        }
        Reached::NotRunning => {
            println!("The worker is not running; it will read this the next time it starts.")
        }
    }
    Ok(())
}
