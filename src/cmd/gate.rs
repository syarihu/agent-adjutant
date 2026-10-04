//! `adj gate` — opening one, and answering it.
//!
//! Opening is how an agent hands the ball over: it writes the payload down and ends its
//! turn. Answering is how it comes back, and it is deliberately not a channel of its own —
//! the answer goes into the worktree's outbox and pokes the worker exactly as `adj tell`
//! does, because that path already survives the worker having died in the meantime.

use serde_json::{Value, json};

use crate::gate::{self, Gate, Shelf};
use crate::registry::Context;
// For the callers that still reach them through `cmd` (cmd/mod.rs, the board's handlers and state) until #363.
pub use crate::gate::{answer, close, close_resumed, open};

// ── the subcommands ──────────────────────────────────────────────────

pub fn open_cmd(
    repo: Option<&str>,
    hub: Option<&str>,
    file: Option<&str>,
    body_file: Option<&str>,
    as_json: bool,
) -> Result<(), String> {
    let ctx = crate::registry::context(repo, hub)?;
    // The payload arrives whole rather than as a dozen flags: every interesting field is
    // multi-line prose, and a shell quoting three paragraphs into `--focus` is a worse
    // interface than a heredoc.
    let raw = match file {
        Some(path) => {
            let path = crate::infra::paths::expand_home(path);
            std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        }
        // `read_body(None)` is the stdin path, which is the one a heredoc uses.
        None => super::read_body(None)?,
    };
    let mut payload: Value = serde_json::from_str(&raw).map_err(|e| format!("bad JSON: {e}"))?;
    // The body read from a file as it is, so the one who opens the gate need not read a long
    // report into its own context to quote it into JSON — the hub opening a plan a sub-agent
    // wrote is the case this is for, and what the person approves is that file, byte for byte.
    if let Some(path) = body_file {
        let path = crate::infra::paths::expand_home(path);
        let body = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        with_body(&mut payload, body)?;
    }
    let (gate, served) = open(&ctx, &payload)?;

    if as_json {
        println!("{}", open_json(&ctx, &gate, served));
        return Ok(());
    }
    println!("{} — {}", gate.id, gate.title);
    if !gate.wait {
        println!("{RECORDED}");
    } else if served && gate.answered_by_hub() {
        println!("Waiting on the board. The answer arrives in your inbox as `kind: gate`.");
    } else if served {
        println!("Waiting on the board. Read `adj outbox` when you are woken.");
    } else {
        // Not an error: the gate is written either way, and the caller decides what to do
        // with a queue nobody is watching.
        println!(
            "No dashboard is running, so nobody will see this. Ask in this tab instead \
             (the gate is written, and stays)."
        );
    }
    Ok(())
}

/// Put a body read from a file into a payload. Refused when the payload has one of its own:
/// which of the two was meant cannot be told, and the one dropped is what a person expected
/// to be shown.
fn with_body(payload: &mut Value, body: String) -> Result<(), String> {
    let fields = payload.as_object_mut().ok_or("expected an object")?;
    if fields.get("body").is_some_and(|b| !b.is_null()) {
        return Err("the payload has a body already; drop it or --body-file".to_string());
    }
    fields.insert("body".to_string(), json!(body));
    Ok(())
}

/// What a caller that kept a record is told. Said, because the procedure it follows has
/// always ended its turn after opening a gate.
const RECORDED: &str = "Kept as a record: nobody is asked to answer it. Do not wait; go on \
                        with your work. If a person sends it back, the answer arrives in \
                        `adj outbox`.";

/// `adj gate open --json`, and the MCP tool's answer: the same object, so the procedure can
/// branch on it the same way whichever it used.
pub fn open_json(ctx: &Context, gate: &Gate, served: bool) -> Value {
    let mut out = json!({ "gate": gate, "server": if served { "up" } else { "down" } });
    if !gate.wait {
        out["wait"] = json!(false);
        out["note"] = json!(RECORDED);
    } else {
        let (wake, default_line) = if gate.answered_by_hub() {
            (
                &ctx.settings.hub_wake,
                crate::infra::terminal::HUB_WAKE_LINE,
            )
        } else {
            (
                &ctx.settings.worker_wake,
                crate::infra::terminal::WORKER_WAKE_LINE,
            )
        };
        if !wake.hook.is_off() {
            out["wakeLine"] = json!(wake.line_or(default_line));
            // Said only where it holds: the built-in tmux wake reads the screen and holds
            // its line back from a question, and a caller that knows that can end its turn
            // at an empty prompt instead of asking the same thing in the terminal too.
            if crate::mail::wake_looks_at_screen(&ctx.settings, gate.answered_by_hub()) {
                out["wakeChecksScreen"] = json!(true);
            }
        }
    }
    out
}

pub fn list(repo: Option<&str>, hub: Option<&str>, as_json: bool) -> Result<(), String> {
    let ctx = crate::registry::context(repo, hub)?;
    let gates = gate::list(&ctx.state, &ctx.repo.slug, Shelf::Open);
    if as_json {
        println!("{}", json!(gates));
        return Ok(());
    }
    if gates.is_empty() {
        println!("No gates are waiting for {}.", ctx.repo.nwo);
        return Ok(());
    }
    for gate in &gates {
        println!(
            "{:<9} {}  {}  {}",
            gate.kind.as_str(),
            gate.opened_at,
            gate.id,
            gate.title
        );
    }
    Ok(())
}

pub fn show(repo: Option<&str>, hub: Option<&str>, id: &str) -> Result<(), String> {
    let ctx = crate::registry::context(repo, hub)?;
    let gate = gate::get(&ctx.state, &ctx.repo.slug, id)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&gate).map_err(|e| e.to_string())?
    );
    Ok(())
}

pub struct AnswerArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub id: &'a str,
    pub decision: &'a str,
    pub choice: Option<&'a str>,
    pub comment: Option<&'a str>,
    pub json: bool,
}

pub fn answer_cmd(args: &AnswerArgs<'_>) -> Result<(), String> {
    let ctx = crate::registry::context(args.repo, args.hub)?;
    let (gate, told) = answer(&ctx, args.id, args.decision, args.choice, args.comment)?;
    if args.json {
        let mut out = json!({
            "gate": gate,
            "present": told.is_present(),
            "woken": told.was_woken(),
            "path": told.path.display().to_string(),
        });
        if let crate::mail::Reached::Running {
            wake: crate::mail::NotWoken::Held { why: Some(why) },
        } = &told.reached
        {
            out["wakeNote"] = json!(why);
        }
        println!("{out}");
        return Ok(());
    }
    println!("{} → {}", gate.id, args.decision);
    println!("wrote {}", told.path.display());
    if gate.answered_by_hub() {
        match &told.reached {
            crate::mail::Reached::Woken => println!("Woke the hub."),
            crate::mail::Reached::Running { wake } => {
                println!(
                    "The hub is running; it will read this the next time it checks its inbox."
                );
                if let crate::mail::NotWoken::Held { why: Some(note) } = wake {
                    println!("{}", super::wake_note_sentence(note));
                }
            }
            crate::mail::Reached::NotRunning => println!(
                "The hub is not running. The answer waits in its inbox for the next time it starts."
            ),
        }
        return Ok(());
    }
    match &told.reached {
        crate::mail::Reached::Woken => println!("Woke the worker."),
        crate::mail::Reached::Running { wake } => {
            println!(
                "The worker is running; it will read this the next time it checks its outbox."
            );
            if let crate::mail::NotWoken::Held { why: Some(note) } = wake {
                println!("{}", super::wake_note_sentence(note));
            }
        }
        crate::mail::Reached::NotRunning => println!(
            "The worker is not running. The answer waits in that worktree's outbox for \
             whoever starts one there next."
        ),
    }
    Ok(())
}

/// Arguments for `adj gate close`.
pub struct CloseArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub id: &'a str,
    pub comment: Option<&'a str>,
    /// The person answered in the worker's terminal.
    pub terminal: bool,
    pub json: bool,
}

/// `adj gate close`: archive an open gate from the command line.
pub fn close_cmd(args: &CloseArgs<'_>) -> Result<(), String> {
    let ctx = crate::registry::context(args.repo, args.hub)?;
    let gate = close(&ctx, args.id, args.comment, args.terminal)?;
    let on_board = gate.answered_on_board();
    if args.json {
        println!(
            "{}",
            json!({ "gate": gate, "closed": !on_board, "alreadyAnswered": on_board })
        );
        return Ok(());
    }
    if on_board {
        println!(
            "{} → already answered ({})",
            gate.id,
            gate.decision.as_deref().unwrap_or("unknown")
        );
    } else if gate.decision.as_deref() == Some(gate::TERMINAL) {
        println!("{} → answered in the terminal", gate.id);
    } else {
        println!("{} → closed", gate.id);
    }
    Ok(())
}
