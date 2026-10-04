use super::*;

// ── pending ──────────────────────────────────────────────────────────

pub struct PendingArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub path_only: bool,
    pub limit: usize,
    pub as_json: bool,
    pub read: Option<&'a str>,
    pub ack: Option<&'a str>,
}

pub fn pending(args: &PendingArgs<'_>) -> Result<(), String> {
    let info = resolve(args.repo, args.hub)?;
    let root = crate::registry::state_root(Some(std::path::Path::new(&info.main)));
    if args.path_only {
        println!("{}", mail::open_inbox(&root, &info.slug)?.display());
        return Ok(());
    }
    if let Some(name) = args.read {
        print!("{}", mail::read(&root, &info.slug, name)?);
        return Ok(());
    }
    if let Some(name) = args.ack {
        let moved = mail::ack(&root, &info.slug, name)?;
        println!("filed {} ({})", name, moved.display());
        return Ok(());
    }

    let mail::Pending {
        dir,
        messages: entries,
    } = mail::pending(&root, &info.slug);
    if args.as_json {
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

pub struct SendArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub from: Option<&'a str>,
    pub kind: &'a str,
    pub subject: Option<&'a str>,
    pub body: Option<&'a str>,
    pub quiet: bool,
    pub wake: Option<bool>,
}

/// A wake note as a sentence for the person at the terminal.
pub(crate) fn wake_note_sentence(note: &str) -> String {
    let mut chars = note.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

pub fn send(args: &SendArgs<'_>) -> Result<(), String> {
    let ctx = context(args.repo, args.hub)?;
    let body = read_body(args.body)?;
    let message = Message {
        from: args.from.unwrap_or("unknown").to_string(),
        // Where this is being sent from, taken from the same directory the repository was
        // resolved in rather than from anything the sender says about itself.
        worktree: identity::current_worktree(None),
        kind: args.kind.to_string(),
        subject: args.subject.unwrap_or("").to_string(),
        body,
    };
    let outcome = deliver_to_hub_with_wake(&ctx, &message, true, args.wake)?;

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
