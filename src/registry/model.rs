//! The records and answers `registry` deals in, and the pure helpers over them.

use super::*;
use crate::infra::terminal::SessionTerminal;
use serde_json::Map;

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
