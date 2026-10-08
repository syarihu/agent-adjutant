//! The records and answers `registry` deals in, and the pure helpers over them.

use super::*;
use crate::infra::terminal::SessionTerminal;
use serde::{Deserialize, Serialize};
use serde_json::Map;
use std::collections::BTreeMap;

// ── presence ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubStatus {
    pub slug: String,
    pub hub_name: String,
    pub present: bool,
    pub pid: Option<u32>,
    pub cwd: Option<String>,
    pub started_at: Option<String>,
    /// A record was found but the process behind it is gone.
    pub stale: bool,
    /// Where the hub was started, when its record says (#150).
    pub terminal: Option<SessionTerminal>,
}

/// The answer to "may this process be the hub for `slug`?".
#[derive(Debug)]
pub enum Claim {
    /// The record is ours. The caller may go on to become the agent. Where it landed is
    /// `hub_record_path`, so the claim does not need to hand it back.
    Ours,
    /// Another hub holds the record and is alive. Its status, so the caller can say which.
    Taken(Box<HubStatus>),
}

/// What a record file can be: a hub's, a worker's, or the identity read out of a worker's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded<T> {
    /// Nothing is there.
    Absent,
    /// Something is there and it cannot be read as a record: not JSON, not an object, or not
    /// readable at all. Not evidence that nobody holds the name.
    Unreadable,
    Found(T),
}

/// What `hubs/<slug>.json` says, read leniently.
///
/// Records are written by `claim_hub` but also edited by hand and left over from older
/// versions, so every field is optional and a key whose value is of another type is not an
/// error: it stays in `other` and the rest still reads. Each reader then falls back as it
/// did when it looked the key up in raw JSON, rather than treating a record with one bad
/// key as unreadable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubRecord {
    pub pid: Option<u64>,
    pub hub_name: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<String>,
    /// As written: untrimmed, possibly blank. `recorded_anchor` is the reading of it.
    pub ps_started: Option<String>,
    /// Untrimmed, as the scans read it.
    pub hub: Option<String>,
    pub name_in_command: Option<bool>,
    pub terminal: Option<SessionTerminal>,
    /// Every key not read into a field above: unknown ones, and known ones whose value is
    /// null or of another type.
    pub other: Map<String, Value>,
}

/// Take `key` out of `fields` when its value reads as `T`, and leave it there when it does not.
pub(super) fn lift<T>(
    fields: &mut Map<String, Value>,
    key: &str,
    read: impl Fn(&Value) -> Option<T>,
) -> Option<T> {
    let read = fields.get(key).and_then(read)?;
    fields.remove(key);
    Some(read)
}

impl HubRecord {
    /// `None` only for something that is not a JSON object.
    pub(super) fn from_value(value: Value) -> Option<HubRecord> {
        let Value::Object(mut fields) = value else {
            return None;
        };
        // A key leaves `fields` only when its value reads as the field's type, so what is
        // left over is exactly what the typed fields do not account for.
        let text = |value: &Value| value.as_str().map(str::to_string);
        Some(HubRecord {
            pid: lift(&mut fields, "pid", Value::as_u64),
            hub_name: lift(&mut fields, "hubName", text),
            cwd: lift(&mut fields, "cwd", text),
            started_at: lift(&mut fields, "startedAt", text),
            ps_started: lift(&mut fields, "psStarted", text),
            hub: lift(&mut fields, "hub", text),
            name_in_command: lift(&mut fields, "nameInCommand", Value::as_bool),
            terminal: lift(&mut fields, "terminal", |t| {
                serde_json::from_value::<SessionTerminal>(t.clone()).ok()
            }),
            other: fields,
        })
    }

    /// The JSON `claim_hub` writes, and only for it: a record built with an empty `other`.
    ///
    /// A record that was read is never written back. Its wrong-typed known keys sit in
    /// `other` and would be overwritten here by `null` for the typed field that is `None`,
    /// so the round trip would change a file nobody meant to change.
    pub(super) fn to_value(&self) -> Value {
        let mut fields = self.other.clone();
        fields.insert("pid".to_string(), json!(self.pid));
        fields.insert("hubName".to_string(), json!(self.hub_name));
        fields.insert("cwd".to_string(), json!(self.cwd));
        fields.insert("startedAt".to_string(), json!(self.started_at));
        fields.insert("psStarted".to_string(), json!(self.ps_started));
        fields.insert("nameInCommand".to_string(), json!(self.name_in_command));
        if let Some(hub) = &self.hub {
            fields.insert("hub".to_string(), json!(hub));
        }
        if let Some(terminal) = &self.terminal
            && let Ok(value) = serde_json::to_value(terminal)
        {
            fields.insert("terminal".to_string(), value);
        }
        Value::Object(fields)
    }

    /// Whether the record names a process at all, even one `pid` could not read: a `pid` of
    /// the wrong type is somebody's claim to the name, and only a null or missing one is not.
    pub(super) fn names_a_pid(&self) -> bool {
        self.pid.is_some() || self.other.get("pid").is_some_and(|pid| !pid.is_null())
    }
}

/// `recorded_anchor` for a start time already lifted out of whatever record held it.
pub(super) fn anchor_of(started: Option<&str>) -> Option<&str> {
    started.map(str::trim).filter(|s| !s.is_empty())
}

/// Public because `close` needs the same three answers about a worker that `claim_hub`
/// needs about a hub, and for the same reason: it acts destructively on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Alive,
    Gone,
    /// `ps` could not be run, or the record could not be read. Not evidence of anything.
    CannotTell,
}

/// Blank is silence. An identifier that is empty or only spaces is a caller passing the
/// flag through without a value, and reading it as a hub called "" builds an address nobody
/// can type a second time.
pub(super) fn said(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|hub| !hub.is_empty())
        .map(str::to_string)
}

/// What a worker record says about where its worker reports: `None` for a repository's own
/// hub. Kept apart from "no record", which `worker_hub` answers with `None` around this.
pub(super) struct RecordedHub {
    pub(super) hub: Option<String>,
    /// The process that then `exec`s the agent, so the agent and the servers it starts are it
    /// or below it.
    pub(super) pid: Option<u32>,
}

/// What `.claude/adjutant-worker.json` says, read leniently, and rewritten with every key it had.
///
/// A worker record is written back (a phase said, a task linked), so unlike `HubRecord` what
/// this cannot read must survive the round trip: a key of another type, one this version does
/// not know, and the entries of `phases` it cannot read all stay as they were.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerRecord {
    pub pid: Option<u64>,
    pub title: Option<String>,
    pub started_at: Option<String>,
    /// As written: untrimmed, possibly blank. `anchor` is the reading of it.
    pub ps_started: Option<String>,
    /// Untrimmed, and only when a string. Null or another type stays in `other`, which is
    /// how `hub_is_not_a_name` tells them apart.
    pub hub: Option<String>,
    /// Untrimmed, as the readers trim it.
    pub task: Option<String>,
    pub phase: Option<String>,
    pub phase_at: Option<i64>,
    /// Entries as they are, readable or not, so a rewrite keeps one this version cannot read.
    pub phases: Option<Vec<Value>>,
    /// Every key not read into a field above: unknown ones, known ones that are null or of
    /// another type, and `terminal`, kept raw because `SessionTerminal` drops keys inside it
    /// that it does not know.
    pub other: Map<String, Value>,
}

impl WorkerRecord {
    /// `None` only for something that is not a JSON object.
    pub(super) fn from_value(value: Value) -> Option<WorkerRecord> {
        let Value::Object(mut fields) = value else {
            return None;
        };
        let text = |value: &Value| value.as_str().map(str::to_string);
        Some(WorkerRecord {
            pid: lift(&mut fields, "pid", Value::as_u64),
            title: lift(&mut fields, "title", text),
            started_at: lift(&mut fields, "startedAt", text),
            ps_started: lift(&mut fields, "psStarted", text),
            hub: lift(&mut fields, "hub", text),
            task: lift(&mut fields, "task", text),
            phase: lift(&mut fields, "phase", text),
            phase_at: lift(&mut fields, "phaseAt", Value::as_i64),
            phases: lift(&mut fields, "phases", |v| v.as_array().cloned()),
            other: fields,
        })
    }

    /// The JSON to write back: `other` as it was, and each field that has a value.
    ///
    /// Not what `HubRecord::to_value` does, which writes `null` for a field with none. A
    /// worker record is read and written again, so a key that was absent has to stay absent,
    /// and one that was null or of another type is in `other` and has to survive.
    pub(super) fn to_value(&self) -> Value {
        let mut fields = self.other.clone();
        if let Some(pid) = self.pid {
            fields.insert("pid".to_string(), Value::from(pid));
        }
        let texts = [
            ("title", &self.title),
            ("startedAt", &self.started_at),
            ("psStarted", &self.ps_started),
            ("hub", &self.hub),
            ("task", &self.task),
            ("phase", &self.phase),
        ];
        for (key, text) in texts {
            if let Some(text) = text {
                fields.insert(key.to_string(), json!(text));
            }
        }
        if let Some(at) = self.phase_at {
            fields.insert("phaseAt".to_string(), json!(at));
        }
        if let Some(phases) = &self.phases {
            fields.insert("phases".to_string(), Value::Array(phases.clone()));
        }
        Value::Object(fields)
    }

    /// Where the worker runs, as a record has always been read for it: a `terminal` that does
    /// not read as one is none.
    pub fn terminal(&self) -> Option<SessionTerminal> {
        self.other
            .get("terminal")
            .and_then(|t| serde_json::from_value(t.clone()).ok())
    }

    /// When the process started, for telling it from whatever is handed its pid later. Blank
    /// counts as absent.
    pub(super) fn anchor(&self) -> Option<&str> {
        anchor_of(self.ps_started.as_deref())
    }

    /// The pid as a process id that can be asked about: in range, and not 0. Out of range
    /// is refused rather than truncated, because a truncated pid names a live process that
    /// nobody asked about.
    pub(super) fn usable_pid(&self) -> Option<u32> {
        self.pid
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
    }

    /// Whether `hub` says something but not a name: a record that was dispatched and has lost
    /// its address, as opposed to one with no `hub` or a null one, which is the repository's
    /// own hub.
    pub(super) fn hub_is_not_a_name(&self) -> bool {
        self.hub.is_none() && self.other.get("hub").is_some_and(|hub| !hub.is_null())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerStatus {
    pub worktree: String,
    pub present: bool,
    pub pid: Option<u32>,
    pub title: Option<String>,
    pub stale: bool,
    /// What the worker last said it was doing (`adj phase --set`), and since when.
    pub phase: Option<String>,
    pub phase_at: Option<i64>,
    /// Every phase entered, oldest first, as `(phase, epoch seconds)`.
    pub phases: Vec<(String, i64)>,
    /// Where the worker was started, when its record says (#150).
    pub terminal: Option<SessionTerminal>,
}

/// How many entries a record's `phases` keeps. A worker that says a phase on every step of a
/// long task would otherwise grow a file the board reads every two seconds.
pub(super) const PHASES_KEPT: usize = 64;

/// The history a record has: its `phases`, or the one entry its `phase` and `phaseAt` make
/// for a record written before it kept a history.
pub(super) fn recorded_phases(record: &WorkerRecord) -> Vec<Value> {
    if let Some(phases) = &record.phases {
        return phases.clone();
    }
    match (&record.phase, record.phase_at) {
        (Some(phase), Some(at)) => vec![json!([phase, at])],
        _ => Vec::new(),
    }
}

/// Enter `phase` at `at` in the record: the current phase, and one more entry in the
/// history — even for a phase already said, because the time it was said again is a fact too.
pub(super) fn append_phase(record: &mut WorkerRecord, phase: &str, at: i64) {
    let mut phases = recorded_phases(record);
    phases.push(json!([phase, at]));
    if phases.len() > PHASES_KEPT {
        phases.drain(..phases.len() - PHASES_KEPT);
    }
    record.phase = Some(phase.to_string());
    record.phase_at = Some(at);
    record.phases = Some(phases);
}

/// The steps a worker says it is in. A fixed list so the board can show them in order and a
/// typo is refused rather than shown as a phase of its own. `pr-bots` is a PR waiting on review
/// bots, which nobody has to act on; `pr` is one handed to human reviewers.
pub const PHASES: [&str; 8] = [
    "plan",
    "implement",
    "self-review",
    "verify",
    "pr",
    "pr-bots",
    "review",
    "report",
];

/// What is kept of the agent's last message of a turn (`lastMessage`): a paragraph or two, line
/// breaks and all. The hook cuts it on write and the board cuts it again for rows of other binaries.
pub const LAST_MESSAGE_CHARS: usize = 1000;

/// How long a worker that was dispatched but has not registered yet still holds its slot.
///
/// Registration happens inside the new tab, seconds after `adj work` returns. A hub that
/// dispatches three in one turn would otherwise count none of the first two and go past the
/// limit. A minute covers a slow terminal; past that the tab most likely never started, and
/// a slot held for a worker that does not exist is the one mistake a limit must not make.
pub const STARTING_GRACE_SECS: i64 = 60;

/// The worker a record names, as an individual rather than as a file.
///
/// Read once and then carried, because everything a caller does about a worker has to be
/// about *one* process: a record re-read between asking whether the worker is alive and
/// acting on the answer can have been rewritten by the next worker registering in the same
/// worktree, and then one worker's registration is answering a question asked about
/// another one's pid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerIdentity {
    pub pid: u32,
    /// What `ps` said about when that pid started, as the record has it. The anchor that
    /// tells this worker from whatever the system later hands that pid to.
    ///
    /// `None` in a record written at a moment when `ps` would not answer, and that record
    /// can never be acted on destructively: with no anchor there is nothing to tell this
    /// worker from the next owner of the pid, so `worker_liveness` answers `CannotTell`
    /// and the caller stops. Little is lost by it — a machine where `ps` cannot answer at
    /// registration time is one where it cannot answer at the check either, and that was
    /// already `CannotTell`; what closes is the narrow window where it failed only once.
    pub started: Option<String>,
    pub title: Option<String>,
    /// Where the worker was started, when its record says (#150).
    pub terminal: Option<SessionTerminal>,
}

/// What clearing a record came to.
pub enum Cleared {
    /// The record named this worker and is gone now — or was already gone, which is the
    /// same end state.
    Yes,
    /// It names another worker, one that registered in this worktree since.
    AnotherWorker,
    /// It cannot be read as naming anybody, so it is nobody's to remove.
    Unreadable,
}

// ── sessions: what `--resume` reopens ────────────────────────────────

/// The conversation a hub or a worker was started into, written down so it can be reopened
/// after the agent itself has gone — an update, a crash, a closed tab.
///
/// Kept apart from the presence records on purpose. Those say who is running *now*, and are
/// removed the moment nobody is: `hub-stop` clears the hub's, `close` the worker's, and a
/// takeover rewrites either. The session is wanted precisely after that has happened, so it
/// lives in a file nothing clears — a later start in the same place overwrites it, which is
/// the one thing that should.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSession {
    pub session_id: String,
    /// The hub identifier: which hub this was (for a hub), or which hub dispatched it (for a
    /// worker). `None` is the repository's own hub.
    pub hub: Option<String>,
    /// `owner/name`, for a hub. What lets `--resume` list a repository's resumable hubs when
    /// asked for one that has nothing saved.
    pub nwo: Option<String>,
    pub hub_name: Option<String>,
    /// The worker's tab title, so a resumed worker is named what it was named before.
    pub title: Option<String>,
    pub task: Option<String>,
    pub saved_at: Option<String>,
}

/// Where a board for a hub is being served from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Served {
    Resident(u16),
    Dedicated(u16),
}

/// The resident server when there is one, the hub's own board otherwise. The resident wins
/// because it is the one that outlives the hub, and a URL that named the hub's board would
/// stop working when the hub did.
pub(crate) fn prefer(resident: Option<u16>, dedicated: Option<u16>) -> Option<Served> {
    resident
        .map(Served::Resident)
        .or(dedicated.map(Served::Dedicated))
}

/// The pieces of the address book entry `slug` names, when they still describe that board:
/// the checkout is there and the repository and hub still come to the same slug.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Address {
    pub(crate) slug: String,
    pub(crate) main: String,
    pub(crate) nwo: String,
    pub(crate) hub: Option<String>,
}

// ── the agent session ledger ─────────────────────────────────────────

/// What an agent's hooks last said about one session, as `agent-sessions/<id>.json` keeps it.
///
/// Every field but the key is optional and keys this version does not know stay in `other`
/// (rule 10 in `docs/architecture.md`). `model`, `contextPercent` and `rateLimits` come from the
/// status line (`adj hook claude --status-line`), never from a hook; `lastMessage` is the
/// opposite, from the `Stop` and `StopFailure` hooks. A known key of the wrong
/// type fails the load, and the row is then moved aside like any unreadable one.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    #[serde(default)]
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentStatus>,
    /// A `done` or `failed` held back while sub-agents run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_status: Option<AgentStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// The agent process's start time as `ps` prints it, opaque like every other `psStarted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ps_started: Option<String>,
    /// Epoch seconds, as are the other times here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    /// Moves only when `status` changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// The "seen alive" mark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<Subagent>,
    /// `agent_id` to when it stopped, so a late event cannot bring one back.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub finished_subagents: BTreeMap<String, i64>,
    /// What the status line last showed, as the person sees it: the model's display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// A whole number of percent of the context window in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limits: Option<RateLimits>,
    /// What the agent said at the end of its last turn that said anything, kept until the next
    /// one replaces it. Line breaks are kept; the receiver caps its length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    /// When `last_message` was received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<i64>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// The account's rate limit windows, as the status line last showed them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<RateWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<RateWindow>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateWindow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    /// Epoch seconds, as Claude Code gives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subagent {
    pub id: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<i64>,
    /// The tool it last ran, as the parent's `activity` words one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// A status this version names, or the string a newer one wrote, kept as it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum AgentStatus {
    Idle,
    Running,
    Waiting,
    Done,
    Failed,
    Other(String),
}

impl AgentStatus {
    pub fn as_str(&self) -> &str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Running => "running",
            AgentStatus::Waiting => "waiting",
            AgentStatus::Done => "done",
            AgentStatus::Failed => "failed",
            AgentStatus::Other(text) => text,
        }
    }
}

impl From<String> for AgentStatus {
    fn from(text: String) -> Self {
        match text.as_str() {
            "idle" => AgentStatus::Idle,
            "running" => AgentStatus::Running,
            "waiting" => AgentStatus::Waiting,
            "done" => AgentStatus::Done,
            "failed" => AgentStatus::Failed,
            _ => AgentStatus::Other(text),
        }
    }
}

impl From<AgentStatus> for String {
    fn from(status: AgentStatus) -> Self {
        status.as_str().to_string()
    }
}

/// A session id that is safe as a file name: it keys `agent-sessions/<id>.json`, and an id with
/// a `/` or a `..` in it would name a file outside the directory.
pub fn valid_session_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// How long a `finishedSubagents` entry is kept.
const FINISHED_KEPT_SECS: i64 = 300;
/// A sub-agent not seen for this long never sent its `SubagentStop`.
const SUBAGENT_SILENT_SECS: i64 = 600;
/// `lastEventAt` and a sub-agent's `lastSeenAt` move at most this often on their own.
const HEARTBEAT_SECS: i64 = 60;

/// What `apply` still has to be told, because it takes no part in running a process.
#[derive(Debug, Default)]
pub(super) struct Lookups {
    /// A `cwd` and the worktree git named for it, `None` outside git.
    pub worktree: Option<(String, Option<String>)>,
    /// A pid and its start time, `None` when `ps` did not say.
    pub started: Option<(u32, Option<String>)>,
}

/// The lookups an event needs for a row, which are the ones whose input differs from the row's.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Wanted {
    pub cwd: Option<String>,
    pub pid: Option<u32>,
}

impl Wanted {
    /// What of this `lookups` has not answered yet.
    pub(super) fn unanswered(&self, lookups: &Lookups) -> Wanted {
        Wanted {
            cwd: self
                .cwd
                .clone()
                .filter(|cwd| lookups.worktree.as_ref().map(|(asked, _)| asked) != Some(cwd)),
            pid: self
                .pid
                .filter(|pid| lookups.started.as_ref().map(|(asked, _)| asked) != Some(pid)),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.cwd.is_none() && self.pid.is_none()
    }
}

pub(super) enum Applied {
    /// Nothing to write.
    Unchanged,
    Write(Box<AgentSession>),
    Remove,
    /// A lookup the event needs has not been made: release the lock, make it, apply again.
    NeedsLookup,
}

/// Whether the event comes from a sub-agent rather than the session itself: it names an
/// `agent_id` and is not one of the two events about a sub-agent's life, or a notification
/// (which is always the parent's).
fn from_subagent(event: &AgentEvent) -> bool {
    event.agent_id.is_some()
        && !matches!(
            event.hook,
            HookEvent::SubagentStart | HookEvent::SubagentStop | HookEvent::Notification { .. }
        )
}

/// What a notification of this type says: that the person is wanted, that the agent is idle
/// again, or nothing about the session's state.
enum Notified {
    Waiting,
    Idle,
    Nothing,
}

fn notified(kind: Option<&str>) -> Notified {
    match kind {
        Some("idle_prompt") => Notified::Idle,
        Some(
            "auth_success"
            | "elicitation_complete"
            | "elicitation_response"
            | "agent_completed"
            | "quota_auto_resume_fired"
            | "quota_auto_resume_stale"
            | "quota_auto_resume_disabled",
        ) => Notified::Nothing,
        // The prompts, and anything this version does not know: asking the person is the
        // reading that costs least when it is wrong.
        _ => Notified::Waiting,
    }
}

/// Whether an event for a session with no row makes one. A stray `Stop` or `SessionEnd` from a
/// session that ended before its row was written never does.
pub(super) fn can_create(event: &AgentEvent) -> bool {
    if from_subagent(event) {
        return matches!(
            event.hook,
            HookEvent::PostToolUse | HookEvent::PostToolUseFailure | HookEvent::PermissionRequest
        );
    }
    match &event.hook {
        HookEvent::SessionStart
        | HookEvent::UserPromptSubmit
        | HookEvent::PostToolUse
        | HookEvent::PostToolUseFailure
        | HookEvent::PermissionRequest => true,
        HookEvent::SubagentStart => event.agent_id.is_some(),
        HookEvent::Notification { kind, .. } => {
            matches!(notified(kind.as_deref()), Notified::Waiting)
        }
        HookEvent::Stop { .. }
        | HookEvent::StopFailure { .. }
        | HookEvent::SessionEnd
        | HookEvent::SubagentStop
        | HookEvent::StatusLine { .. }
        | HookEvent::Other(_) => false,
    }
}

/// A late event from a sub-agent that has already stopped: it must not bring it back.
fn is_late(row: Option<&AgentSession>, event: &AgentEvent) -> bool {
    match (row, &event.agent_id) {
        (Some(row), Some(id)) => {
            from_subagent(event) && row.finished_subagents.contains_key(id.as_str())
        }
        _ => false,
    }
}

/// The lookups `event` needs to be applied to `row`: the worktree when the `cwd` is new to the
/// row, the start time when the pid is.
pub(super) fn wanted(row: Option<&AgentSession>, event: &AgentEvent) -> Wanted {
    if is_late(row, event) {
        return Wanted::default();
    }
    Wanted {
        // A sub-agent may work elsewhere (its own worktree); the row says where the session is.
        cwd: event
            .cwd
            .clone()
            .filter(|_| !from_subagent(event))
            .filter(|cwd| row.and_then(|r| r.cwd.as_ref()) != Some(cwd)),
        // Also for a pid whose start time was never read: `ps` failing once must not stick.
        pid: event.pid.filter(|pid| {
            row.and_then(|r| r.pid) != Some(*pid) || row.is_some_and(|r| r.ps_started.is_none())
        }),
    }
}

fn new_row(event: &AgentEvent) -> AgentSession {
    AgentSession {
        session_id: event.session_id.clone(),
        agent: Some(event.agent.clone()),
        created_at: Some(event.at),
        updated_at: Some(event.at),
        last_event_at: Some(event.at),
        ..AgentSession::default()
    }
}

/// Move to `status`. The text that belongs to the old one goes: a request only means something
/// while waiting, and the tool in use means nothing once the turn is over.
fn set_status(row: &mut AgentSession, status: AgentStatus) {
    if status != AgentStatus::Waiting {
        row.request = None;
    }
    if matches!(
        status,
        AgentStatus::Idle | AgentStatus::Done | AgentStatus::Failed
    ) {
        row.activity = None;
    }
    row.status = Some(status);
}

fn apply_pending(row: &mut AgentSession) {
    if let Some(pending) = row.pending_status.take() {
        set_status(row, pending);
    }
}

/// What a turn's last message was, when it had one. Kept even while `done` is held for
/// sub-agents: it is said, whatever the status shows. A turn without one leaves the last, and
/// the same words again within the heartbeat leave their time, so that a repeated `Stop` is not
/// a write of its own.
fn remember_message(row: &mut AgentSession, message: Option<&str>, now: i64) {
    let Some(message) = message else {
        return;
    };
    let repeated = row.last_message.as_deref() == Some(message)
        && row
            .last_message_at
            .is_some_and(|at| now - at < HEARTBEAT_SECS);
    if !repeated {
        row.last_message = Some(message.to_string());
        row.last_message_at = Some(now);
    }
}

/// What `Stop` and `StopFailure` say, held while sub-agents are still running.
fn finish(row: &mut AgentSession, status: AgentStatus) {
    if row.subagents.is_empty() {
        row.pending_status = None;
        set_status(row, status);
    } else {
        row.pending_status = Some(status);
    }
}

/// Forget what is too old to matter: stop entries kept for five minutes, and sub-agents whose
/// `SubagentStop` never came. The parent's held `done` goes through once the last one is gone.
fn tidy(row: &mut AgentSession, now: i64) {
    row.finished_subagents
        .retain(|_, stopped| now - *stopped <= FINISHED_KEPT_SECS);
    let before = row.subagents.len();
    let mut dropped = Vec::new();
    row.subagents.retain(|sub| {
        let seen = sub.last_seen_at.or(sub.started_at);
        let silent = seen.is_some_and(|seen| now - seen >= SUBAGENT_SILENT_SECS);
        if silent {
            dropped.push(sub.id.clone());
        }
        !silent
    });
    for id in dropped {
        row.finished_subagents.insert(id, now);
    }
    if before > 0 && row.subagents.is_empty() {
        apply_pending(row);
    }
}

/// Note that sub-agent `id` is alive: add it, or move its `lastSeenAt` if it is a minute old.
fn see_subagent(row: &mut AgentSession, id: &str, kind: Option<&str>, now: i64) {
    match row.subagents.iter_mut().find(|sub| sub.id == id) {
        Some(sub) => {
            let seen = sub.last_seen_at.or(sub.started_at);
            if seen.is_none_or(|seen| now - seen >= HEARTBEAT_SECS) {
                sub.last_seen_at = Some(now);
            }
        }
        None => row.subagents.push(Subagent {
            id: id.to_string(),
            kind: kind.map(str::to_string),
            started_at: Some(now),
            last_seen_at: Some(now),
            activity: None,
            other: Map::new(),
        }),
    }
}

fn identify(row: &mut AgentSession, event: &AgentEvent, lookups: &Lookups) {
    if let Some(pid) = event.pid
        && (row.pid != Some(pid) || row.ps_started.is_none())
    {
        row.pid = Some(pid);
        row.ps_started = lookups
            .started
            .as_ref()
            .filter(|(asked, _)| *asked == pid)
            .and_then(|(_, started)| started.clone());
    }
    if let Some(cwd) = &event.cwd
        && !from_subagent(event)
        && row.cwd.as_ref() != Some(cwd)
    {
        row.cwd = Some(cwd.clone());
        row.worktree = lookups
            .worktree
            .as_ref()
            .filter(|(asked, _)| asked == cwd)
            .and_then(|(_, worktree)| worktree.clone());
    }
    if let Some(dir) = &event.config_dir
        && row.config_dir.as_ref() != Some(dir)
    {
        row.config_dir = Some(dir.clone());
    }
    if row.agent.is_none() {
        row.agent = Some(event.agent.clone());
    }
}

/// What `event` does to the session's row, which is `row` (`None` for no row). Pure: the time is
/// the event's own, and what git and `ps` said comes in as `lookups`.
pub(super) fn apply(row: Option<AgentSession>, event: &AgentEvent, lookups: &Lookups) -> Applied {
    let now = event.at;
    match (&event.hook, &row) {
        (HookEvent::Other(_), _) => return Applied::Unchanged,
        (HookEvent::SessionEnd, Some(_)) => return Applied::Remove,
        (HookEvent::SessionEnd, None) => return Applied::Unchanged,
        (_, None) if !can_create(event) => return Applied::Unchanged,
        _ => {}
    }
    if !wanted(row.as_ref(), event).unanswered(lookups).is_empty() {
        return Applied::NeedsLookup;
    }
    let late = is_late(row.as_ref(), event);
    let old = row.clone();
    let mut cur = row.unwrap_or_else(|| new_row(event));
    tidy(&mut cur, now);
    if !late {
        identify(&mut cur, event, lookups);
        match (&event.agent_id, from_subagent(event)) {
            // Only these are a sub-agent at work. Any other event that names one changes
            // nothing: it could add a sub-agent nobody removes.
            (Some(id), true) => {
                if matches!(
                    event.hook,
                    HookEvent::PostToolUse
                        | HookEvent::PostToolUseFailure
                        | HookEvent::PermissionRequest
                ) {
                    see_subagent(&mut cur, id, event.agent_type.as_deref(), now);
                    apply_from_subagent(&mut cur, id, event);
                }
            }
            _ => apply_from_session(&mut cur, event, now),
        }
    }
    if cur.status != old.as_ref().and_then(|row| row.status.clone()) {
        cur.updated_at = Some(now);
    }
    // A change is anything but `lastEventAt`; with none, the write is skipped until that mark is
    // a minute old, so a busy session does not rewrite its row on every tool call.
    let Some(old) = old else {
        cur.last_event_at = Some(now);
        return Applied::Write(Box::new(cur));
    };
    let (mut before, mut after) = (old.clone(), cur.clone());
    before.last_event_at = None;
    after.last_event_at = None;
    let stale = old
        .last_event_at
        .is_none_or(|seen| now - seen >= HEARTBEAT_SECS);
    if before != after || stale {
        cur.last_event_at = Some(now);
        Applied::Write(Box::new(cur))
    } else {
        Applied::Unchanged
    }
}

/// An event from a sub-agent (already noted by `see_subagent`): it leaves the parent's status
/// alone, except that the person is asked in the parent's terminal either way.
fn apply_from_subagent(row: &mut AgentSession, id: &str, event: &AgentEvent) {
    match &event.hook {
        HookEvent::PermissionRequest => {
            set_status(row, AgentStatus::Waiting);
            row.request = event.summary.clone();
        }
        HookEvent::PostToolUse | HookEvent::PostToolUseFailure => {
            match row.status {
                // The prompt was answered, or the sub-agent would not be running tools.
                Some(AgentStatus::Waiting) | None => set_status(row, AgentStatus::Running),
                _ => {}
            }
            // Its own tool, not the parent's: `activity` of the row stays the parent's.
            if let Some(summary) = &event.summary
                && let Some(sub) = row.subagents.iter_mut().find(|sub| sub.id == id)
                && sub.activity.as_ref() != Some(summary)
            {
                sub.activity = Some(summary.clone());
            }
        }
        _ => {}
    }
}

/// What a draw gave of one rate limit window over what the row has: a field the draw left out,
/// or a key this version does not know, stays as it was.
fn merge_window(stored: &mut Option<RateWindow>, drawn: Option<&RateWindow>) {
    let Some(drawn) = drawn else {
        return;
    };
    let window = stored.get_or_insert_with(RateWindow::default);
    if drawn.used_percent.is_some() {
        window.used_percent = drawn.used_percent;
    }
    if drawn.resets_at.is_some() {
        window.resets_at = drawn.resets_at;
    }
    window.other.extend(
        drawn
            .other
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
}

fn apply_from_session(row: &mut AgentSession, event: &AgentEvent, now: i64) {
    match &event.hook {
        HookEvent::SessionStart => match row.status {
            // A new row. On an existing one it fires on resume, compaction and `/clear` too,
            // and must not wipe a turn in progress.
            None => set_status(row, AgentStatus::Idle),
            Some(_) => row.request = None,
        },
        HookEvent::UserPromptSubmit => {
            row.pending_status = None;
            set_status(row, AgentStatus::Running);
        }
        HookEvent::PostToolUse | HookEvent::PostToolUseFailure => {
            // A new turn: what was held from the last one is not for this one.
            row.pending_status = None;
            set_status(row, AgentStatus::Running);
            if event.summary.is_some() {
                row.activity = event.summary.clone();
            }
        }
        HookEvent::PermissionRequest => {
            row.pending_status = None;
            set_status(row, AgentStatus::Waiting);
            row.request = event.summary.clone();
        }
        HookEvent::Notification { kind, message } => match notified(kind.as_deref()) {
            Notified::Waiting => {
                set_status(row, AgentStatus::Waiting);
                if row.request.is_none() {
                    row.request = message.clone();
                }
            }
            Notified::Idle => {
                if row.status == Some(AgentStatus::Waiting) {
                    set_status(row, AgentStatus::Idle);
                }
            }
            Notified::Nothing => {}
        },
        HookEvent::Stop { message } => {
            remember_message(row, message.as_deref(), now);
            finish(row, AgentStatus::Done);
        }
        HookEvent::StopFailure { message } => {
            remember_message(row, message.as_deref(), now);
            finish(row, AgentStatus::Failed);
        }
        HookEvent::SubagentStart => {
            if let Some(id) = &event.agent_id
                && !row.finished_subagents.contains_key(id)
            {
                see_subagent(row, id, event.agent_type.as_deref(), now);
            }
            // A row made by a sub-agent starting is busy; an existing one keeps its status.
            if row.status.is_none() {
                set_status(row, AgentStatus::Running);
            }
        }
        HookEvent::SubagentStop => {
            if let Some(id) = &event.agent_id {
                row.subagents.retain(|sub| &sub.id != id);
                row.finished_subagents.insert(id.clone(), now);
                if row.subagents.is_empty() {
                    apply_pending(row);
                }
            }
        }
        HookEvent::StatusLine {
            model,
            context_percent,
            five_hour,
            seven_day,
        } => {
            if model.is_some() {
                row.model = model.clone();
            }
            if context_percent.is_some() {
                row.context_percent = *context_percent;
            }
            if five_hour.is_some() || seven_day.is_some() {
                let limits = row.rate_limits.get_or_insert_with(RateLimits::default);
                merge_window(&mut limits.five_hour, five_hour.as_ref());
                merge_window(&mut limits.seven_day, seven_day.as_ref());
            }
        }
        HookEvent::SessionEnd | HookEvent::Other(_) => {}
    }
}
