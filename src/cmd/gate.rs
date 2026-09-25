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

pub fn answered_dir(ctx: &Context) -> PathBuf {
    gate::answered_dir(&messaging::state_dir(), &ctx.repo.slug)
}

pub fn records_dir(ctx: &Context) -> PathBuf {
    gate::records_dir(&messaging::state_dir(), &ctx.repo.slug)
}

/// A gate by id, open or kept as a record. Open first: that is what an id usually names, and
/// a record's id cannot be an open gate's (see `gate::claim_record_id`).
fn find(ctx: &Context, id: &str) -> Result<Gate, String> {
    // Only a missing file falls through: one that is there and broken says so, rather than
    // reading as an id that does not exist.
    if gate::path_of(&dir(ctx), id).exists() {
        return gate::load(&dir(ctx), id);
    }
    if gate::path_of(&records_dir(ctx), id).exists() {
        return gate::load(&records_dir(ctx), id);
    }
    Err(format!("no open gate or record: {id}"))
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
///
/// With `"wait": false` the gate is kept as a record instead: written beside the open queue
/// rather than in it, so it asks nobody for anything, and the caller goes on with its work.
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

    // A dispatch gate asks whether to start a task, and its answer is acted on by reading the
    // task out of the message. Without one the hub would be told "approve" and not what.
    if kind == Kind::Dispatch
        && payload
            .get("task")
            .and_then(Value::as_str)
            .is_none_or(|t| t.trim().is_empty())
    {
        return Err("a dispatch gate needs the task it asks about".to_string());
    }

    let wait = match payload.get("wait") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(wait)) => *wait,
        Some(_) => return Err("wait must be true or false".to_string()),
    };
    if !wait && !kind.can_be_recorded() {
        return Err(format!(
            "a {} gate cannot be kept as a record; only diff and verify can",
            kind.as_str()
        ));
    }
    if payload
        .get("stoppedBy")
        .is_some_and(|v| !v.is_null() && !v.is_array())
    {
        return Err("stoppedBy must be a list of rules".to_string());
    }
    // A diff or verify gate that waits does so because a rule fired, and the board shows
    // which. One that names none leaves a person looking at a stop nobody can explain.
    if wait
        && kind.can_be_recorded()
        && payload
            .get("stoppedBy")
            .and_then(Value::as_array)
            .is_none_or(|rules| rules.is_empty())
    {
        return Err(format!(
            "a waiting {} gate needs stoppedBy; keep it as a record with \"wait\": false if no rule stopped it",
            kind.as_str()
        ));
    }
    // Why a gate stops only means something where it could have been a record instead, and
    // a record that says why it stopped the worker is one of the two statements being false.
    if payload
        .get("stoppedBy")
        .and_then(Value::as_array)
        .is_some_and(|rules| !rules.is_empty())
    {
        if !kind.can_be_recorded() {
            return Err(format!(
                "a {} gate always waits; stoppedBy is for diff and verify",
                kind.as_str()
            ));
        }
        if !wait {
            return Err("a record does not stop the worker; drop stoppedBy or wait".to_string());
        }
        // Read here rather than left to serde below, which runs after the id is claimed and
        // would leave an empty gate file behind for an unknown rule.
        for rule in &payload["stoppedBy"].as_array().cloned().unwrap_or_default() {
            let parsed: gate::StopRule = serde_json::from_value(rule.clone())
                .map_err(|_| format!("no such stop rule: {rule}"))?;
            if !parsed.applies_to(kind) {
                return Err(format!(
                    "{} cannot stop a {} gate",
                    parsed.as_str(),
                    kind.as_str()
                ));
            }
        }
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
    let home = if wait { dir(ctx) } else { records_dir(ctx) };
    let id = if wait {
        gate::claim_id(&home, &stamp, kind)?
    } else {
        gate::claim_record_id(&home, &stamp, kind)?
    };

    let mut value = payload.clone();
    let fields = value.as_object_mut().ok_or("expected an object")?;
    fields.insert("id".to_string(), json!(id));
    fields.insert("worktree".to_string(), json!(worktree));
    fields.insert("openedAt".to_string(), json!(stamp));
    fields
        .entry("options")
        .or_insert(json!(kind.default_options()));
    fields.insert("wait".to_string(), json!(wait));
    // A record's answers are appended by whoever answers it, never brought in with it.
    fields.remove("answers");
    let gate: Gate = serde_json::from_value(value).map_err(|e| format!("bad gate: {e}"))?;

    gate::save(&home, &gate)?;
    Ok((gate, super::serve::running(&ctx.repo.slug).is_some()))
}

/// Hand the ball back. The gate leaves the queue and the answer lands in the outbox.
///
/// A record can be answered too, with `changes` only: a person sending back a review the
/// worker had already moved past. The answer reaches the worker the same way, and the record
/// stays where it is with the answer appended, so what was sent back is still readable.
pub fn answer(
    ctx: &Context,
    id: &str,
    decision: &str,
    choice: Option<&str>,
    comment: Option<&str>,
) -> Result<(Gate, super::Told), String> {
    let mut gate = find(ctx, id)?;
    // Nothing else means anything to a worker that is not waiting: an approval of a record
    // would wake it to be told to carry on with what it is already doing.
    if !gate.wait && (decision != "changes" || choice.is_some()) {
        return Err(format!(
            "a record can only be answered with changes, not {decision}{}",
            if choice.is_some() {
                " and a choice"
            } else {
                ""
            }
        ));
    }
    if let Some(choice) = choice
        && !gate.choices.iter().any(|c| c.id == choice)
    {
        return Err(format!("no such choice on this gate: {choice}"));
    }

    let subject = gate::answer_subject(&gate, decision);
    let body = gate::answer_body(&gate, decision, choice, comment);
    let told = if gate.kind.answered_by_hub() {
        // `gate` rather than `answer`: the hub pairs an `answer` with a question it asked a
        // worker, and this is a person deciding on something the hub put on the board.
        let message = crate::messaging::Message {
            from: "dashboard".to_string(),
            // None, for the reason `task::hand_over` gives.
            worktree: None,
            kind: "gate".to_string(),
            subject: subject.clone(),
            body: body.clone(),
        };
        let handed = super::deliver_to_hub_announcing(ctx, &message, false)?;
        super::Told {
            path: handed.delivery.path,
            present: handed.delivery.present,
            woken: handed.woken,
        }
    } else {
        super::deliver_to_worker(
            ctx,
            std::path::Path::new(&gate.worktree),
            &ctx.repo.hub_name,
            &subject,
            &body,
        )?
    };

    let comment = comment
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    if !gate.wait {
        let at = stamp();
        // Read again rather than appended to the copy loaded before delivery: the board and
        // `adj gate answer` can send the same record back at once, and the second write would
        // otherwise drop the first answer the worker has already been given.
        let mut gate = gate::load(&records_dir(ctx), id).unwrap_or(gate);
        gate.answers.push(gate::Answer {
            decision: decision.to_string(),
            comment,
            answered_at: at.clone(),
        });
        gate::save(&records_dir(ctx), &gate)?;
        note_answered(ctx, gate.task.as_deref(), &at);
        return Ok((gate, told));
    }

    // Archived after delivery, not before: if the outbox could not be written the gate is
    // still open, and the person can try again rather than losing what they were shown.
    gate.decision = Some(decision.to_string());
    gate.choice = choice.map(str::to_string);
    gate.comment = comment;
    let at = stamp();
    gate.answered_at = Some(at.clone());
    gate::archive(&dir(ctx), &answered_dir(ctx), &gate)?;
    note_answered(ctx, gate.task.as_deref(), &at);
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
    let at = stamp();
    gate.answered_at = Some(at.clone());
    gate::archive(&dir(ctx), &answered_dir(ctx), &gate)?;
    note_answered(ctx, gate.task.as_deref(), &at);
    Ok(gate)
}

/// Write the answer's time onto the gate's task, for the board's stuck badge. Best effort:
/// the answer has been delivered and archived by now, and a task record that is missing or
/// unwritable must not turn that into a failure.
fn note_answered(ctx: &Context, task: Option<&str>, at: &str) {
    let Some(id) = task else {
        return;
    };
    let dir = super::task::dir(ctx);
    // Under the task's lock, like `task::update`: a whole-record write racing another would
    // undo whichever landed first.
    let Ok(_lock) = super::task::lock_task(ctx, id) else {
        return;
    };
    if let Ok(mut task) = crate::task::load(&dir, id) {
        task.gate_answered_at = Some(at.to_string());
        let _ = crate::task::save(&dir, &task);
    }
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
        println!("{}", open_json(&gate, served));
        return Ok(());
    }
    println!("{} — {}", gate.id, gate.title);
    if !gate.wait {
        println!("{RECORDED}");
    } else if served && gate.kind.answered_by_hub() {
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

/// What a caller that kept a record is told. Said, because the procedure it follows has
/// always ended its turn after opening a gate.
const RECORDED: &str = "Kept as a record: nobody is asked to answer it. Do not wait; go on \
                        with your work. If a person sends it back, the answer arrives in \
                        `adj outbox`.";

/// `adj gate open --json`, and the MCP tool's answer: the same object, so the procedure can
/// branch on it the same way whichever it used.
pub fn open_json(gate: &Gate, served: bool) -> Value {
    let mut out = json!({ "gate": gate, "server": if served { "up" } else { "down" } });
    if !gate.wait {
        out["wait"] = json!(false);
        out["note"] = json!(RECORDED);
    }
    out
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
    let gate = find(&ctx, id)?;
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
    if gate.kind.answered_by_hub() {
        match (told.present, told.woken) {
            (true, true) => println!("Woke the hub."),
            (true, false) => {
                println!("The hub is running; it will read this the next time it checks its inbox.")
            }
            (false, _) => println!(
                "The hub is not running. The answer waits in its inbox for the next time it starts."
            ),
        }
        return Ok(());
    }
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
