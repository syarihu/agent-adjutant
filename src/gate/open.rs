//! Opening a gate, or keeping one as a record.

use super::store::{self, claim_id, save, stamp};
use super::*;

use crate::registry::Context;

/// Write a gate down and say whether anybody is there to see it.
///
/// The second half of that answer is the whole reason this reports rather than just
/// succeeding: with no dashboard running, a gate is a message into a directory nobody
/// opens, and an agent that waited on one would wait for ever. The procedure branches on
/// it and falls back to asking in its own tab.
///
/// With `"wait": false` the gate is kept as a record instead: written beside the open queue
/// rather than in it, so it asks nobody for anything, and the caller goes on with its work.
///
/// For a plan opened with a `task`, the plan is also written to the task's record (see
/// `write_plan`). The third part is the reason when that write failed: the gate stands either
/// way, so the failure is reported rather than returned as an error.
pub fn open(ctx: &Context, request: GateRequest) -> Result<(Gate, bool, Option<String>), String> {
    let kind = request.kind;
    let has_task = request
        .task
        .as_deref()
        .is_some_and(|t| !t.trim().is_empty());

    // A dispatch gate asks whether to start a task, and a relay gate whether to pass a task's
    // review comments on; the answer to either is acted on by reading the task out of the
    // message. Without one the hub would be told "approve" and not what.
    if matches!(kind, Kind::Dispatch | Kind::Relay) && !has_task {
        return Err(format!(
            "a {} gate needs the task it asks about",
            kind.as_str()
        ));
    }

    // Who waits on the answer, for a plan: the hub opens one for a task handed to Jules.
    if request.opened_by == Opener::Hub {
        // Every other kind's opener follows from the kind; only a plan is opened by either.
        if kind != Kind::Plan {
            return Err(format!(
                "openedBy is for a plan; a {} gate's opener follows from its kind",
                kind.as_str()
            ));
        }
        // The hub is told which task an answer is about by the message alone, as with a
        // dispatch gate: the gate is archived by the time it reads it.
        if !has_task {
            return Err("a plan the hub opens needs the task it is for".to_string());
        }
    }

    if !request.wait && !kind.can_be_recorded() {
        // A plan is the one exception, and only for a task whose Stop at says nothing waits.
        // Read off the task's own record rather than taken from the request: the worker is
        // the one asking to skip the wait, so its say-so cannot be what permits it.
        if kind == Kind::Plan {
            check_plan_record(ctx, &request)?;
        } else {
            return Err(format!(
                "a {} gate cannot be kept as a record; only diff, verify and, for a task that stops at none, plan can",
                kind.as_str()
            ));
        }
    }
    // A diff or verify gate that waits does so because a rule fired, and the board shows
    // which. One that names none leaves a person looking at a stop nobody can explain.
    if request.wait && kind.can_be_recorded() && request.stopped_by.is_empty() {
        return Err(format!(
            "a waiting {} gate needs stoppedBy; keep it as a record with \"wait\": false if no rule stopped it",
            kind.as_str()
        ));
    }
    // Why a gate stops only means something where it could have been a record instead, and
    // a record that says why it stopped the worker is one of the two statements being false.
    if !request.stopped_by.is_empty() {
        if !kind.can_be_recorded() {
            return Err(format!(
                "a {} gate has no stop rules; stoppedBy is for diff and verify",
                kind.as_str()
            ));
        }
        if !request.wait {
            return Err("a record does not stop the worker; drop stoppedBy or wait".to_string());
        }
        for rule in &request.stopped_by {
            if !rule.applies_to(kind) {
                return Err(format!(
                    "{} cannot stop a {} gate",
                    rule.as_str(),
                    kind.as_str()
                ));
            }
        }
    }

    check_content(&request)?;

    // The address the answer is delivered to, so an agent that mistypes it waits on an
    // outbox nobody writes to. The caller fills it in from where it stands when the payload
    // does not name one.
    let worktree = request
        .worktree
        .clone()
        .ok_or("not inside a worktree, and no --worktree was given")?;

    // Read and checked before an id is claimed, so a report that is refused leaves nothing
    // behind. The copy is written after, once there is an id to name it by.
    let report = match request.report.as_deref() {
        Some(given) => Some((report::read(&worktree, given)?, given.to_string())),
        None => None,
    };

    let stamp = stamp();
    let shelf = if request.wait {
        Shelf::Open
    } else {
        Shelf::Record
    };
    let id = claim_id(ctx, shelf, &stamp, kind)?;

    let GateRequest {
        kind,
        title,
        worktree: _,
        opened_by,
        task,
        wait,
        stopped_by,
        options,
        facts,
        focus,
        decided,
        unsure,
        body,
        run,
        diff,
        choices,
        rounds,
        problem,
        goal,
        steps,
        sections,
        quick,
        review_rounds,
        findings,
        commands,
        manual,
        report: _,
    } = request;
    let gate = Gate {
        id,
        kind,
        worktree,
        opened_by,
        task,
        title,
        facts,
        focus,
        decided,
        unsure,
        body,
        run,
        diff,
        choices,
        options: options.unwrap_or_else(|| kind.default_options()),
        rounds,
        problem,
        goal,
        steps,
        sections,
        quick,
        review_rounds,
        findings,
        commands,
        manual,
        report: report.as_ref().map(|(_, given)| file_name(given)),
        stopped_by,
        wait,
        opened_at: stamp,
        // Written when it is answered, never brought in with the request.
        decision: None,
        choice: None,
        comment: None,
        answered_at: None,
        answers: Vec::new(),
        extra: serde_json::Map::new(),
    };

    if let Some((html, _)) = &report
        && let Err(e) = report::write(ctx, &gate.id, html)
    {
        // The claimed file is empty and nobody has seen it; leaving it would be a gate that
        // cannot be read in the open queue.
        let _ = std::fs::remove_file(store::path_of(&store::dir_of(ctx, shelf), &gate.id));
        return Err(e);
    }
    if let Err(e) = save(ctx, shelf, &gate) {
        // A copy with no gate is a report nobody can open.
        if report.is_some() {
            report::remove(ctx, &gate.id);
        }
        return Err(e);
    }
    let warning = write_plan(ctx, &gate);
    Ok((
        gate,
        crate::registry::running(&ctx.state, &ctx.repo).is_some(),
        warning,
    ))
}

/// Write a plan gate's plan to its task's record, as the plan of record. Runs after the gate is
/// saved and with no gate lock held, so it takes only the task's lock. Best effort, as
/// `task::note_gate_answered` is: the gate is what the person is asked, and a task that is
/// missing or unwritable must not undo it. The failure comes back as a warning for the opener
/// to say. Nothing is written, and nothing said, for a gate with no task.
fn write_plan(ctx: &Context, gate: &Gate) -> Option<String> {
    if gate.kind != Kind::Plan {
        return None;
    }
    let id = gate.task.as_deref().filter(|t| !t.trim().is_empty())?;
    let patch = crate::task::TaskPatch {
        plan: Some(Some(crate::task::PlanInput {
            problem: gate.problem.clone(),
            goal: gate.goal.clone(),
            steps: gate
                .steps
                .iter()
                .map(|step| crate::task::PlanStep {
                    text: step.text.clone(),
                    detail: step.detail.clone(),
                    extra: serde_json::Map::new(),
                })
                .collect(),
            decided: gate.decided.clone(),
            unsure: gate.unsure.clone(),
            gate: Some(gate.id.clone()),
        })),
        ..Default::default()
    };
    crate::task::update(ctx, id, &patch, false).err()
}

/// What the board needs in a gate to show it, refused before an ID is claimed so that a
/// request without it leaves nothing behind.
fn check_content(request: &GateRequest) -> Result<(), String> {
    let kind = request.kind;
    // Every gate that waits is answered from what the card says it asks.
    if request.wait && request.focus.is_none() {
        return Err("a gate that waits needs focus: the question the person answers".to_string());
    }
    if kind == Kind::Plan {
        // The check `adj task update --plan` makes, so the two cannot drift.
        crate::task::check_plan_steps(
            "a plan gate",
            request.steps.iter().map(|s| s.text.as_str()),
        )?;
    } else {
        for (at, step) in request.steps.iter().enumerate() {
            if step.text.trim().is_empty() {
                return Err(format!("step {} has no text", at + 1));
            }
        }
    }
    for (at, section) in request.sections.iter().enumerate() {
        if section.heading.trim().is_empty() || section.body.trim().is_empty() {
            return Err(format!("section {} needs a heading and a body", at + 1));
        }
    }
    if kind == Kind::Result
        && request.sections.is_empty()
        && request.body.as_deref().is_none_or(|b| b.trim().is_empty())
    {
        return Err("a result gate needs what was found: sections, or a body".to_string());
    }
    if request.quick {
        if !request.wait {
            return Err("a record asks nobody, so it cannot be quick".to_string());
        }
        if request.choices.is_empty() {
            return Err("a quick gate is answered with one press: give it choices".to_string());
        }
        let blank = |text: &Option<String>| text.as_deref().is_none_or(|t| t.trim().is_empty());
        let carried = [
            ("body", !blank(&request.body)),
            ("diff", !blank(&request.diff)),
            ("run", !blank(&request.run)),
            ("steps", !request.steps.is_empty()),
            ("sections", !request.sections.is_empty()),
            ("findings", !request.findings.is_empty()),
            ("commands", !request.commands.is_empty()),
            ("manual", !request.manual.is_empty()),
            ("report", request.report.is_some()),
        ];
        if let Some((field, _)) = carried.iter().find(|(_, present)| *present) {
            return Err(format!(
                "a quick gate carries nothing to read: drop {field} or quick"
            ));
        }
    }
    Ok(())
}

/// Whether a plan may be kept as a record: the task it is for stops at `none`, and a worker
/// is the one opening it. Fails closed: a task that cannot be found or read, a hub as opener
/// (a plan the hub opens is Jules' and waits) and every other Stop at refuse.
fn check_plan_record(ctx: &Context, request: &GateRequest) -> Result<(), String> {
    if request.opened_by == Opener::Hub {
        return Err(
            "a plan the hub opens waits for an answer; it cannot be kept as a record".to_string(),
        );
    }
    // The id as it will be stored, not trimmed: the board ties a record to its task by the
    // exact id, so one that passes only once trimmed would be a record no task shows.
    let id = request
        .task
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .ok_or(
            "a plan gate cannot be kept as a record without the task it is for; only a task whose Stop at is none can",
        )?;
    let task = crate::task::get(&ctx.state, &ctx.repo.slug, id).map_err(|why| {
        format!("a plan gate cannot be kept as a record: task {id} could not be checked for Stop at none: {why}")
    })?;
    if task.stop_at != crate::task::StopAt::Nothing {
        return Err(format!(
            "a plan gate cannot be kept as a record: only a task whose Stop at is none can, and task {id} stops at {}",
            task.stop_at.as_str()
        ));
    }
    Ok(())
}

/// The last component of a path the caller gave, for the board to name the report by.
fn file_name(given: &str) -> String {
    std::path::Path::new(given)
        .file_name()
        .map_or_else(|| given.to_string(), |n| n.to_string_lossy().into_owned())
}
