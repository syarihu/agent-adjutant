//! `adj gate` — opening one, and answering it.
//!
//! Opening is how an agent hands the ball over: it writes the payload down and ends its
//! turn. Answering is how it comes back, and it is deliberately not a channel of its own —
//! the answer goes into the worktree's outbox and pokes the worker exactly as `adj tell`
//! does, because that path already survives the worker having died in the meantime.

use serde_json::{Value, json};

use super::Context;
use crate::gate::{self, Gate, Kind};
use crate::messaging;

use std::path::PathBuf;

pub fn dir(ctx: &Context) -> PathBuf {
    gate::dir(&messaging::state_dir(), &ctx.repo.slug)
}

fn answered_dir(ctx: &Context) -> PathBuf {
    gate::answered_dir(&messaging::state_dir(), &ctx.repo.slug)
}

fn stamp() -> String {
    messaging::utc_stamp(messaging::now_secs())
}

/// Write a gate down and say whether anybody is there to see it.
///
/// The second half of that answer is the whole reason this reports rather than just
/// succeeding: with no dashboard running, a gate is a message into a directory nobody
/// opens, and an agent that waited on one would wait for ever. The procedure branches on
/// it and falls back to asking in its own tab.
pub fn open(ctx: &Context, payload: &Value) -> Result<(Gate, bool), String> {
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("a gate needs a kind")?;
    let kind: Kind =
        serde_json::from_value(json!(kind)).map_err(|_| format!("no such gate kind: {kind}"))?;
    // Checked before deserialising so the error names the field, rather than arriving as
    // a serde message about a struct the caller never saw.
    if payload
        .get("title")
        .and_then(Value::as_str)
        .is_none_or(|t| t.trim().is_empty())
    {
        return Err("a gate needs a title".to_string());
    }

    // The payload may name the worktree; otherwise it is derived from where the caller
    // stands. This is the address the answer is delivered to, so an agent that mistypes it
    // waits on an outbox nobody writes to.
    let worktree = match payload.get("worktree").and_then(Value::as_str) {
        Some(path) => path.to_string(),
        None => crate::repo::current_worktree(None)
            .ok_or("not inside a worktree, and no --worktree was given")?,
    };

    let stamp = stamp();
    let id = gate::claim_id(&dir(ctx), &stamp, kind)?;

    let mut value = payload.clone();
    let fields = value.as_object_mut().ok_or("expected an object")?;
    fields.insert("id".to_string(), json!(id));
    fields.insert("worktree".to_string(), json!(worktree));
    fields.insert("openedAt".to_string(), json!(stamp));
    fields
        .entry("options")
        .or_insert(json!(kind.default_options()));
    let gate: Gate = serde_json::from_value(value).map_err(|e| format!("bad gate: {e}"))?;

    gate::save(&dir(ctx), &gate)?;
    Ok((gate, super::serve::running(&ctx.repo.slug).is_some()))
}

/// Hand the ball back. The gate leaves the queue and the answer lands in the outbox.
pub fn answer(
    ctx: &Context,
    id: &str,
    decision: &str,
    choice: Option<&str>,
    comment: Option<&str>,
) -> Result<(Gate, super::Told), String> {
    let mut gate = gate::load(&dir(ctx), id)?;
    if let Some(choice) = choice
        && !gate.choices.iter().any(|c| c.id == choice)
    {
        return Err(format!("no such choice on this gate: {choice}"));
    }

    let subject = gate::answer_subject(&gate, decision);
    let body = gate::answer_body(&gate, decision, choice, comment);
    let told = super::deliver_to_worker(
        ctx,
        std::path::Path::new(&gate.worktree),
        &ctx.repo.hub_name,
        &subject,
        &body,
    )?;

    // Archived after delivery, not before: if the outbox could not be written the gate is
    // still open, and the person can try again rather than losing what they were shown.
    gate.decision = Some(decision.to_string());
    gate.choice = choice.map(str::to_string);
    gate.comment = comment
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    gate.answered_at = Some(stamp());
    gate::archive(&dir(ctx), &answered_dir(ctx), &gate)?;
    Ok((gate, told))
}

/// Archive a gate without delivering an answer to the worker's outbox.
///
/// Used when the conversation happened directly in a terminal tab, or when a gate was
/// rendered moot. It leaves the gate in `answered/` with decision "closed" so the record
/// survives, but skips the delivery and the wake.
pub fn close(ctx: &Context, id: &str, comment: Option<&str>) -> Result<Gate, String> {
    let mut gate = gate::load(&dir(ctx), id)?;
    gate.decision = Some("closed".to_string());
    gate.comment = comment
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    gate.answered_at = Some(stamp());
    gate::archive(&dir(ctx), &answered_dir(ctx), &gate)?;
    Ok(gate)
}

// ── the subcommands ──────────────────────────────────────────────────

pub fn open_cmd(
    repo: Option<&str>,
    hub: Option<&str>,
    file: Option<&str>,
    as_json: bool,
) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    // The payload arrives whole rather than as a dozen flags: every interesting field is
    // multi-line prose, and a shell quoting three paragraphs into `--focus` is a worse
    // interface than a heredoc.
    let raw = match file {
        Some(path) => {
            let path = crate::config::expand_home(path);
            std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        }
        // `read_body(None)` is the stdin path, which is the one a heredoc uses.
        None => super::read_body(None)?,
    };
    let payload: Value = serde_json::from_str(&raw).map_err(|e| format!("bad JSON: {e}"))?;
    let (gate, served) = open(&ctx, &payload)?;

    if as_json {
        println!(
            "{}",
            json!({ "gate": gate, "server": if served { "up" } else { "down" } })
        );
        return Ok(());
    }
    println!("{} — {}", gate.id, gate.title);
    if served {
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

pub fn list(repo: Option<&str>, hub: Option<&str>, as_json: bool) -> Result<(), String> {
    let ctx = super::context(repo, hub)?;
    let gates = gate::list(&dir(&ctx));
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
    let ctx = super::context(repo, hub)?;
    let gate = gate::load(&dir(&ctx), id)?;
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
    let ctx = super::context(args.repo, args.hub)?;
    let (gate, told) = answer(&ctx, args.id, args.decision, args.choice, args.comment)?;
    if args.json {
        println!(
            "{}",
            json!({
                "gate": gate,
                "present": told.present,
                "woken": told.woken,
                "path": told.path.display().to_string(),
            })
        );
        return Ok(());
    }
    println!("{} → {}", gate.id, args.decision);
    println!("wrote {}", told.path.display());
    match (told.present, told.woken) {
        (true, true) => println!("Woke the worker."),
        (true, false) => {
            println!("The worker is running; it will read this the next time it checks its outbox.")
        }
        (false, _) => println!(
            "The worker is not running. The answer waits in that worktree's outbox for \
             whoever starts one there next."
        ),
    }
    Ok(())
}

pub struct CloseArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub id: &'a str,
    pub comment: Option<&'a str>,
    pub json: bool,
}

pub fn close_cmd(args: &CloseArgs<'_>) -> Result<(), String> {
    let ctx = super::context(args.repo, args.hub)?;
    let gate = close(&ctx, args.id, args.comment)?;
    if args.json {
        println!("{}", json!({ "gate": gate, "closed": true }));
        return Ok(());
    }
    println!("{} → closed", gate.id);
    Ok(())
}
