use super::*;

// Moved to `mail`; re-exported until #331 so that `cmd::deliver_to_hub` and the rest still resolve.
pub(crate) use crate::mail::wake_agent;
pub use crate::mail::{
    Delivered, Told, deliver_to_hub, deliver_to_hub_announcing, deliver_to_hub_with_wake,
    deliver_to_worker, post_to_hub,
};

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
    let dir = messaging::inbox_dir(&info.slug);
    if args.path_only {
        // A caller asking for the path is about to write into it, so hand back a directory
        // that exists.
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        println!("{}", dir.display());
        return Ok(());
    }
    if let Some(name) = args.read {
        print!("{}", messaging::read(&info.slug, name)?);
        return Ok(());
    }
    if let Some(name) = args.ack {
        let moved = messaging::ack(&info.slug, name)?;
        println!("filed {} ({})", name, moved.display());
        return Ok(());
    }

    let entries = messaging::list(&info.slug);
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
    let Delivered {
        delivery,
        woken,
        wake_needed,
        wake_note,
    } = deliver_to_hub_with_wake(&ctx, &message, true, args.wake)?;

    if args.quiet {
        return Ok(());
    }
    println!(
        "delivered to {}: {}",
        ctx.repo.hub_name,
        delivery.path.display()
    );
    match (delivery.present, woken, wake_needed) {
        (true, true, _) => println!("Woke the hub; it will pick this up."),
        (true, false, false) => {
            println!(
                "The hub is running; wake skipped (no action needed). It will pick this up the next time it checks its inbox."
            )
        }
        (true, false, true) => {
            println!("The hub is running; it will pick this up the next time it checks its inbox.");
            if let Some(note) = wake_note {
                println!("{}", wake_note_sentence(&note));
            }
        }
        (false, _, _) => {
            println!(
                "The hub is not running. Left in its inbox; it will be picked up the next time it starts."
            )
        }
    }
    Ok(())
}
