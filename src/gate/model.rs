//! What a gate is: the record, its vocabulary, the resume rule and the wording of an answer.

use serde::{Deserialize, Serialize};

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
    /// version and saved by this one loses nothing. Only ever filled from disk: `open` starts it
    /// empty.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// What a caller asks for when it opens a gate.
///
/// Not a `Gate`: the id and `openedAt` are written by `open`, and the decision, choice,
/// comment, `answeredAt` and answers by whoever answers it, so none of them is here for a
/// payload to carry in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRequest {
    pub kind: Kind,
    pub title: String,
    /// Where the answer goes. `None` when the caller did not say; `open` refuses that.
    pub worktree: Option<String>,
    pub opened_by: Opener,
    pub task: Option<String>,
    pub wait: bool,
    pub stopped_by: Vec<StopRule>,
    /// `None` for the kind's default buttons.
    pub options: Option<Vec<String>>,
    pub facts: Vec<String>,
    pub focus: Option<String>,
    pub decided: Option<String>,
    pub unsure: Option<String>,
    pub body: Option<String>,
    pub run: Option<String>,
    pub diff: Option<String>,
    pub choices: Vec<Choice>,
    pub rounds: u32,
    pub problem: Option<String>,
    pub goal: Option<String>,
    pub review_rounds: Vec<ReviewRound>,
    pub findings: Vec<Finding>,
    pub commands: Vec<CommandRun>,
    pub manual: Vec<String>,
}

/// The keys of a payload that are shown as they are. The rest of a `GateRequest` is read
/// by hand in `from_json`, so that a wrong one is named before anything is written.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Shown {
    task: Option<String>,
    options: Option<Vec<String>>,
    facts: Vec<String>,
    focus: Option<String>,
    decided: Option<String>,
    unsure: Option<String>,
    body: Option<String>,
    run: Option<String>,
    diff: Option<String>,
    problem: Option<String>,
    goal: Option<String>,
    choices: Vec<Choice>,
    rounds: u32,
    review_rounds: Vec<ReviewRound>,
    findings: Vec<Finding>,
    commands: Vec<CommandRun>,
    manual: Vec<String>,
}

impl GateRequest {
    /// Read a request out of a JSON payload. Keys a gate does not take from a caller are
    /// ignored, and the ones that are wrong are refused here, in the words `adj gate open`
    /// has always used.
    pub fn from_json(payload: &serde_json::Value) -> Result<GateRequest, String> {
        let kind = payload
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .ok_or("a gate needs a kind")?;
        let kind: Kind = serde_json::from_value(serde_json::json!(kind))
            .map_err(|_| format!("no such gate kind: {kind}"))?;
        // Checked before the rest so the error names the field, rather than arriving as a
        // serde message about a struct the caller never saw.
        let title = match payload.get("title").and_then(serde_json::Value::as_str) {
            Some(t) if !t.trim().is_empty() => t.to_string(),
            _ => return Err("a gate needs a title".to_string()),
        };
        let opened_by: Opener = match payload.get("openedBy") {
            None | Some(serde_json::Value::Null) => Opener::Worker,
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|_| format!("no such opener: {value} (worker or hub)"))?,
        };
        let wait = match payload.get("wait") {
            None | Some(serde_json::Value::Null) => true,
            Some(serde_json::Value::Bool(wait)) => *wait,
            Some(_) => return Err("wait must be true or false".to_string()),
        };
        // `null` too: it is a present value of the wrong type, not a missing one.
        let stopped_by = match payload.get("stoppedBy") {
            None => Vec::new(),
            Some(serde_json::Value::Array(rules)) => rules
                .iter()
                .map(|rule| {
                    serde_json::from_value(rule.clone())
                        .map_err(|_| format!("no such stop rule: {rule}"))
                })
                .collect::<Result<Vec<StopRule>, String>>()?,
            Some(_) => return Err("stoppedBy must be a list of rules".to_string()),
        };
        let worktree = payload
            .get("worktree")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let shown: Shown =
            serde_json::from_value(payload.clone()).map_err(|e| format!("bad gate: {e}"))?;
        Ok(GateRequest {
            kind,
            title,
            worktree,
            opened_by,
            task: shown.task,
            wait,
            stopped_by,
            options: shown.options,
            facts: shown.facts,
            focus: shown.focus,
            decided: shown.decided,
            unsure: shown.unsure,
            body: shown.body,
            run: shown.run,
            diff: shown.diff,
            choices: shown.choices,
            rounds: shown.rounds,
            problem: shown.problem,
            goal: shown.goal,
            review_rounds: shown.review_rounds,
            findings: shown.findings,
            commands: shown.commands,
            manual: shown.manual,
        })
    }
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
