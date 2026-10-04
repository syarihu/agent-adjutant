//! A gate: something an agent has prepared for a person to look at, and the ball handed
//! over with it.
//!
//! Not a question with buttons. The agent stops, puts the artefact where a person can see
//! it, and waits — so the shape of the payload is the point, and it is three frames rather
//! than one wall of prose: what to look at, what was already decided, and what the agent
//! was unsure of. A reviewer who has to read four hundred lines to find the two decisions
//! that matter is a reviewer who approves without reading.
//!
//! The answer travels back out through the outbox the hub already uses to reach a worker,
//! so a gate needs no channel of its own — and an answer to a worktree whose worker has
//! died simply waits there, which is the same promise `adjutant tell` already makes.
//!
//! Reads take a state root and a hub's slug, so the board can read every hub's gates; writes
//! take the `Context` of the hub they are for. Nothing outside this module builds
//! `gates/<slug>/…` or takes a gate's lock. Told the time rather than asking.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::registry::Context;

/// What is being shown. Each one is a moment that used to be an `AskUserQuestion` in a tab
/// nobody was watching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A worker's plan, before it writes anything.
    Plan,
    /// A diff, once the worker's own review rounds have converged.
    Diff,
    /// Built and ready; somebody has to run it. The one kind whose answer comes after work
    /// that happens outside the browser.
    Verify,
    /// The hub asking whether to start on something.
    Dispatch,
    /// The hub's draft of an issue it is about to file.
    Issue,
    /// Neither of the above: something an agent cannot decide by itself.
    Question,
    /// A finished piece of investigation. There is nothing to approve — it is read.
    Result,
    /// The hub's selection of review comments on a Jules PR, each with what Jules should know
    /// about it, to be passed on once a person approves.
    Relay,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Plan => "plan",
            Kind::Diff => "diff",
            Kind::Verify => "verify",
            Kind::Dispatch => "dispatch",
            Kind::Issue => "issue",
            Kind::Question => "question",
            Kind::Result => "result",
            Kind::Relay => "relay",
        }
    }

    /// Whether the hub opened this rather than a worker. The hub reads its inbox and never
    /// an outbox, so the answer has to be delivered there instead — to the outbox of the
    /// main checkout it would sit unread.
    ///
    /// Decided by kind because the kind already says who is asking: whether to start a task
    /// and whether to file an issue are the hub's questions, and nothing a worker asks about.
    /// A kind that either side could open has the opener written into the gate instead (see
    /// `Gate::answered_by_hub`).
    pub fn answered_by_hub(self) -> bool {
        matches!(self, Kind::Dispatch | Kind::Issue | Kind::Relay)
    }

    /// Whether a gate of this kind may be kept as a record instead of waited on.
    ///
    /// Only the two a worker opens after its own checks have run: the review and the check
    /// can have nothing in them for a person. A plan always waits, and the hub's and the
    /// question kinds exist to be answered.
    pub fn can_be_recorded(self) -> bool {
        matches!(self, Kind::Diff | Kind::Verify)
    }

    /// What the buttons are, when the payload does not say.
    pub fn default_options(self) -> Vec<String> {
        let options: &[&str] = match self {
            Kind::Result => &["ack", "ask"],
            Kind::Verify | Kind::Diff => &["approve", "changes"],
            Kind::Question => &["answer"],
            _ => &["approve", "changes", "reject"],
        };
        options.iter().map(|o| o.to_string()).collect()
    }
}

/// Who opened a gate, for the one kind either side can open. A plan is the worker's own
/// step for a task it implements, and the hub's for a task handed to Jules, where no worker
/// is started and the hub has a sub-agent write the plan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Opener {
    #[default]
    Worker,
    Hub,
}

impl Opener {
    fn is_worker(&self) -> bool {
        *self == Opener::Worker
    }
}

/// How bad a review finding is. The same three words the worker's review loop reports in, so
/// a record reads the way the rounds were run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// A defect if merged.
    Must,
    /// Correct, and could be better.
    Want,
    /// Not what the task asked for.
    Scope,
}

/// What became of a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Still there: not fixed, and not ruled out.
    Open,
    Fixed,
    /// Checked against the code and ruled a false positive. The reason goes with it.
    Declined,
}

/// Why a `diff` or `verify` gate waits on a person rather than being kept as a record. The
/// worker's procedure names the same five rules, and a gate that stops it says which fired,
/// so the board can tell a stop that needs a person from one the task was handed over with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopRule {
    /// The review rounds hit `selfReviewRounds` with a must still open.
    RoundLimit,
    /// `verify` failed and the worker could not fix it.
    VerifyFailed,
    /// Something only a person can check, such as a screen change.
    ManualCheck,
    /// The worker wrote down where its confidence ran out.
    Unsure,
    /// The task's stop point covers this gate.
    StopAt,
}

impl StopRule {
    pub fn as_str(self) -> &'static str {
        match self {
            StopRule::RoundLimit => "round-limit",
            StopRule::VerifyFailed => "verify-failed",
            StopRule::ManualCheck => "manual-check",
            StopRule::Unsure => "unsure",
            StopRule::StopAt => "stop-at",
        }
    }

    /// Whether this rule can be why a gate of `kind` stopped. The same table the worker's
    /// procedure decides by: the review's round limit is the diff's, a check left for a
    /// person is the verify's, and the rest can stop either.
    pub fn applies_to(self, kind: Kind) -> bool {
        match self {
            StopRule::RoundLimit => kind == Kind::Diff,
            StopRule::ManualCheck => kind == Kind::Verify,
            StopRule::VerifyFailed | StopRule::Unsure | StopRule::StopAt => kind.can_be_recorded(),
        }
    }
}

/// One round of the worker's own review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRound {
    /// Who read the diff: `claude`, `codex`, ...
    pub engine: String,
    #[serde(default)]
    pub must: u32,
    #[serde(default)]
    pub want: u32,
    #[serde(default)]
    pub scope: u32,
    #[serde(default)]
    pub false_positives: u32,
}

/// One thing a review round raised, and how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub severity: Severity,
    /// `file:line`, or whatever the reviewer pointed at.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub location: String,
    pub text: String,
    pub outcome: Outcome,
    /// Why it was declined. A false positive with no reason is one the next reader has to
    /// re-check from scratch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunResult {
    Pass,
    Fail,
}

/// One `verify` command as it was run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRun {
    pub command: String,
    pub result: RunResult,
    /// How long it took, as the worker measured it (`42s`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    /// What it printed, or the tail of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// How many runs it took to reach `result`. Above one, it failed first and was fixed: a
    /// pass the board marks, since the first run is the one that says something was wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
}

/// One answer to a record. A record stays where it is when it is answered, so the answers
/// are appended rather than written over the gate's own `decision`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub answered_at: String,
}

/// One of the designs an agent is asking a person to choose between.
///
/// Two of these side by side is the thing a terminal cannot do: in a tab the second option
/// has already scrolled past the first by the time you have read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<String>,
    /// Which one the agent would pick. Said out loud rather than implied by ordering.
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gate {
    pub id: String,
    pub kind: Kind,
    /// Where the answer goes. A worktree rather than a session, so an answer outlives the
    /// agent that asked for it.
    pub worktree: String,
    /// Who is waiting on the answer. Only ever written for a plan the hub opened; every other
    /// gate's opener follows from its kind.
    #[serde(default, skip_serializing_if = "Opener::is_worker")]
    pub opened_by: Opener,
    /// The task this belongs to, when there is a record for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub title: String,
    /// What is true regardless of the decision: rounds run, tests passed, files touched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<String>,
    // ── the three frames ──
    /// What the person has to decide. Read this and you can answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    /// Settled. Shown folded away, because it is here to be available rather than read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided: Option<String>,
    /// Where the agent's own confidence ran out. The half of a review people never get.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsure: Option<String>,
    // ── the attachments, by kind ──
    /// A report, for a gate that is read rather than decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// How to run it, for `verify`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// How many times this same point has been round-tripped. The agent knows; the board
    /// uses it to suggest going to the tab instead, because past two rounds a gate has
    /// become a conversation and a conversation is faster where it is not posted.
    #[serde(default)]
    pub rounds: u32,
    // ── the structured attachments, by kind ──
    /// `plan`: what is wrong today, from the request and the issue the worker read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// `plan`: what done looks like.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// `diff`: the worker's own review, one entry per round. Not `rounds`, which already
    /// counts how often this gate has been round-tripped with a person.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_rounds: Vec<ReviewRound>,
    /// `diff`: what those rounds raised.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    /// `verify`: the commands that were run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandRun>,
    /// `verify`: the checks left for a person.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manual: Vec<String>,
    /// `diff` / `verify`: the rules that made this gate wait rather than be kept as a record.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stopped_by: Vec<StopRule>,
    /// `false` for a record: written down for the board, while the worker carries on.
    #[serde(default = "waits", skip_serializing_if = "is_waiting")]
    pub wait: bool,
    pub opened_at: String,
    // ── written when it is answered ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<String>,
    /// A record's answers, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<Answer>,
    /// Keys this binary does not know, kept from the file so that a record written by another
    /// version and saved by this one loses nothing. Only ever filled from disk: `open` empties it.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The decision a gate is archived under when it is closed without an answer.
pub const CLOSED: &str = "closed";

/// The decision a gate is archived under when the person answered it in the worker's
/// terminal instead of on the board.
pub const TERMINAL: &str = "terminal";

/// What showed a worker had moved on from a waiting gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// The worker's phase changed.
    Phase,
    /// The worker opened a later gate or record.
    Gate,
}

/// When the worker that opened `gate` visibly moved on, and by what: the earlier of its
/// phase changing and a later gate or record coming from the same worktree, either strictly
/// after the gate was opened.
///
/// `None` for a gate nobody waits on the worker for (a record, or one the hub opened), and
/// for a worktree whose worker record does not show the worker that opened this gate:
/// `worker_started` is that record's `startedAt`, and a worker started after the gate was
/// opened cannot be the one that opened it. A worker started in the same second cannot be
/// told apart from it either, so only one started strictly earlier is believed. Stamps are
/// fixed-width, so they compare as strings, and the same second is not later.
pub fn resumed_at(
    gate: &Gate,
    worker_started: Option<&str>,
    phase_at: Option<&str>,
    later_opened: Option<&str>,
) -> Option<(String, Signal)> {
    if !gate.wait || gate.answered_by_hub() {
        return None;
    }
    if worker_started.is_none_or(|started| started >= gate.opened_at.as_str()) {
        return None;
    }
    let later = |at: Option<&str>, signal| {
        at.filter(|at| *at > gate.opened_at.as_str())
            .map(|at| (at.to_string(), signal))
    };
    match (
        later(phase_at, Signal::Phase),
        later(later_opened, Signal::Gate),
    ) {
        (Some(phase), Some(opened)) => Some(if opened.0 < phase.0 { opened } else { phase }),
        (phase, opened) => phase.or(opened),
    }
}

/// The gates that can show a worker in `open` (one hub's open gates, as `list` read them)
/// moved on to something else: `open` itself, and what the hub's records and answered gates
/// hold that was written since the earliest waiting gate was opened. Only reads, so a board
/// can ask it about a hub that is not its own.
///
/// A gate already answered on the board still shows the worker got as far as opening it. The
/// archive only grows, so it is not parsed whole: a file older than the earliest waiting gate
/// cannot be a signal. A little slack for coarse file times; the mtime only prunes, and
/// `resumed_at` and the caller's filter decide.
pub fn resume_signals(root: &Path, slug: &str, open: &[Gate]) -> Vec<Gate> {
    let open_dir = dir(root, slug, Shelf::Open);
    let since = open
        .iter()
        .filter(|g| g.wait && !g.answered_by_hub())
        .filter_map(|g| {
            std::fs::metadata(path_of(&open_dir, &g.id))
                .and_then(|m| m.modified())
                .ok()
        })
        .min()
        .map(|t| t - std::time::Duration::from_secs(2))
        .unwrap_or(std::time::UNIX_EPOCH);
    open.iter()
        .cloned()
        .chain(list_modified_since(&dir(root, slug, Shelf::Record), since))
        .chain(list_modified_since(
            &dir(root, slug, Shelf::Answered),
            since,
        ))
        .filter(|g| !g.answered_by_hub())
        .collect()
}

impl Gate {
    /// Whether a person decided this on the board: it has a decision, and that decision is
    /// not one of the two ways a gate is closed without the board's answer.
    pub fn answered_on_board(&self) -> bool {
        self.decision
            .as_deref()
            .is_some_and(|d| d != CLOSED && d != TERMINAL)
    }

    /// Whether the answer goes to the hub's inbox rather than the worktree's outbox: a kind
    /// only the hub opens, or a plan the hub opened for a task handed to Jules. That plan's
    /// worktree has no worker in it, and an answer left in its outbox would never be read.
    pub fn answered_by_hub(&self) -> bool {
        self.kind.answered_by_hub() || self.opened_by == Opener::Hub
    }
}

fn waits() -> bool {
    true
}

fn is_waiting(wait: &bool) -> bool {
    *wait
}

/// Where a gate lives. Three directories of one hub's state, each holding `<id>.json` files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    /// Where this hub's open gates wait.
    Open,
    /// Where a gate kept as a record lives. Beside `answered/` rather than in the open queue,
    /// so nothing that counts what is waiting for a person counts it.
    Record,
    /// Where a gate goes once it has been answered. Kept rather than deleted, for the same
    /// reason the inbox keeps what it has read: a decision that turned out wrong has to be
    /// findable afterwards.
    Answered,
}

fn dir(root: &Path, slug: &str, shelf: Shelf) -> PathBuf {
    let open = root.join("gates").join(slug);
    match shelf {
        Shelf::Open => open,
        Shelf::Record => open.join("records"),
        Shelf::Answered => open.join("answered"),
    }
}

fn dir_of(ctx: &Context, shelf: Shelf) -> PathBuf {
    dir(&ctx.state, &ctx.repo.slug, shelf)
}

fn path_of(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// Whether a gate file is on `shelf`, without reading it. Lets a caller tell a missing gate
/// from one that is there and broken, which `load` reports the same way.
pub fn exists(root: &Path, slug: &str, shelf: Shelf, id: &str) -> bool {
    path_of(&dir(root, slug, shelf), id).exists()
}

/// `20260922T041233Z-diff`, and a suffix if that name is taken.
///
/// Claimed rather than checked, like a task id and an inbox filename: a worker finishing two
/// pieces of work in the same second is ordinary, and the loser of a check-then-write would
/// overwrite a gate somebody is in the middle of reading.
///
/// A record's id ends in `-record` (`20260922T041233Z-diff-record`). Named apart from an open
/// gate's id rather than claimed on both shelves: an answer finds its gate by id alone, and a
/// record and a gate opened in the same second must not be mistaken for each other.
pub fn claim_id(ctx: &Context, shelf: Shelf, stamp: &str, kind: Kind) -> Result<String, String> {
    let base = if shelf == Shelf::Record {
        format!("{stamp}-{}-record", kind.as_str())
    } else {
        format!("{stamp}-{}", kind.as_str())
    };
    claim(&dir_of(ctx, shelf), base)
}

fn claim(dir: &Path, base: String) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for seq in 1..1000 {
        let id = if seq == 1 {
            base.clone()
        } else {
            format!("{base}-{seq}")
        };
        match crate::infra::fs::create_new(&path_of(dir, &id)) {
            Ok(true) => return Ok(id),
            Ok(false) => continue,
            Err(e) => {
                return Err(format!(
                    "cannot create a gate file in {}: {e}",
                    dir.display()
                ));
            }
        }
    }
    Err(format!("no free gate id for {base}"))
}

/// Write a gate, replacing whatever is at its name in one step.
///
/// A record is written again each time it is answered, while the board reads it every few
/// seconds and nothing it reads through takes the writer's lock. Written in place, a reader
/// could catch it half-written and drop it from the listing, and a write cut short would
/// leave it unreadable for good. Staged as a dotfile beside it, synced and renamed over it,
/// the name never points at a partial file.
pub fn save(ctx: &Context, shelf: Shelf, gate: &Gate) -> Result<PathBuf, String> {
    let path = path_of(&dir_of(ctx, shelf), &gate.id);
    crate::infra::fs::write_json(&path, gate)?;
    Ok(path)
}

pub fn load(root: &Path, slug: &str, shelf: Shelf, id: &str) -> Result<Gate, String> {
    let path = path_of(&dir(root, slug, shelf), id);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no open gate: {id}"))?;
    serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// A gate by id, open or kept as a record. Open first: that is what an id usually names, and
/// a record's id cannot be an open gate's (see `claim_id`).
pub fn get(root: &Path, slug: &str, id: &str) -> Result<Gate, String> {
    // Only a missing file falls through: one that is there and broken says so, rather than
    // reading as an id that does not exist.
    if exists(root, slug, Shelf::Open, id) {
        return load(root, slug, Shelf::Open, id);
    }
    if exists(root, slug, Shelf::Record, id) {
        return load(root, slug, Shelf::Record, id);
    }
    Err(format!("no open gate or record: {id}"))
}

/// Every gate on `shelf`, oldest first — which for the open shelf is the order they should
/// be worked through.
pub fn list(root: &Path, slug: &str, shelf: Shelf) -> Vec<Gate> {
    list_where(&dir(root, slug, shelf), |_| true)
}

/// The gates in `dir` whose file was written at or after `since`, oldest first. For a caller
/// that can only care about gates opened after some moment: a gate's file is written after it
/// is opened, so an older file cannot be one, and the directory's history is not parsed.
/// Only prunes: a caller still checks what it needs of each gate it gets.
fn list_modified_since(dir: &Path, since: SystemTime) -> Vec<Gate> {
    let mut gates: Vec<Gate> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter(|e| {
                e.metadata()
                    .and_then(|m| m.modified())
                    .is_ok_and(|modified| modified >= since)
            })
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|text| serde_json::from_str::<Gate>(&text).ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    gates.sort_by(|a, b| a.opened_at.cmp(&b.opened_at));
    gates
}

/// The gates of one kind on `shelf`, told apart by their file names before any is read. For
/// the archive, which only grows: the board asks it for plans on every poll, and parsing every
/// diff ever answered to find them would cost more each day.
pub fn list_of_kind(root: &Path, slug: &str, shelf: Shelf, kind: Kind) -> Vec<Gate> {
    // `{stamp}-{kind}`, `{stamp}-{kind}-{seq}` or `{stamp}-{kind}-record`. No kind's name
    // begins another's, so the prefix is enough; the kind is checked again once parsed.
    list_where(&dir(root, slug, shelf), |id| {
        id.split_once('-')
            .is_some_and(|(_, rest)| rest.starts_with(kind.as_str()))
    })
    .into_iter()
    .filter(|g| g.kind == kind)
    .collect()
}

fn list_where(dir: &Path, wanted: impl Fn(&str) -> bool) -> Vec<Gate> {
    let mut gates: Vec<Gate> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter(|e| {
                e.path()
                    .file_stem()
                    .is_some_and(|stem| wanted(&stem.to_string_lossy()))
            })
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|text| serde_json::from_str::<Gate>(&text).ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    gates.sort_by(|a, b| a.opened_at.cmp(&b.opened_at));
    gates
}

/// Move an answered gate out of the way, so "open" means what it says.
///
/// Written through `write_json` like `save`: the board reads `answered/` on every poll, so it
/// must never see a half-written file there.
pub fn archive(ctx: &Context, gate: &Gate) -> Result<PathBuf, String> {
    let path = path_of(&dir_of(ctx, Shelf::Answered), &gate.id);
    crate::infra::fs::write_json(&path, gate)?;
    let _ = std::fs::remove_file(path_of(&dir_of(ctx, Shelf::Open), &gate.id));
    Ok(path)
}

/// Hold the write lock of one gate or record until the returned handle is dropped. The same
/// advisory lock `task::lock` takes, for the same reason: appending an answer is a read
/// and a write of the whole file, and two at once would each write back what they read. On
/// an open gate it is what lets only one of the board's answer, the worker's close and the
/// board's own sweep decide it.
pub fn lock(ctx: &Context, shelf: Shelf, id: &str) -> Result<std::fs::File, String> {
    crate::infra::fs::lock(&lock_path(&dir_of(ctx, shelf), id))
}

/// `lock` without waiting: `None` when somebody else holds it.
pub fn try_lock(ctx: &Context, shelf: Shelf, id: &str) -> Result<Option<std::fs::File>, String> {
    crate::infra::fs::try_lock(&lock_path(&dir_of(ctx, shelf), id))
}

/// Not named `.json`, so the listing never reads it as a gate. Never removed, for the reason
/// given at `with_dispatch_lock`: a lock file that is unlinked can be locked twice.
fn lock_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.lock"))
}

/// The subject the answer is delivered under.
///
/// The identifier goes first, exactly as the hub's own `[question {stamp}]` does: it is the
/// only thing tying an answer to what was asked, and an agent reading its outbox has
/// nothing else to match on.
pub fn answer_subject(gate: &Gate, decision: &str) -> String {
    format!("[gate {}] {decision}", gate.id)
}

/// What the worker reads in its outbox.
///
/// Written as prose rather than JSON because the reader is an agent mid-task: it has to be
/// obvious what was decided from the first line, and the comment is the part that changes
/// what happens next.
pub fn answer_body(
    gate: &Gate,
    decision: &str,
    choice: Option<&str>,
    comment: Option<&str>,
) -> String {
    let mut out = format!("## Decision   {decision}\n");
    if let Some(choice) = choice {
        let label = gate
            .choices
            .iter()
            .find(|c| c.id == choice)
            .map(|c| c.label.as_str())
            .unwrap_or(choice);
        out.push_str(&format!("## Chosen     {label} ({choice})\n"));
    }
    // A record said as one, so the worker knows the person went back to something it had
    // already moved past rather than something it is waiting on.
    let record = if gate.wait { "" } else { ", record" };
    out.push_str(&format!(
        "## gate       {} ({}{record})\n",
        gate.id,
        gate.kind.as_str()
    ));
    // By the time this is read the gate has been archived, so `adj gate show` no longer
    // finds it. The hub has to know which task it just decided on from the message alone.
    if let Some(task) = &gate.task {
        out.push_str(&format!("## task       {task}\n"));
    }
    match comment.map(str::trim).filter(|c| !c.is_empty()) {
        Some(comment) => out.push_str(&format!("\n## Comment\n\n{comment}\n")),
        // Said rather than left out: an agent that sees no comment section has to work out
        // whether there was none or whether it lost one.
        None => out.push_str("\n## Comment\n\n(none)\n"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hub's context in a sandboxed state directory. Hold the sandbox for the whole test.
    fn hub() -> (crate::testing::Sandbox, Context) {
        let sandbox = crate::testing::Sandbox::empty();
        let repo = crate::kernel::identity::RepoInfo {
            main: "/tmp/acme-widget".to_string(),
            nwo: "acme/widget".to_string(),
            repo: "widget".to_string(),
            hub: None,
            slug: "acme-widget".to_string(),
            hub_name: "adjutant-acme-widget".to_string(),
            nwo_source: "dirname",
        };
        let ctx = crate::registry::context_at(repo, sandbox.state()).unwrap();
        (sandbox, ctx)
    }

    fn ids(gates: Vec<Gate>) -> Vec<String> {
        gates.into_iter().map(|g| g.id).collect()
    }

    fn gate(kind: Kind) -> Gate {
        Gate {
            id: "20260922T041233Z-plan".to_string(),
            kind,
            worktree: "/tmp/wt".to_string(),
            opened_by: Opener::Worker,
            task: Some("20260922T041000Z-cache".to_string()),
            title: "Design review: caching search results".to_string(),
            facts: vec!["6 files to touch".to_string()],
            focus: Some("Please decide how the TTL is held".to_string()),
            decided: Some("An LRU of 64 entries".to_string()),
            unsure: None,
            body: None,
            run: None,
            diff: None,
            choices: vec![Choice {
                id: "const".to_string(),
                label: "Option A — a constant".to_string(),
                why: "there is no remote config".to_string(),
                points: vec!["small diff".to_string()],
                recommended: true,
            }],
            options: vec!["approve".to_string(), "changes".to_string()],
            rounds: 0,
            problem: None,
            goal: None,
            review_rounds: Vec::new(),
            findings: Vec::new(),
            commands: Vec::new(),
            manual: Vec::new(),
            stopped_by: Vec::new(),
            wait: true,
            opened_at: "20260922T041233Z".to_string(),
            decision: None,
            choice: None,
            comment: None,
            answered_at: None,
            answers: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn a_key_this_binary_does_not_know_survives_a_load_save_and_archive() {
        let (_sandbox, ctx) = hub();
        let gate = gate(Kind::Plan);
        let open = dir_of(&ctx, Shelf::Open);
        std::fs::create_dir_all(&open).unwrap();
        let mut raw = serde_json::to_value(&gate).unwrap();
        raw["futureField"] = serde_json::json!("x");
        std::fs::write(path_of(&open, &gate.id), raw.to_string()).unwrap();

        let loaded = load(&ctx.state, &ctx.repo.slug, Shelf::Open, &gate.id).unwrap();
        save(&ctx, Shelf::Open, &loaded).unwrap();
        let text = std::fs::read_to_string(path_of(&open, &gate.id)).unwrap();
        assert!(text.contains("\"futureField\": \"x\""), "{text}");

        let archived = archive(&ctx, &loaded).unwrap();
        let text = std::fs::read_to_string(archived).unwrap();
        assert!(text.contains("\"futureField\": \"x\""), "{text}");
    }

    #[test]
    fn only_a_decision_made_on_the_board_counts_as_answered_there() {
        let mut g = gate(Kind::Plan);
        assert!(!g.answered_on_board());
        for (decision, on_board) in [(CLOSED, false), (TERMINAL, false), ("approve", true)] {
            g.decision = Some(decision.to_string());
            assert_eq!(g.answered_on_board(), on_board, "{decision}");
        }
    }

    const STARTED: &str = "20260922T040000Z";

    #[test]
    fn a_later_phase_shows_the_worker_moved_on() {
        let g = gate(Kind::Question);
        let got = resumed_at(&g, Some(STARTED), Some("20260922T041300Z"), None);
        assert_eq!(got, Some(("20260922T041300Z".to_string(), Signal::Phase)));
        // The same second is not later, and neither is earlier.
        assert_eq!(
            resumed_at(&g, Some(STARTED), Some("20260922T041233Z"), None),
            None
        );
        assert_eq!(
            resumed_at(&g, Some(STARTED), Some("20260922T041000Z"), None),
            None
        );
        assert_eq!(resumed_at(&g, Some(STARTED), None, None), None);
    }

    #[test]
    fn a_later_gate_shows_the_worker_moved_on() {
        let g = gate(Kind::Question);
        let got = resumed_at(&g, Some(STARTED), None, Some("20260922T042000Z"));
        assert_eq!(got, Some(("20260922T042000Z".to_string(), Signal::Gate)));
    }

    #[test]
    fn with_both_signals_the_earlier_one_is_the_time() {
        let g = gate(Kind::Question);
        let got = resumed_at(
            &g,
            Some(STARTED),
            Some("20260922T043000Z"),
            Some("20260922T042000Z"),
        );
        assert_eq!(got, Some(("20260922T042000Z".to_string(), Signal::Gate)));
        let got = resumed_at(
            &g,
            Some(STARTED),
            Some("20260922T041500Z"),
            Some("20260922T042000Z"),
        );
        assert_eq!(got, Some(("20260922T041500Z".to_string(), Signal::Phase)));
    }

    #[test]
    fn a_worker_that_is_not_the_one_that_opened_the_gate_is_not_believed() {
        let g = gate(Kind::Question);
        let later = Some("20260922T050000Z");
        assert_eq!(resumed_at(&g, Some("20260922T041234Z"), later, later), None);
        assert_eq!(resumed_at(&g, None, later, later), None);
        // Started in the same second as the gate: it may be a replacement, so it is not believed.
        assert_eq!(resumed_at(&g, Some("20260922T041233Z"), later, later), None);
        assert!(resumed_at(&g, Some("20260922T041232Z"), later, None).is_some());
    }

    #[test]
    fn a_record_and_the_hub_s_gates_are_never_resumed() {
        let later = Some("20260922T050000Z");
        let mut record = gate(Kind::Verify);
        record.wait = false;
        assert_eq!(resumed_at(&record, Some(STARTED), later, later), None);
        let dispatch = gate(Kind::Dispatch);
        assert_eq!(resumed_at(&dispatch, Some(STARTED), later, later), None);
        let mut plan = gate(Kind::Plan);
        plan.opened_by = Opener::Hub;
        assert_eq!(resumed_at(&plan, Some(STARTED), later, later), None);
    }

    #[test]
    fn listing_by_modification_time_skips_older_files() {
        let (_sandbox, ctx) = hub();
        let (mut old, mut new) = (gate(Kind::Plan), gate(Kind::Plan));
        old.id = "old".to_string();
        new.id = "new".to_string();
        save(&ctx, Shelf::Open, &old).unwrap();
        save(&ctx, Shelf::Open, &new).unwrap();
        let open = dir_of(&ctx, Shelf::Open);
        let now = SystemTime::now();
        let then = now - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(path_of(&open, "old"))
            .unwrap()
            .set_modified(then)
            .unwrap();
        let since = now - std::time::Duration::from_secs(60);
        assert_eq!(ids(list_modified_since(&open, since)), ["new"]);
    }

    #[test]
    fn a_saved_gate_reads_back_the_same() {
        let (_sandbox, ctx) = hub();
        let gate = gate(Kind::Plan);
        save(&ctx, Shelf::Open, &gate).unwrap();
        assert_eq!(
            load(&ctx.state, &ctx.repo.slug, Shelf::Open, &gate.id).unwrap(),
            gate
        );
    }

    /// Written again over itself, a gate leaves nothing staged behind and still reads back.
    #[test]
    fn saving_over_a_gate_replaces_it_and_leaves_nothing_behind() {
        let (_sandbox, ctx) = hub();
        let mut gate = gate(Kind::Diff);
        save(&ctx, Shelf::Open, &gate).unwrap();
        gate.title = "Rewritten".to_string();
        save(&ctx, Shelf::Open, &gate).unwrap();
        assert_eq!(
            load(&ctx.state, &ctx.repo.slug, Shelf::Open, &gate.id).unwrap(),
            gate
        );
        let names: Vec<String> = std::fs::read_dir(dir_of(&ctx, Shelf::Open))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, [format!("{}.json", gate.id)]);
    }

    /// Two pieces of work finishing in the same second is ordinary, and the loser of a
    /// check-then-write would overwrite a gate somebody is reading.
    #[test]
    fn ids_claimed_in_the_same_second_do_not_collide() {
        let (_sandbox, ctx) = hub();
        let ids: Vec<String> = (0..3)
            .map(|_| claim_id(&ctx, Shelf::Open, "20260922T041233Z", Kind::Diff).unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "20260922T041233Z-diff",
                "20260922T041233Z-diff-2",
                "20260922T041233Z-diff-3"
            ]
        );
    }

    #[test]
    fn listing_is_oldest_first_so_the_queue_is_worked_in_order() {
        let (_sandbox, ctx) = hub();
        for (id, at) in [
            ("c", "20260922T03"),
            ("a", "20260922T01"),
            ("b", "20260922T02"),
        ] {
            let mut gate = gate(Kind::Plan);
            gate.id = id.to_string();
            gate.opened_at = at.to_string();
            save(&ctx, Shelf::Open, &gate).unwrap();
        }
        assert_eq!(
            ids(list(&ctx.state, &ctx.repo.slug, Shelf::Open)),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn listing_by_kind_reads_only_that_kind() {
        let (_sandbox, ctx) = hub();
        for (id, kind) in [
            ("20260922T01Z-plan", Kind::Plan),
            ("20260922T02Z-plan-2", Kind::Plan),
            ("20260922T03Z-diff", Kind::Diff),
        ] {
            let mut gate = gate(kind);
            gate.id = id.to_string();
            save(&ctx, Shelf::Open, &gate).unwrap();
        }
        let ids = ids(list_of_kind(
            &ctx.state,
            &ctx.repo.slug,
            Shelf::Open,
            Kind::Plan,
        ));
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert!(ids.iter().all(|id| id.contains("-plan")), "{ids:?}");
    }

    /// The `answered` subdirectory lives inside the gate directory, so it must not read as
    /// a gate itself.
    #[test]
    fn the_archive_is_not_listed_as_an_open_gate() {
        let (_sandbox, ctx) = hub();
        let gate = gate(Kind::Plan);
        save(&ctx, Shelf::Open, &gate).unwrap();
        archive(&ctx, &gate).unwrap();
        assert!(list(&ctx.state, &ctx.repo.slug, Shelf::Open).is_empty());
        assert!(exists(
            &ctx.state,
            &ctx.repo.slug,
            Shelf::Answered,
            &gate.id
        ));
        assert!(!exists(&ctx.state, &ctx.repo.slug, Shelf::Open, &gate.id));
    }

    /// The board reads `answered/` on every poll, so the archived gate is staged and renamed
    /// rather than written in place, and nothing of the staging is left behind.
    #[test]
    fn archiving_leaves_only_the_answered_file() {
        let (_sandbox, ctx) = hub();
        let gate = gate(Kind::Plan);
        save(&ctx, Shelf::Open, &gate).unwrap();
        archive(&ctx, &gate).unwrap();
        let names: Vec<String> = std::fs::read_dir(dir_of(&ctx, Shelf::Answered))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, [format!("{}.json", gate.id)]);
        assert!(!path_of(&dir_of(&ctx, Shelf::Open), &gate.id).exists());
    }

    /// A write in flight is a dotfile beside the gates, and it is not a gate yet.
    #[test]
    fn a_staged_file_is_not_listed() {
        let (_sandbox, ctx) = hub();
        let open = dir_of(&ctx, Shelf::Open);
        std::fs::create_dir_all(&open).unwrap();
        let json = serde_json::to_string(&gate(Kind::Plan)).unwrap();
        crate::infra::fs::stage(&open, &json).unwrap();
        assert!(list(&ctx.state, &ctx.repo.slug, Shelf::Open).is_empty());
    }

    #[test]
    fn a_gate_is_found_open_first_then_as_a_record() {
        let (_sandbox, ctx) = hub();
        let (root, slug) = (&ctx.state, &ctx.repo.slug);
        let mut open = gate(Kind::Diff);
        open.id = "same".to_string();
        let mut record = open.clone();
        record.title = "the record".to_string();
        record.wait = false;
        save(&ctx, Shelf::Record, &record).unwrap();
        assert_eq!(get(root, slug, "same").unwrap(), record);
        save(&ctx, Shelf::Open, &open).unwrap();
        assert_eq!(get(root, slug, "same").unwrap(), open);
        assert_eq!(
            get(root, slug, "missing").unwrap_err(),
            "no open gate or record: missing"
        );
    }

    #[test]
    fn a_broken_file_is_skipped_rather_than_blanking_the_queue() {
        let (_sandbox, ctx) = hub();
        save(&ctx, Shelf::Open, &gate(Kind::Plan)).unwrap();
        std::fs::write(dir_of(&ctx, Shelf::Open).join("broken.json"), "{ not json").unwrap();
        assert_eq!(list(&ctx.state, &ctx.repo.slug, Shelf::Open).len(), 1);
    }

    /// The identifier leads, because it is the only thing an agent reading its outbox can
    /// match an answer against.
    #[test]
    fn the_subject_leads_with_the_gate_id() {
        assert_eq!(
            answer_subject(&gate(Kind::Plan), "approve"),
            "[gate 20260922T041233Z-plan] approve"
        );
    }

    #[test]
    fn the_answer_body_names_the_decision_and_the_chosen_option() {
        let body = answer_body(
            &gate(Kind::Plan),
            "choice",
            Some("const"),
            Some("Go ahead with this"),
        );
        assert!(body.contains("## Decision   choice"), "{body}");
        assert!(body.contains("Option A — a constant (const)"), "{body}");
        assert!(body.contains("Go ahead with this"), "{body}");
    }

    /// An agent that sees no comment section cannot tell "there was none" from "one was
    /// lost on the way".
    #[test]
    fn a_missing_comment_is_said_rather_than_left_out() {
        let body = answer_body(&gate(Kind::Diff), "approve", None, None);
        assert!(body.contains("## Comment\n\n(none)"), "{body}");
    }

    #[test]
    fn a_record_s_id_is_told_apart_from_a_gate_s() {
        let (_sandbox, ctx) = hub();
        assert_eq!(
            claim_id(&ctx, Shelf::Record, "20260922T041233Z", Kind::Diff).unwrap(),
            "20260922T041233Z-diff-record"
        );
    }

    /// The structured fields are what the board draws its tables from, so they have to come
    /// back out exactly as the worker wrote them.
    #[test]
    fn a_record_with_its_structured_fields_reads_back_the_same() {
        let (_sandbox, ctx) = hub();
        let mut gate = gate(Kind::Diff);
        gate.wait = false;
        gate.review_rounds = vec![ReviewRound {
            engine: "claude".to_string(),
            must: 2,
            want: 1,
            scope: 0,
            false_positives: 1,
        }];
        gate.findings = vec![Finding {
            severity: Severity::Must,
            location: "src/gate.rs:10".to_string(),
            text: "unwrap on a missing file".to_string(),
            outcome: Outcome::Declined,
            reason: Some("the file is created just above".to_string()),
        }];
        gate.answers = vec![Answer {
            decision: "changes".to_string(),
            comment: Some("Please look at it again".to_string()),
            answered_at: "20260922T050000Z".to_string(),
        }];
        save(&ctx, Shelf::Record, &gate).unwrap();
        assert_eq!(
            load(&ctx.state, &ctx.repo.slug, Shelf::Record, &gate.id).unwrap(),
            gate
        );

        let json = serde_json::to_value(&gate).unwrap();
        assert_eq!(json["wait"], false);
        assert_eq!(json["reviewRounds"][0]["falsePositives"], 1);
        assert_eq!(json["findings"][0]["outcome"], "declined");
    }

    /// A gate written before records existed has no `wait`, and waits.
    #[test]
    fn a_gate_without_wait_is_one_that_waits() {
        let mut json = serde_json::to_value(gate(Kind::Plan)).unwrap();
        assert!(json.get("wait").is_none(), "{json}");
        json.as_object_mut().unwrap().remove("wait");
        assert!(serde_json::from_value::<Gate>(json).unwrap().wait);
    }

    #[test]
    fn an_answer_to_a_record_says_it_is_one() {
        let mut gate = gate(Kind::Verify);
        gate.wait = false;
        let body = answer_body(&gate, "changes", None, Some("Please fix it"));
        assert!(body.contains("(verify, record)"), "{body}");
    }

    /// A report is read, not approved, so it must not come with an Approve button.
    #[test]
    fn a_result_gate_offers_reading_rather_than_approving() {
        assert_eq!(Kind::Result.default_options(), ["ack", "ask"]);
        assert_eq!(Kind::Diff.default_options(), ["approve", "changes"]);
    }

    /// A plan the hub opened for a task handed to Jules sits in a worktree with no worker, so
    /// its answer has to reach the hub. One a worker opened stays the worker's.
    #[test]
    fn a_plan_is_answered_by_whoever_opened_it() {
        let mut plan = gate(Kind::Plan);
        assert!(!plan.answered_by_hub());
        plan.opened_by = Opener::Hub;
        assert!(plan.answered_by_hub());
        assert!(gate(Kind::Dispatch).answered_by_hub());
    }

    /// Written only when it says something: every gate a worker opens reads as it always has.
    #[test]
    fn the_opener_is_written_only_for_the_hub() {
        let worker = serde_json::to_value(gate(Kind::Plan)).unwrap();
        assert!(worker.get("openedBy").is_none());
        let mut plan = gate(Kind::Plan);
        plan.opened_by = Opener::Hub;
        let hub = serde_json::to_value(&plan).unwrap();
        assert_eq!(hub["openedBy"], "hub");
        let back: Gate = serde_json::from_value(hub).unwrap();
        assert_eq!(back, plan);
    }
}
