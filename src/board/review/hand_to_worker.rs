//! Handing the drafts of a task's PR to the task's worker.
//!
//! The drafts are the person's own file under `others/drafts/`, kept by the page; this reads them
//! and writes nothing there. The page marks what was handed as sent once it is told, and saves.
//! The sending itself is the gate's: the points go to the worker as a send-back of its diff.

use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;

use crate::gate::{self, Instruction, Kind, ReadHandoff, SendBackRefused, Shelf};
use crate::others::{self, PrOpenState, Side, Source, StoredDrafts};
use crate::registry::Context;
use crate::task::{self, Executor, PrRef};

/// What was handed, and where it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Handed {
    pub gate_id: String,
    pub kind: Kind,
    /// Where the gate is now: `answered` (it was waiting) or `record` (the answer was appended).
    pub shelf: &'static str,
    /// The keys of the drafts that were sent.
    pub sent: Vec<String>,
    /// The comment on the whole was sent.
    pub overall: bool,
    /// Stamp of the answer.
    pub at: String,
    /// Whether the worker was running to read it.
    pub present: bool,
    /// Whether it was woken.
    pub woken: bool,
}

/// Why the drafts were not handed over. Every one but `Unreadable` and `Failed` is said before
/// anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandRefused {
    NoTask,
    /// The task has no PR this can read.
    NoPr,
    /// The task's PR is not the one the page was showing.
    PrChanged,
    /// A task handed to Jules has no worker to tell.
    NoWorker,
    /// The PR has no record: it was never read.
    NoRecord,
    /// The PR is merged or closed.
    PrDone,
    /// No draft is unsent and written, and the comment on the whole is blank.
    Nothing,
    /// A hand-over for this task is running.
    Busy,
    /// The task has no `diff` gate waiting and no `diff` record.
    NoGate,
    /// The task has a gate of another kind waiting.
    GateWaiting(Kind),
    /// The task, the record, the drafts or the read-through could not be read, and why. Not the
    /// same as there being none.
    Unreadable(String),
    /// The gate could not be read, delivered to or written, and why.
    Failed(String),
}

/// The tasks a hand-over is running for, by state root, board and id: a second click is refused
/// while the first is on its way, so one point is not sent twice.
static RUNNING: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Running(String);

impl Running {
    fn take(root: &Path, slug: &str, task_id: &str) -> Option<Running> {
        let key = format!("{}|{slug}|{task_id}", root.display());
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if running.contains(&key) {
            return None;
        }
        running.push(key.clone());
        Some(Running(key))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        running.retain(|k| k != &self.0);
    }
}

/// Hand the unsent drafts of the PR of task `task_id`, and the comment on the whole, to its
/// worker. `pr_seen` is the PR (`owner/repo#N`) the page showed them for: a task that has another
/// PR now is refused.
pub fn hand_to_worker(ctx: &Context, task_id: &str, pr_seen: &str) -> Result<Handed, HandRefused> {
    let root = &ctx.state;
    let _running = Running::take(root, &ctx.repo.slug, task_id).ok_or(HandRefused::Busy)?;
    let found = match task::get(root, &ctx.repo.slug, task_id) {
        Ok(found) => found,
        Err(why) => {
            return Err(match task::exists(root, &ctx.repo.slug, task_id) {
                Ok(false) => HandRefused::NoTask,
                _ => HandRefused::Unreadable(why),
            });
        }
    };
    let pr: PrRef = task::pr_refs(ctx, std::slice::from_ref(&found))
        .into_iter()
        .next()
        .flatten()
        // The records are keyed by owner and repository only: a PR on another host would find another's.
        .filter(|pr| pr.host == "github.com")
        .ok_or(HandRefused::NoPr)?;
    let id = format!("{}#{}", pr.nwo(), pr.number);
    if !id.eq_ignore_ascii_case(pr_seen.trim()) {
        return Err(HandRefused::PrChanged);
    }
    if found.executor == Executor::Jules {
        return Err(HandRefused::NoWorker);
    }
    let record = others::record(root, &pr)
        .map_err(HandRefused::Unreadable)?
        .ok_or(HandRefused::NoRecord)?;
    if record.pr_state != PrOpenState::Open {
        return Err(HandRefused::PrDone);
    }

    let drafts = match others::drafts(root, &pr).map_err(HandRefused::Unreadable)? {
        Some(value) => serde_json::from_value::<StoredDrafts>(value)
            .map_err(|e| HandRefused::Unreadable(format!("cannot read the drafts of {id}: {e}")))?
            .input()
            .map_err(HandRefused::Unreadable)?,
        None => return Err(HandRefused::Nothing),
    };
    let items: Vec<Instruction> = drafts
        .drafts
        .iter()
        .filter(|d| d.sent_at.is_none() && !d.body.trim().is_empty())
        .map(|d| Instruction {
            key: d.key.clone(),
            // A point with no file is on the whole PR, whatever line it carries.
            line: d.path.as_ref().and(d.line),
            start_line: d.path.as_ref().and(d.start_line),
            path: d.path.clone(),
            left: d.side == Side::Left,
            body: d.body.clone(),
            by_reader: d.source == Source::Reader,
        })
        .collect();
    let overall = drafts
        .overall
        .as_ref()
        .map(|o| o.body.trim())
        .filter(|o| !o.is_empty());
    if items.is_empty() && overall.is_none() {
        return Err(HandRefused::Nothing);
    }
    // The head the read-through was made at: what its lines are of.
    let sha = others::read_through(root, &pr)
        .map_err(HandRefused::Unreadable)?
        .and_then(|read| read.get("sha").and_then(|s| s.as_str().map(str::to_string)));

    // Drafts kept across a re-read are of the head they were taken from, not of the read-through now.
    let drafts_sha = drafts
        .from_sha
        .clone()
        .filter(|from| sha.as_deref().is_some_and(|now| now != from));
    let handoff = ReadHandoff {
        pr: id,
        sha,
        drafts_sha,
        count: items.len() as u32,
        overall: overall.is_some(),
        extra: serde_json::Map::new(),
    };
    let sent =
        gate::send_back_read(ctx, task_id, &items, overall, handoff).map_err(|e| match e {
            SendBackRefused::NoGate => HandRefused::NoGate,
            SendBackRefused::GateWaiting { kind } => HandRefused::GateWaiting(kind),
            SendBackRefused::Nothing => HandRefused::Nothing,
            SendBackRefused::Failed(why) => HandRefused::Failed(why),
        })?;
    let at = sent
        .gate
        .answers
        .last()
        .map(|a| a.answered_at.clone())
        .or_else(|| sent.gate.answered_at.clone())
        .unwrap_or_default();
    Ok(Handed {
        gate_id: sent.gate.id,
        kind: sent.gate.kind,
        shelf: match sent.shelf {
            Shelf::Record => "record",
            _ => "answered",
        },
        sent: items.into_iter().map(|i| i.key).collect(),
        overall: overall.is_some(),
        at,
        present: sent.told.is_present(),
        woken: sent.told.was_woken(),
    })
}

#[cfg(test)]
mod tests;
