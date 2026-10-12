//! Sending the worker the points a person chose from a read-through of its task's PR.

use super::answer::{AnswerError, answer_with};
use super::store::dir;
use super::*;

use crate::mail::DeliveryOutcome;
use crate::registry::Context;

/// Why points were not sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendBackRefused {
    /// The task has no `diff` gate waiting and no `diff` record to send them back on.
    NoGate,
    /// The task has a gate of another kind waiting: sending changes on a diff would answer
    /// around it, and the person has to answer it first.
    GateWaiting { kind: Kind },
    /// Nothing was chosen to send.
    Nothing,
    /// The gate chosen was answered or closed by somebody else before this could answer it.
    GateChanged,
    /// A gate could not be read, delivered to or written, and why.
    Failed(String),
}

/// What was sent, and where it went.
#[derive(Debug, Clone)]
pub struct SentBack {
    /// The gate as it is now: answered and archived, or the record with the answer appended.
    pub gate: Gate,
    /// The shelf the gate is on now: `Answered` or `Record`.
    pub shelf: Shelf,
    pub told: DeliveryOutcome,
}

/// Send the worker of `task_id` the points `items` and the comment on the whole PR, as `changes`
/// on one of the task's gates.
///
/// The gate is the one a person would have sent back by hand: a waiting `diff` or `verify` gate
/// of the task (answered and archived, exactly as `answer` does), else the task's newest `diff`
/// record (the answer is appended, as a record is sent back today). A gate of another kind that is
/// waiting is refused, and so is a task with neither; both before anything is written. Answered
/// gates and records of other kinds are never written to.
///
/// `handoff` says where the points are from; its `count` and `overall` are made to agree with
/// `items` and `overall` here.
pub fn send_back_read(
    ctx: &Context,
    task_id: &str,
    items: &[Instruction],
    overall: Option<&str>,
    handoff: ReadHandoff,
) -> Result<SentBack, SendBackRefused> {
    let overall = overall.map(str::trim).filter(|o| !o.is_empty());
    if items.is_empty() && overall.is_none() {
        return Err(SendBackRefused::Nothing);
    }
    let handoff = ReadHandoff {
        count: items.len() as u32,
        overall: overall.is_some(),
        ..handoff
    };
    let of_task = |g: &Gate| g.task.as_deref() == Some(task_id);
    let newest = |gates: Vec<Gate>| {
        gates
            .into_iter()
            .filter(of_task)
            .max_by(|a, b| (&a.opened_at, &a.id).cmp(&(&b.opened_at, &b.id)))
    };
    let slug = &ctx.repo.slug;
    let waiting: Vec<Gate> =
        super::store::list_where(&dir(&ctx.state, slug, Shelf::Open), |_| true)
            .into_iter()
            .filter(|g| g.wait && !g.answered_by_hub())
            .collect();
    let (gate, shelf) = if let Some(g) = newest(
        waiting
            .iter()
            .filter(|g| matches!(g.kind, Kind::Diff | Kind::Verify))
            .cloned()
            .collect(),
    ) {
        (g, Shelf::Answered)
    } else if let Some(other) = newest(waiting) {
        return Err(SendBackRefused::GateWaiting { kind: other.kind });
    } else if let Some(g) = newest(list_of_kind(&ctx.state, slug, Shelf::Record, Kind::Diff)) {
        (g, Shelf::Record)
    } else {
        return Err(SendBackRefused::NoGate);
    };

    let comment = comment_of(items, overall, &handoff);
    let (gate, told) = answer_with(
        ctx,
        &gate.id,
        "changes",
        None,
        Some(&comment),
        Some(&handoff),
    )
    .map_err(|e| match e {
        AnswerError(_, true) => SendBackRefused::GateChanged,
        AnswerError(why, false) => SendBackRefused::Failed(why),
    })?;
    Ok(SentBack { gate, shelf, told })
}

/// The comment the worker reads: what these are, then each point numbered with where it is.
fn comment_of(items: &[Instruction], overall: Option<&str>, handoff: &ReadHandoff) -> String {
    let n = items.len();
    let of = handoff
        .short_sha()
        .map(|sha| format!("Lines are of {sha} (right side unless said)."))
        .unwrap_or_else(|| "Lines are of the read head (right side unless said).".to_string());
    let points = match n {
        0 => "a comment on the whole PR".to_string(),
        1 => "1 point".to_string(),
        _ => format!("{n} points"),
    };
    // Drafts kept across a re-read: the reader's points are of the earlier head, which is said.
    let earlier = handoff
        .short_drafts_sha()
        .map(|sha| {
            format!(" Some points were kept from an earlier read at {sha}; their lines are of that head.")
        })
        .unwrap_or_default();
    let mut out = format!(
        "The person read this PR with an AI read-through and sends you {points}. {of}{earlier}\n(AI) = the reader wrote it and the person kept it; the rest the person wrote.\n"
    );
    for (i, item) in items.iter().enumerate() {
        let ai = if item.by_reader { " (AI)" } else { "" };
        out.push_str(&format!(
            "\n### {}. {}{ai}\n{}\n",
            i + 1,
            place_of(item),
            item.body.trim()
        ));
    }
    if let Some(overall) = overall {
        out.push_str(&format!("\n### Overall\n{overall}\n"));
    }
    out
}

fn place_of(item: &Instruction) -> String {
    let Some(path) = &item.path else {
        return "the whole PR".to_string();
    };
    let Some(line) = item.line else {
        return format!("{path} (the file)");
    };
    let range = match item.start_line {
        Some(start) if start != line => format!("{start}-{line}"),
        _ => line.to_string(),
    };
    let side = if item.left { " (old side)" } else { "" };
    format!("{path}:{range}{side}")
}
