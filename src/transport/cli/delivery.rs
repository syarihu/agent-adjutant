use super::args::{PendingArgs, SendArgs};
use super::*;

// ── pending ──────────────────────────────────────────────────────────

pub fn pending(args: &PendingArgs) -> Result<(), String> {
    let info = resolve(args.repo.as_deref(), args.hub.as_deref())?;
    let root = crate::registry::state_root(Some(std::path::Path::new(&info.main)));
    if args.path {
        println!("{}", mail::open_inbox(&root, &info.slug)?.display());
        return Ok(());
    }
    if let Some(name) = args.read.as_deref() {
        print!("{}", mail::read(&root, &info.slug, name)?);
        return Ok(());
    }
    if let Some(name) = args.ack.as_deref() {
        let moved = mail::ack(&root, &info.slug, name)?;
        println!("filed {} ({})", name, moved.display());
        return Ok(());
    }

    let mail::Pending {
        dir,
        messages: entries,
    } = mail::pending(&root, &info.slug);
    if args.json {
        let items: Vec<Value> = entries
            .iter()
            .map(|e| {
                json!({"name": e.name, "from": e.from, "worktree": e.worktree,
                       "kind": e.kind, "subject": e.subject})
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "hubName": info.hub_name,
                "dir": dir.to_string_lossy(),
                "count": items.len(),
                "messages": items,
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }
    // The path is printed even when nothing is waiting: an empty listing is the common case,
    // and it is also the one where the reader would otherwise have to guess the location.
    println!("dir: {}", dir.display());
    if entries.is_empty() {
        println!("(empty)");
        return Ok(());
    }
    for entry in entries.iter().take(args.limit) {
        println!(
            "{}  [{}] {} — {}",
            entry.name, entry.kind, entry.from, entry.subject
        );
    }
    if entries.len() > args.limit {
        println!("... and {} more", entries.len() - args.limit);
    }
    Ok(())
}

// ── send ─────────────────────────────────────────────────────────────

pub fn send(args: &SendArgs) -> Result<(), String> {
    let ctx = context(args.repo.as_deref(), args.hub.as_deref())?;
    let body = read_body(args.body.as_deref())?;
    let message = Message {
        from: args.from.as_deref().unwrap_or("unknown").to_string(),
        // Where this is being sent from, taken from the same directory the repository was
        // resolved in rather than from anything the sender says about itself.
        worktree: identity::current_worktree(None),
        kind: args.kind.clone(),
        subject: args.subject.as_deref().unwrap_or("").to_string(),
        body,
    };
    let outcome = deliver_to_hub_with_wake(&ctx, &message, true, args.wake())?;

    if args.quiet {
        return Ok(());
    }
    println!(
        "delivered to {}: {}",
        ctx.repo.hub_name,
        outcome.path.display()
    );
    match outcome.reached {
        Reached::Woken => println!("Woke the hub; it will pick this up."),
        Reached::Running {
            wake: NotWoken::NotNeeded,
        } => {
            println!(
                "The hub is running; wake skipped (no action needed). It will pick this up the next time it checks its inbox."
            )
        }
        Reached::Running {
            wake: NotWoken::Held { why },
        } => {
            println!("The hub is running; it will pick this up the next time it checks its inbox.");
            if let Some(note) = why {
                println!("{}", wake_note_sentence(&note));
            }
        }
        Reached::NotRunning => {
            println!(
                "The hub is not running. Left in its inbox; it will be picked up the next time it starts."
            )
        }
    }
    Ok(())
}
