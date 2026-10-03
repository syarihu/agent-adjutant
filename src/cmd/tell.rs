use super::*;

pub struct TellArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub worktree: &'a str,
    pub subject: &'a str,
    pub body: Option<&'a str>,
    pub from: Option<&'a str>,
    pub quiet: bool,
    pub wake: Option<bool>,
}

pub fn tell(args: &TellArgs<'_>) -> Result<(), String> {
    let ctx = context(args.repo, args.hub)?;
    let worktree = config::expand_home(args.worktree);
    if !worktree.is_dir() {
        return Err(format!("no such worktree: {}", worktree.display()));
    }
    let body = read_body(args.body)?;
    let from = args.from.unwrap_or(&ctx.repo.hub_name).to_string();
    let Told {
        path,
        present,
        woken,
        wake_needed,
        wake_note,
    } = deliver_to_worker(&ctx, &worktree, &from, args.subject, &body, args.wake)?;

    if args.quiet {
        return Ok(());
    }
    println!("wrote {}", path.display());
    match (present, woken, wake_needed) {
        (true, true, _) => println!("Woke the worker."),
        (true, false, false) => {
            println!(
                "The worker is running; wake skipped (no action needed). It will read this the next time it checks its outbox."
            )
        }
        (true, false, true) => {
            println!(
                "The worker is running; it will read this the next time it checks its outbox."
            );
            if let Some(note) = wake_note {
                println!("{}", wake_note_sentence(&note));
            }
        }
        (false, _, _) => {
            println!("The worker is not running; it will read this the next time it starts.")
        }
    }
    Ok(())
}
