use super::*;
use crate::infra::terminal::SessionTerminal;
use serde_json::Map;

// ── the other direction: a worker in a worktree ──────────────────────

/// A worker's record and its messages both live in the worktree, not in the state
/// directory. The hub already knows the worktree path — it created it — so there is no key
/// to derive and no way for the two sides to disagree about one.
pub fn worker_record_path(worktree: &Path) -> PathBuf {
    worktree.join(".claude").join("adjutant-worker.json")
}

/// Where the hub leaves messages for the worker. One markdown file rather than one file per
/// message, because the worker reads it with its eyes as often as with a tool.
pub fn outbox_path(worktree: &Path) -> PathBuf {
    worktree.join(".claude").join("adjutant-outbox.md")
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

pub fn read_worker_record(worktree: &Path) -> Recorded<WorkerRecord> {
    read_record(&worker_record_path(worktree), WorkerRecord::from_value)
}

/// A record that is there and cannot be rewritten, which is not the same news as no record:
/// the worker is registered and its file is damaged.
fn unreadable_worker_record(worktree: &Path) -> String {
    format!(
        "cannot read the worker record at {}: it is not the JSON this tool writes",
        worker_record_path(worktree).display()
    )
}

pub(super) fn write_worker_record(worktree: &Path, record: &WorkerRecord) -> Result<(), String> {
    write_json(&worker_record_path(worktree), &record.to_value())
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
fn recorded_phases(record: &WorkerRecord) -> Vec<Value> {
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
fn append_phase(record: &mut WorkerRecord, phase: &str, at: i64) {
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

/// Write down which step the worker in `worktree` is in, and when it got there.
///
/// Into the worker's own record rather than the task's, because the phase belongs to this
/// run of the worker: a worker started again in the same worktree starts without one, as its
/// record is written fresh. The time is what the board measures "stuck" from.
pub fn set_worker_phase(worktree: &Path, phase: &str) -> Result<(), String> {
    if !PHASES.contains(&phase) {
        return Err(format!(
            "no such phase: {phase} (one of {})",
            PHASES.join(", ")
        ));
    }
    let mut record = match read_worker_record(worktree) {
        Recorded::Found(record) => record,
        Recorded::Unreadable => return Err(unreadable_worker_record(worktree)),
        Recorded::Absent => {
            return Err(format!(
                "no worker is registered in {}: `adj phase` is for the worker running there",
                worktree.display()
            ));
        }
    };
    append_phase(&mut record, phase, now_secs());
    write_worker_record(worktree, &record)
}

/// Join the worker in `worktree` to a task and to the hub that task belongs to.
///
/// Written into the record the worker already has rather than a new one: the process fields
/// are what say it is still the same worker, and only the answers to "which task" and "which
/// hub" change. `hub` is `None` for the repository's own hub, which a record says by having
/// no key. A worker linked to a task is at work on it, so a record with no phase yet gets
/// `implement` — the card needs one to show, and nothing else has said otherwise. A `phase`
/// the caller names is entered whatever the record says: the person linking a session knows
/// where it stands, and an earlier phase must not decide for them.
///
/// The saved session is rewritten too, because `all_repo_hubs` counts a hub's children from
/// both, and one that kept naming the old hub would keep it from closing.
pub fn relink_worker(
    worktree: &Path,
    hub: Option<&str>,
    task: &str,
    phase: Option<&str>,
) -> Result<(), String> {
    let mut record = match read_worker_record(worktree) {
        Recorded::Found(record) => record,
        Recorded::Unreadable => return Err(unreadable_worker_record(worktree)),
        Recorded::Absent => {
            return Err(format!("no worker is registered in {}", worktree.display()));
        }
    };
    record.task = Some(task.to_string());
    record.hub = said(hub);
    if record.hub.is_none() {
        // A hub of another type is in `other`, and the repository's own hub is no key at all.
        record.other.remove("hub");
    }
    match phase {
        Some(phase) => append_phase(&mut record, phase, now_secs()),
        None if record.phase.is_none() => append_phase(&mut record, "implement", now_secs()),
        None => {}
    }
    // The saved session first and the record last: the record is what the board and the
    // worker read, so a failure in the second write must not leave it naming a task the
    // caller then takes back. The session is put back if the record cannot be written.
    let saved_path = worker_session_path(worktree);
    let previous = std::fs::read_to_string(&saved_path).ok();
    if let Some(saved) = worker_session(worktree) {
        save_worker_session(
            worktree,
            saved.title.as_deref().unwrap_or_default(),
            hub,
            Some(task),
            &saved.session_id,
        )?;
    }
    if let Err(e) = write_worker_record(worktree, &record) {
        if let Some(previous) = previous {
            let _ = std::fs::write(&saved_path, previous);
        }
        return Err(e);
    }
    Ok(())
}

/// Record this process as the worker for `worktree`, before `exec`ing the agent over it —
/// the same trick the hub uses, and for the same reason: the PID has to outlive the
/// launcher that wrote it down.
/// `hub` is written down for the worker's benefit rather than for this launcher's: the
/// agent about to be `exec`ed here reports with no address in its hands, so the address has
/// to be somewhere it can be found from the worktree. Omitted entirely when there is none,
/// so a record written by this version and read by any other says the same thing.
pub fn register_worker(
    worktree: &Path,
    title: &str,
    hub: Option<&str>,
    task: Option<&str>,
    terminal: Option<&crate::infra::terminal::SessionTerminal>,
) -> Result<PathBuf, String> {
    // A worker started again in the same worktree for the same task keeps the timeline of the
    // run before it, though not its current phase: that belongs to the run that said it. A
    // worktree reused for another task starts a timeline of its own.
    let carried = match read_worker_record(worktree) {
        Recorded::Found(old) if old.task == said(task) => recorded_phases(&old),
        _ => Vec::new(),
    };
    // Where the worker runs, read at the moment it starts. Settings and live lookups say
    // where a new tab would go or where some pane is now, not where this one was opened.
    let mut other = Map::new();
    if let Some(terminal) = terminal
        && let Ok(value) = serde_json::to_value(terminal)
    {
        other.insert("terminal".to_string(), value);
    }
    let pid = std::process::id();
    let record = WorkerRecord {
        pid: Some(u64::from(pid)),
        title: Some(title.to_string()),
        started_at: Some(utc_stamp(now_secs())),
        ps_started: ps_started(pid),
        hub: said(hub),
        task: said(task),
        phase: None,
        phase_at: None,
        phases: (!carried.is_empty()).then_some(carried),
        other,
    };
    write_worker_record(worktree, &record)?;
    // The slot is held by the record from here on. Failing to drop the marker only keeps it
    // counted until the grace runs out, which the record would have done anyway.
    let _ = unmark_worker_starting(worktree);
    Ok(worker_record_path(worktree))
}

pub fn unregister_worker(worktree: &Path) -> Result<(), String> {
    remove_if_present(&worker_record_path(worktree))
}

/// How long a worker that was dispatched but has not registered yet still holds its slot.
///
/// Registration happens inside the new tab, seconds after `adj work` returns. A hub that
/// dispatches three in one turn would otherwise count none of the first two and go past the
/// limit. A minute covers a slow terminal; past that the tab most likely never started, and
/// a slot held for a worker that does not exist is the one mistake a limit must not make.
pub const STARTING_GRACE_SECS: i64 = 60;

/// Written by `adj work` just before it opens the tab, removed when the worker registers.
pub fn starting_marker_path(worktree: &Path) -> PathBuf {
    worktree
        .join(".claude")
        .join("adjutant-worker-starting.json")
}

pub fn mark_worker_starting(worktree: &Path) -> Result<(), String> {
    write_json(
        &starting_marker_path(worktree),
        &json!({ "at": now_secs() }),
    )
}

pub fn unmark_worker_starting(worktree: &Path) -> Result<(), String> {
    remove_if_present(&starting_marker_path(worktree))
}

/// Whether `worktree` holds a worker slot: a worker that is there, or one dispatched less
/// than `STARTING_GRACE_SECS` ago that has not said so yet. A worker parked at a gate holds
/// its slot like any other — it is still a process on this machine — and a dead one does not.
///
/// "Dead" means `worker_liveness` said `Gone`, not merely that presence could not be shown:
/// when `ps` cannot answer, or the record has no start time to match against, the worker
/// may well be running, and counting it free is how a limit is overshot. Counting it busy
/// only delays a dispatch, and not for long — a pid that is no longer running is `Gone`
/// whatever else the record lacks.
pub fn holds_worker_slot(worktree: &Path, now: i64) -> bool {
    holds_worker_slot_with(&ProcessTable::each(), worktree, now)
}

/// `holds_worker_slot`, asking `table` when the worker's process started.
pub fn holds_worker_slot_with(table: &ProcessTable, worktree: &Path, now: i64) -> bool {
    let registered = match read_worker(worktree) {
        Recorded::Found(worker) => worker_liveness_with(table, &worker) != Liveness::Gone,
        // Neither names a process that could be running.
        Recorded::Absent | Recorded::Unreadable => false,
    };
    registered || is_starting(worktree, now)
}

/// The half of `holds_worker_slot` that is not a `ps` call, for a caller that already has
/// the worker's status in hand.
pub fn is_starting(worktree: &Path, now: i64) -> bool {
    read_json(&starting_marker_path(worktree))
        .and_then(|marker| marker.get("at").and_then(Value::as_i64))
        // Bounded below as well: a clock set back after the dispatch would otherwise hold
        // the slot for as long as it was moved.
        .is_some_and(|at| (0..STARTING_GRACE_SECS).contains(&(now - at)))
}

/// How long a "being removed" marker is believed when the process that wrote it cannot be
/// shown to be running. One that can be is believed for as long as it runs.
const REMOVING_SECS: i64 = 300;

/// Where the marker saying `worktree` is being removed is kept: in the main checkout, outside
/// the worktree that is about to go.
///
/// The worktree is resolved through its parent directory, so the answer is the same while the
/// directory exists and after it is gone; resolving the path itself would give a symlinked
/// `/tmp` or `/var` one spelling before and another after.
fn removing_marker_path(main: &Path, worktree: &Path) -> PathBuf {
    let resolved = match (worktree.parent(), worktree.file_name()) {
        (Some(parent), Some(leaf)) => parent
            .canonicalize()
            .map(|parent| parent.join(leaf))
            .unwrap_or_else(|_| worktree.to_path_buf()),
        _ => worktree.to_path_buf(),
    };
    let name: String = resolved
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    main.join(".claude")
        .join("adjutant-removing")
        .join(format!("{name}.json"))
}

/// The marker saying a worktree is being removed, removed again when this goes out of scope,
/// whichever way that happens.
pub struct RemovingMark(PathBuf);

impl Drop for RemovingMark {
    fn drop(&mut self) {
        let _ = remove_if_present(&self.0);
    }
}

/// Say that `worktree` is being removed, so that nothing starts a worker in it meanwhile.
/// Written under the dispatch lock, with the process and the time.
pub fn mark_worktree_removing(main: &Path, worktree: &Path) -> Result<RemovingMark, String> {
    let path = removing_marker_path(main, worktree);
    let pid = std::process::id();
    write_json(
        &path,
        &json!({ "pid": pid, "psStarted": ps_started(pid), "at": now_secs() }),
    )?;
    Ok(RemovingMark(path))
}

/// Whether a removal of `worktree` is under way: the process that wrote the marker is still
/// running, or the marker is recent enough that it may be.
pub fn is_being_removed(main: &Path, worktree: &Path) -> bool {
    let Some(marker) = read_json(&removing_marker_path(main, worktree)) else {
        return false;
    };
    let alive = marker
        .get("pid")
        .and_then(Value::as_u64)
        .zip(marker.get("psStarted").and_then(Value::as_str))
        .is_some_and(|(pid, started)| process_matches(pid as u32, None, Some(started)));
    let recent = marker
        .get("at")
        .and_then(Value::as_i64)
        .is_some_and(|at| (0..REMOVING_SECS).contains(&(now_secs() - at)));
    alive || recent
}

/// How long a dispatch waits for another to finish counting before it gives up.
const DISPATCH_LOCK_SECS: u64 = 10;

/// Run `f` while holding this checkout's dispatch lock.
///
/// Counting the slots and marking one taken are two steps, and two `adj work` run side by
/// side — a hub batching its shell calls, or two hubs on one repository — would both count
/// before either marked, and both start. The lock makes the pair one step.
///
/// An advisory lock on an open file, for the reason `take_over` gives: a lock made of a
/// file's existence has to guess when its holder died, and two callers guessing at once both
/// get in. This one is released by the system when its holder exits, so a killed dispatch
/// leaves nothing to clear. Waiting ends in an error rather than in taking the lock anyway.
pub fn with_dispatch_lock<T>(main: &Path, f: impl FnOnce() -> T) -> Result<T, String> {
    let path = main.join(".claude").join("adjutant-dispatch.lock");
    // Never unlinked: removing it while another process holds it open would hand the next
    // two callers two different locks.
    let lock = crate::infra::fs::open_lock(&path)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(DISPATCH_LOCK_SECS);
    loop {
        if crate::infra::fs::try_hold(&lock, &path)? {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "another dispatch has held {} for {DISPATCH_LOCK_SECS}s; try again",
                path.display()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let answer = f();
    drop(lock);
    Ok(answer)
}

/// The worktrees among `worktrees` holding a worker slot, `except` left out — the one about
/// to be dispatched into, which is taking a slot rather than competing for one.
pub fn busy_worktrees(worktrees: &[String], except: Option<&Path>) -> Vec<String> {
    let now = now_secs();
    // Compared resolved, because git answers with the real path and a caller may be holding
    // one through a symlink — `/tmp` against `/private/tmp` on a Mac — and a worktree that
    // failed to match itself would count against its own dispatch.
    let resolved = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let except = except.map(resolved);
    worktrees
        .iter()
        .filter(|path| except.as_deref() != Some(resolved(Path::new(path.as_str())).as_path()))
        .filter(|path| holds_worker_slot(Path::new(path.as_str()), now))
        .cloned()
        .collect()
}

pub fn worker_status(worktree: &Path) -> WorkerStatus {
    worker_status_with(&ProcessTable::each(), worktree)
}

/// `worker_status`, asking `table` when the worker's process started.
pub fn worker_status_with(table: &ProcessTable, worktree: &Path) -> WorkerStatus {
    let mut status = WorkerStatus {
        worktree: worktree.to_string_lossy().to_string(),
        present: false,
        pid: None,
        title: None,
        stale: false,
        phase: None,
        phase_at: None,
        phases: Vec::new(),
    };
    let Recorded::Found(record) = read_worker_record(worktree) else {
        return status;
    };
    // Checked, as `read_worker` checks it: truncated, `4294967297` would be pid 1 and read
    // as a worker that is there, while the slot count reads the same record as nobody.
    status.pid = record.pid.and_then(|p| u32::try_from(p).ok());
    status.title = record.title.clone();
    status.phase = record.phase.clone();
    status.phase_at = record.phase_at;
    status.phases = recorded_phases(&record)
        .iter()
        .filter_map(|entry| {
            let pair = entry.as_array()?;
            Some((pair.first()?.as_str()?.to_string(), pair.get(1)?.as_i64()?))
        })
        .collect();
    // A worker's command line carries nothing distinctive — it is whatever agent the config
    // names — so the start time is the only anchor available here, and with none the
    // question narrows to whether that pid is there at all.
    let ps_started = record.anchor();
    match status.pid {
        Some(pid) if process_matches_with(table, pid, None, ps_started) => status.present = true,
        _ => status.stale = true,
    }
    status
}

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

/// What a worktree's record says, for a caller that is going to act destructively on it.
///
/// `Absent` is no record at all: nobody registered here, or it has already been cleared. The
/// hole this leaves is a real one: a worker started by hand rather than through `adj work`
/// never wrote a record, so it reads as free too. Nothing here can see such a process — a
/// caller's own check for uncommitted and unpushed work is the only net under it.
///
/// `Unreadable` is a record that is there and cannot be read as naming a worker —
/// unparseable, or parseable with no usable pid in it. `holder` reads a pid-less record as
/// naming nobody, and that is the right reading of the question *it* asks: whether a hub's
/// name is free to take. It is the wrong reading of "may this worktree be deleted", so the
/// strict one lives here, beside the caller that needs it, and `holder` is left as it is.
pub fn read_worker(worktree: &Path) -> Recorded<WorkerIdentity> {
    match read_worker_record(worktree) {
        Recorded::Absent => Recorded::Absent,
        Recorded::Unreadable => Recorded::Unreadable,
        // A pid out of range as well as absent or the wrong type is unreadable: `as u32` on a
        // number this large silently truncates, and a truncated pid names a live process that
        // nobody asked about.
        Recorded::Found(record) => match record.usable_pid() {
            None => Recorded::Unreadable,
            Some(pid) => Recorded::Found(WorkerIdentity {
                pid,
                // Blank counts as absent, like everywhere else. Here the fallback for a
                // record with no anchor is `CannotTell`, which is what stops a worktree
                // being deleted on the strength of a pid number alone.
                started: record.anchor().map(str::to_string),
                title: record.title.clone(),
            }),
        },
    }
}

/// Is this exact process still there, with "cannot tell" kept apart from "no"?
///
/// The question is asked of the individual, never of the record: a record that has been
/// removed says nothing about whether the process it named is still running, and reading it
/// as death is how a worktree gets deleted under a live worker.
///
/// `worker_status` answers the delivery version of this question and folds "cannot tell"
/// into "nobody there", which is the safe reading when being wrong costs a message that
/// waits in a file until somebody reads it. Here it is the unsafe one.
pub fn worker_liveness(worker: &WorkerIdentity) -> Liveness {
    worker_liveness_with(&ProcessTable::each(), worker)
}

/// `worker_liveness`, asking `table` when the worker's process started.
pub fn worker_liveness_with(table: &ProcessTable, worker: &WorkerIdentity) -> Liveness {
    match table.lstart(worker.pid) {
        Answer::NoSuchProcess => Liveness::Gone,
        Answer::CannotTell => Liveness::CannotTell,
        Answer::Said(started) => match &worker.started {
            Some(recorded) if &started == recorded => Liveness::Alive,
            // Same pid, another start time: the pid has been handed to something else, and
            // the worker that recorded it is gone.
            Some(_) => Liveness::Gone,
            // Something is running under that pid and nothing says it is this worker. The
            // pid alone would answer `Alive` for whatever inherited the number, and this
            // answer is what closes a tab and clears a worktree — so it is the same
            // reading every other unanswerable question here gets. `holder` says `Alive`
            // to the same record on purpose: the question there is whether a hub's name is
            // free to take, and the cost of its two mistakes runs the other way.
            None => Liveness::CannotTell,
        },
    }
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

/// Clear a worktree's record, but only while it still names `worker`.
///
/// Anything but `Yes` means the record was left alone, and the caller has something to say
/// about a worktree it was about to call free. The three answers are kept apart because two
/// of them are different sentences to a person: "somebody else is working here" and "this
/// file cannot be read" are not the same news.
///
/// **`Yes` is not evidence that `worker` is dead.** This removes a note about a process; it
/// never looks at the process. Establishing the death is `worker_liveness`'s job and the
/// caller's responsibility, and doing it in the other order — clear the record, then read
/// the record to see whether the worker is gone — is the mistake this pair is shaped to
/// prevent. The order is not enforced by the types: these are `pub` inside a private
/// module, reachable only from this crate's own commands, and a token type bought here
/// would be a ceremony with no outside caller to protect.
///
/// Read-then-remove, so a registration landing in the gap between the two still loses its
/// record. The gap is left open on purpose: closing it means a lock on a path the hub's own
/// claim protocol writes, which is the one piece of concurrency here that has been argued
/// over and settled. What the gap costs is a record, which the next worker's launcher
/// rewrites; what a lock would cost is that settlement.
pub fn unregister_worker_if(worktree: &Path, worker: &WorkerIdentity) -> Result<Cleared, String> {
    match read_worker(worktree) {
        Recorded::Found(named) if &named == worker => {
            unregister_worker(worktree)?;
            Ok(Cleared::Yes)
        }
        // Already gone: whoever removed it wanted what this call wanted.
        Recorded::Absent => Ok(Cleared::Yes),
        Recorded::Found(_) => Ok(Cleared::AnotherWorker),
        Recorded::Unreadable => Ok(Cleared::Unreadable),
    }
}

/// Append one entry to the worker's outbox.
///
/// The shape is fixed here rather than described in the hub's procedure: a format spelled
/// out in prose is a format that drifts, and the worker reads the `##` heading to know who
/// is speaking and the first line to know whether it is being asked something.
pub fn tell(worktree: &Path, from: &str, subject: &str, body: &str) -> Result<PathBuf, String> {
    let path = outbox_path(worktree);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let entry = format!(
        "## {} from {}\n\n{}\n{}\n\n",
        utc_stamp(now_secs()),
        one_line(from),
        one_line(subject),
        body.trim_end()
    );
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.write_all(entry.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

pub fn read_outbox(worktree: &Path) -> String {
    std::fs::read_to_string(outbox_path(worktree)).unwrap_or_default()
}

/// Everything in the outbox has been dealt with. Removed rather than emptied so that
/// "is there anything for me" is answered by the file existing at all.
pub fn clear_outbox(worktree: &Path) -> Result<(), String> {
    remove_if_present(&outbox_path(worktree))
}
