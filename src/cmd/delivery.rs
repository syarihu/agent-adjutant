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

fn has_bracketed_tag(first_line: &str, tag: &str) -> bool {
    let lower = first_line.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix(tag) {
        rest.starts_with(' ') || rest.starts_with(':') || rest.starts_with(']')
    } else {
        false
    }
}

/// Whether a message left for a worker requires waking it.
///
/// A worker tab is woken only when there is something it has to act on: a question asked
/// back by the hub, a decision on a gate it opened, or a link to a task. The last is woken
/// because a session that was started with no task is idle at its prompt and would never
/// look at its outbox on its own. Notices (`[ack]`, issue filed, etc.)
/// are left in the outbox for the worker to read the next time it checks; waking on a notice
/// risks typing the wake line into an interactive prompt or question the person is looking at.
pub fn should_wake_worker(subject: &str) -> bool {
    let first_line = subject.lines().next().unwrap_or("").trim();
    has_bracketed_tag(first_line, "[question")
        || first_line.starts_with("[質問")
        || has_bracketed_tag(first_line, "[gate")
        || has_bracketed_tag(first_line, "[linked")
}

/// Whether a message delivered to the hub requires waking it.
///
/// A hub tab is woken for reports, answers, done notices, task requests, next triggers,
/// gate decisions, Jules updates, and any actionable custom message kinds. Messages a hub
/// leaves for itself (`kind: question`, `kind: needs-user`, or any message where `from`
/// is the hub itself), plain notices, and acknowledgements do not wake the hub.
pub fn should_wake_hub(from: &str, hub_name: &str, kind: &str, subject: &str) -> bool {
    if !hub_name.is_empty() && from == hub_name {
        return false;
    }
    let kind = if kind.trim().is_empty() {
        "report"
    } else {
        kind.trim()
    };
    if kind == "question" || kind == "needs-user" || kind == "ack" || kind == "notice" {
        return false;
    }
    let first_line = subject.lines().next().unwrap_or("").trim();
    if has_bracketed_tag(first_line, "[ack") {
        return false;
    }
    true
}

/// What became of a message handed to the hub.
pub struct Delivered {
    pub delivery: messaging::Delivery,
    pub woken: bool,
    pub wake_needed: bool,
    /// Why the wake did not happen, when one was tried: what the receiver's screen was
    /// showing, or what went wrong. `None` when it was woken or there was nothing to try.
    pub wake_note: Option<String>,
}

/// The agent a wake will find on the other end, which decides how its screen is read.
///
/// Decided from the receiver's runner alone. `ADJUTANT_AGENT` is the setting of whichever
/// process is sending, and it says nothing about the session being woken: an agy worker
/// sending to a Claude hub would read the hub's screen with agy's table. And stricter than
/// `prompts::resolve_agent`, which falls back to Claude for anything it does not know: a
/// procedure in the wrong dialect is a wording problem, but a screen read with the wrong
/// agent's table is never recognised and the wake would never be typed. So only a runner that
/// is Claude Code or agy is read; any other custom runner is typed into without looking.
pub(crate) fn wake_agent(runner: Option<&str>) -> crate::prompts::Agent {
    use crate::prompts::Agent;
    let Some(runner) = runner else {
        return Agent::Claude;
    };
    // The program the line runs, past `env` and `KEY=VALUE` words: a runner is often written
    // `env CLAUDE_CONFIG_DIR=… claude --resume {sessionId}`.
    match crate::runner::agent_from_runner(runner).as_str() {
        "claude" => Agent::Claude,
        "agy" => Agent::Agy,
        _ => Agent::Generic,
    }
}

/// A wake note as a sentence for the person at the terminal.
pub(crate) fn wake_note_sentence(note: &str) -> String {
    let mut chars = note.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

/// What `terminal::wake` came to, as the pair the callers keep: whether the session was
/// woken, and, when the screen is what stopped it, the reason. Any other failure is left
/// unsaid, as it always was.
fn woken_and_why(tried: Result<terminal::Performed, String>) -> (bool, Option<String>) {
    match tried {
        Ok(done) if done.ran => (true, None),
        Ok(done) if done.screen => (false, Some(done.description)),
        _ => (false, None),
    }
}

/// Leave a message for the hub, poke its tab if needed, and tell the person.
///
/// Shared by `adj send` and by the dashboard's hand-over, which is the whole reason it is a
/// function: the three steps are one rule, and a second copy of it is a second set of
/// conditions about when to wake and when to notify — drifting from the day it is written.
pub fn deliver_to_hub(ctx: &Context, message: &Message) -> Result<Delivered, String> {
    deliver_to_hub_with_wake(ctx, message, true, None)
}

/// `deliver_to_hub`, choosing whether to tell the person. Not when they are the sender: a
/// decision made on the board a moment ago does not need a banner to say it was made.
pub fn deliver_to_hub_announcing(
    ctx: &Context,
    message: &Message,
    announce: bool,
) -> Result<Delivered, String> {
    deliver_to_hub_with_wake(ctx, message, announce, None)
}

/// `deliver_to_hub`, choosing whether to announce and optionally overriding the wake decision.
pub fn deliver_to_hub_with_wake(
    ctx: &Context,
    message: &Message,
    announce: bool,
    wake: Option<bool>,
) -> Result<Delivered, String> {
    Ok(post_to_hub_with_wake(ctx, message, wake)?.follow_up(ctx, announce))
}

/// A message written into the hub's inbox, its two follow-ups not yet run.
pub struct Posted {
    subject: String,
    delivery: messaging::Delivery,
    wake_needed: bool,
}

/// The first half of `deliver_to_hub`: the message is in the inbox once this returns.
///
/// Split off for a caller holding a lock: writing a file is quick, while waking the hub and
/// notifying run commands of the person's choosing, which can hang. Such a caller posts under
/// the lock and follows up after letting it go.
pub fn post_to_hub(ctx: &Context, message: &Message) -> Result<Posted, String> {
    post_to_hub_with_wake(ctx, message, None)
}

pub fn post_to_hub_with_wake(
    ctx: &Context,
    message: &Message,
    wake: Option<bool>,
) -> Result<Posted, String> {
    let subject =
        messaging::header_value(&messaging::render_message(message), "subject").unwrap_or_default();
    let delivery = messaging::send(&ctx.repo.slug, &ctx.repo.hub_name, message)?;
    let wake_needed = wake.unwrap_or_else(|| {
        should_wake_hub(&message.from, &ctx.repo.hub_name, &message.kind, &subject)
    });
    Ok(Posted {
        subject,
        delivery,
        wake_needed,
    })
}

impl Posted {
    /// The second half: poke the hub if it is there and waking is needed, and tell the person when `announce`.
    pub fn follow_up(self, ctx: &Context, announce: bool) -> Delivered {
        let Posted {
            subject,
            delivery,
            wake_needed,
        } = self;

        let (woken, wake_note) = if wake_needed {
            match (
                delivery.present,
                messaging::hub_status(&ctx.repo.slug, &ctx.repo.hub_name).pid,
            ) {
                (true, Some(pid)) => woken_and_why(terminal::wake(
                    &ctx.settings.terminal,
                    &ctx.settings.hub_wake,
                    pid,
                    &subject,
                    terminal::HUB_WAKE_LINE,
                    wake_agent(ctx.settings.hub_runner.as_deref()),
                    false,
                )),
                _ => (false, None),
            }
        } else {
            (false, None)
        };

        if announce
            && wake_needed
            && let Some(command) =
                notify::repo_command(&ctx.settings.notification, &ctx.repo, &subject)
        {
            let _ = terminal::run_shell(&command);
        }
        Delivered {
            delivery,
            woken,
            wake_needed,
            wake_note,
        }
    }
}

pub fn send(args: &SendArgs<'_>) -> Result<(), String> {
    let ctx = context(args.repo, args.hub)?;
    let body = read_body(args.body)?;
    let message = Message {
        from: args.from.unwrap_or("unknown").to_string(),
        // Where this is being sent from, taken from the same directory the repository was
        // resolved in rather than from anything the sender says about itself.
        worktree: repo::current_worktree(None),
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

/// Leave a message for the worker in a worktree, and poke it if it is sitting there.
/// What became of a message left for a worker.
pub struct Told {
    pub path: std::path::PathBuf,
    pub present: bool,
    pub woken: bool,
    pub wake_needed: bool,
    /// As `Delivered::wake_note`.
    pub wake_note: Option<String>,
}

/// Append to a worktree's outbox, poke the worker sitting in it if waking is needed,
/// and tell the person when poking was not possible.
///
/// Shared by `adj tell` and by a gate's answer, which is the point: both are the hub-to-
/// worker direction, and the rule about when to wake and when to notify is one rule. A
/// worker that was woken reads the message itself, so the notification is what happens
/// *instead* — unlike the other direction, where the hub is unattended and the person is
/// told either way.
pub fn deliver_to_worker(
    ctx: &Context,
    worktree: &std::path::Path,
    from: &str,
    subject: &str,
    body: &str,
    wake: Option<bool>,
) -> Result<Told, String> {
    let path = messaging::tell(worktree, from, subject, body)?;
    let status = messaging::worker_status(worktree);
    let wake_needed = wake.unwrap_or_else(|| should_wake_worker(subject));
    let (woken, wake_note) = if wake_needed {
        match (status.present, status.pid) {
            (true, Some(pid)) => woken_and_why(terminal::wake(
                &ctx.settings.terminal,
                &ctx.settings.worker_wake,
                pid,
                subject,
                terminal::WORKER_WAKE_LINE,
                wake_agent(ctx.settings.agent_runner.as_deref()),
                false,
            )),
            _ => (false, None),
        }
    } else {
        (false, None)
    };
    if wake_needed
        && !woken
        && let Some(command) = notify::repo_command(&ctx.settings.notification, &ctx.repo, subject)
    {
        let _ = terminal::run_shell(&command);
    }
    Ok(Told {
        path,
        present: status.present,
        woken,
        wake_needed,
        wake_note,
    })
}
