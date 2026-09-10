//! Addressing and delivery between a hub and its workers, over the filesystem.
//!
//! The thing being replaced here is an agent-specific pair of primitives: "list the sessions
//! this harness knows about" and "send a message to one of them". Those exist in exactly one
//! coding agent, which is why the hub only ever worked in that one.
//!
//! What both sides actually need is smaller than a session list: an address they can both
//! derive (the hub name), a way to ask whether anyone is home, and a place to leave a note.
//! A PID file answers the second and a directory answers the third, so any agent that can
//! run a command can take part.
//!
//! Delivery never fails for want of a listener. A message written while the hub is down sits
//! in the same directory the running hub reads from, and is picked up when it next starts —
//! so `present: false` is information for the sender, not an error.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{expand_home, home_dir};

/// `ADJUTANT_STATE_DIR` wins, then `$XDG_STATE_HOME/adjutant`, then `~/.local/state/adjutant`.
pub fn state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ADJUTANT_STATE_DIR")
        && !dir.is_empty()
    {
        return expand_home(&dir);
    }
    match std::env::var("XDG_STATE_HOME") {
        Ok(dir) if !dir.is_empty() => expand_home(&dir).join("adjutant"),
        _ => home_dir().join(".local").join("state").join("adjutant"),
    }
}

pub fn hub_record_path(slug: &str) -> PathBuf {
    state_dir().join("hubs").join(format!("{slug}.json"))
}

/// Where messages for this hub wait. One directory, whether or not the hub is running: two
/// would mean the hub has to remember to read both, and the one it forgets is the one that
/// silently swallows reports.
pub fn inbox_dir(slug: &str) -> PathBuf {
    state_dir().join("inbox").join(slug)
}

/// Where a message goes once the hub has acted on it. Kept rather than deleted so a report
/// that was mishandled can still be found.
pub fn archive_dir(slug: &str) -> PathBuf {
    inbox_dir(slug).join("read")
}

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

/// Record this process as the hub for `slug`, unless another live hub already is.
///
/// Called by `adj hub`, which then `exec`s the agent — so the PID stays valid across
/// the handover and the record points at the live agent process rather than at a launcher
/// that has already exited.
///
/// "One hub per repository" is the whole point of the record, and asking `hub_status`
/// first and writing second cannot enforce it: both askers hear "nobody home" and both
/// write. The record is therefore *claimed* — created with `create_new`, which the
/// filesystem refuses for the loser — and only a claim that was refused goes on to ask
/// whose it is.
///
/// What the loser asks is deliberately not `hub_status`. A hub that has just won the claim
/// has not `exec`ed the agent yet, so for a moment its command line is still the launcher's
/// and the name is not in it — `hub_status` calls that stale, and clearing a "stale" record
/// and retrying is the same check-then-act this is here to remove, arrived at from the
/// other side. The only question that can be asked safely is the one that cannot be
/// mid-change: is the recorded process still the process that was recorded? Alive with the
/// same start time means someone holds the name, whether or not they look like a hub yet.
/// A few retries, not a thousand: each one is two calls to `ps`, and a record that keeps
/// coming back means someone else keeps winning it.
pub fn claim_hub(
    slug: &str,
    hub_name: &str,
    cwd: &str,
    name_in_command: bool,
) -> Result<Claim, String> {
    let path = hub_record_path(slug);
    let record = json!({
        "pid": std::process::id(),
        "hubName": hub_name,
        "cwd": cwd,
        "startedAt": utc_stamp(now_secs()),
        "psStarted": ps_started(std::process::id()),
        "nameInCommand": name_in_command,
    });
    match create_new_json(&path, &record) {
        Ok(()) => return Ok(Claim::Ours),
        Err(CreateError::Taken) => {}
        Err(CreateError::Failed(message)) => return Err(message),
    }
    match holder(&path) {
        Liveness::Alive => return Ok(Claim::Taken(Box::new(hub_status(slug, hub_name)))),
        Liveness::CannotTell => return Err(cannot_tell(&path)),
        Liveness::Gone => {}
    }

    // The record belongs to a process that has gone, and taking it over means deleting a
    // file and creating it again — two steps, which two launchers can interleave: the
    // second delete removes the *first one's live record* and the name is handed out
    // twice. Neither `create_new` nor `rename` prevents that, because the second launcher
    // is acting on a name whose contents changed underneath it, and POSIX has no "remove
    // this file only if it is still the one I looked at".
    //
    // So the takeover — and only the takeover — is serialised. The lock is a file nobody
    // can create twice, it names who holds it, and it is held for the few syscalls between
    // "this record is dead" and "this record is mine". A lock left behind by a crash goes
    // stale on a clock, which is safe here in a way it would never be for the hub record
    // itself: this one is held for microseconds, so an old one is evidence, not a guess.
    take_over(&path, &record, slug, hub_name)
}

fn take_over(path: &Path, record: &Value, slug: &str, hub_name: &str) -> Result<Claim, String> {
    // An advisory lock held on an open file, not a file whose existence is the lock.
    //
    // A lock made of a file has to answer "what if its holder died holding it", and every
    // answer to that is a guess — a clock, a pid, another liveness check — and every guess
    // is check-then-act again, one level down: two launchers both decide a lock is stale,
    // both break it, and both are inside. This one is released by the operating system when
    // the process ends, however it ends, so there is no stale case to reason about. The
    // file itself is never removed: unlinking it while another process holds it open would
    // hand the next two callers two different locks.
    let lock_path = path.with_extension("claiming");
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("cannot open {}: {e}", lock_path.display()))?;
    match lock.try_lock() {
        Ok(()) => {}
        // Someone else is part-way through taking this name. Whatever they end up with, it
        // is not ours — the same answer we would have been given by arriving after they
        // finished.
        Err(std::fs::TryLockError::WouldBlock) => {
            return Ok(Claim::Taken(Box::new(hub_status(slug, hub_name))));
        }
        Err(std::fs::TryLockError::Error(e)) => {
            return Err(format!("cannot lock {}: {e}", lock_path.display()));
        }
    }

    // Asked again inside the lock: the record may have been taken over while we were
    // getting in, and the answer from outside is the one that was about to go stale.
    let claimed = match holder(path) {
        Liveness::Alive => Ok(Claim::Taken(Box::new(hub_status(slug, hub_name)))),
        Liveness::CannotTell => Err(cannot_tell(path)),
        Liveness::Gone => {
            remove_if_present(path)?;
            match create_new_json(path, record) {
                Ok(()) => Ok(Claim::Ours),
                Err(CreateError::Taken) => Ok(Claim::Taken(Box::new(hub_status(slug, hub_name)))),
                Err(CreateError::Failed(message)) => Err(message),
            }
        }
    };
    drop(lock);
    claimed
}

fn cannot_tell(path: &Path) -> String {
    format!(
        "cannot tell whether the hub recorded in {} is still running, so its name is left alone",
        path.display()
    )
}

/// Whether the process a record names is still there — with "cannot tell" kept separate.
///
/// Start time and pid only, never the name in the command line: a record is written by a
/// launcher that has not yet `exec`ed the agent, so for that moment the name is genuinely
/// absent from a perfectly live hub, and mistaking that for a dead one is how a second hub
/// gets started.
fn holder(path: &Path) -> Liveness {
    let Some(record) = read_json(path) else {
        // Unreadable rather than absent — `read_json` cannot say which, and a record that
        // cannot be read cannot be shown to belong to anybody. Refusing to act on it is
        // the answer that never takes a live hub's name.
        return Liveness::CannotTell;
    };
    let Some(pid) = record.get("pid").and_then(Value::as_u64) else {
        // A record with no pid names nobody. Nothing to be careful of.
        return Liveness::Gone;
    };
    let pid = pid as u32;
    let recorded = record.get("psStarted").and_then(Value::as_str);
    match ps_answer(pid, "lstart") {
        Answer::NoSuchProcess => Liveness::Gone,
        Answer::CannotTell => Liveness::CannotTell,
        Answer::Said(started) => match recorded {
            // Same pid, different start time: the pid was recycled onto something else.
            Some(recorded) if started != recorded => Liveness::Gone,
            _ => Liveness::Alive,
        },
    }
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

pub fn unregister_hub(slug: &str) -> Result<(), String> {
    remove_if_present(&hub_record_path(slug))
}

/// Is the process in the record still alive, and still the one that was recorded?
///
/// PIDs get recycled, and a recycled one that happens to be alive would make an absent
/// session look present — the one failure mode that loses a message. Two independent
/// anchors rule that out: the process's start time (which `exec` preserves, so it survives
/// the handover from launcher to agent) and, where there is one, a distinctive string in
/// the command line. `ps` failing at all is read as "not present": guessing yes costs a
/// message, guessing no costs a file that gets picked up later.
/// What `ps` said, and whether it managed to say anything at all.
///
/// "No such process" and "`ps` could not be run" are the same `None` to a presence check
/// — both mean "do not assume anybody is there" — but they are not the same to a *claim*:
/// treating "cannot tell" as "dead" is how one hub takes a live hub's name away.
enum Answer {
    Said(String),
    NoSuchProcess,
    CannotTell,
}

fn ps_answer(pid: u32, field: &str) -> Answer {
    let Ok(out) = Command::new("ps")
        .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
        .output()
    else {
        return Answer::CannotTell;
    };
    if !out.status.success() {
        // `ps` exits non-zero both for "no such process" and for "I could not do that",
        // and only the first is an answer. The one that is an answer says nothing at all;
        // the other explains itself on stderr.
        return match String::from_utf8_lossy(&out.stderr).trim().is_empty() {
            true => Answer::NoSuchProcess,
            false => Answer::CannotTell,
        };
    }
    Answer::Said(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn ps_field(pid: u32, field: &str) -> Option<String> {
    match ps_answer(pid, field) {
        Answer::Said(value) => Some(value),
        _ => None,
    }
}

/// When the process started, as the system reports it. Compared as an opaque string.
pub fn ps_started(pid: u32) -> Option<String> {
    ps_field(pid, "lstart").filter(|s| !s.is_empty())
}

/// Is `pid` still the process that was recorded?
///
/// The recorded start time is the anchor, and where there is one it is the *whole* check.
/// `exec` preserves it, so it survives the handover from launcher to agent, and a recycled
/// pid — the one failure mode that loses a message — cannot match it.
///
/// The name in the command line is only consulted when there is no start time to compare,
/// which is the case where something has to stand in for it. It used to be checked as well,
/// always, and that was a mistake in both directions: a template that puts the name
/// somewhere `exec` discards (`env NAME={name} agent …`) produced a live hub that read as
/// absent forever, and the check bought nothing the start time had not already ruled out.
fn process_matches(pid: u32, expect_in_command: Option<&str>, started: Option<&str>) -> bool {
    match started {
        Some(started) => ps_started(pid).as_deref() == Some(started),
        None => match expect_in_command {
            Some(name) => ps_field(pid, "command").is_some_and(|c| c.contains(name)),
            None => ps_field(pid, "command").is_some(),
        },
    }
}

pub fn hub_status(slug: &str, hub_name: &str) -> HubStatus {
    let mut status = HubStatus {
        slug: slug.to_string(),
        hub_name: hub_name.to_string(),
        present: false,
        pid: None,
        cwd: None,
        started_at: None,
        stale: false,
    };
    let Some(record) = read_json(&hub_record_path(slug)) else {
        return status;
    };
    status.pid = record.get("pid").and_then(Value::as_u64).map(|p| p as u32);
    status.cwd = record
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string);
    status.started_at = record
        .get("startedAt")
        .and_then(Value::as_str)
        .map(str::to_string);
    let ps_started = record.get("psStarted").and_then(Value::as_str);
    // Looking for the hub's name in its command line is the stronger of the two anchors,
    // but it only works if the name is *there* — a `hubRunner` with no `{name}` in it, or
    // one that `exec`s something that keeps none of its arguments, produces a live hub that
    // fails this check forever. So the launcher records whether the name it was about to
    // run actually carried it, and a session that could never match is matched on its
    // start time alone, exactly as a worker is. A record from before this was written has
    // no answer, and the old behaviour is the safe reading of that: it was started by a
    // template that did carry the name, or it would not have been found at all.
    let named = record
        .get("nameInCommand")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let recorded_name = record
        .get("hubName")
        .and_then(Value::as_str)
        .unwrap_or(hub_name);
    let expect = named.then_some(recorded_name);
    match status.pid {
        Some(pid) if process_matches(pid, expect, ps_started) => status.present = true,
        _ => status.stale = true,
    }
    status
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerStatus {
    pub worktree: String,
    pub present: bool,
    pub pid: Option<u32>,
    pub title: Option<String>,
    pub stale: bool,
}

/// Record this process as the worker for `worktree`, before `exec`ing the agent over it —
/// the same trick the hub uses, and for the same reason: the PID has to outlive the
/// launcher that wrote it down.
pub fn register_worker(worktree: &Path, title: &str) -> Result<PathBuf, String> {
    let path = worker_record_path(worktree);
    write_json(
        &path,
        &json!({
            "pid": std::process::id(),
            "title": title,
            "startedAt": utc_stamp(now_secs()),
            "psStarted": ps_started(std::process::id()),
        }),
    )?;
    Ok(path)
}

pub fn unregister_worker(worktree: &Path) -> Result<(), String> {
    remove_if_present(&worker_record_path(worktree))
}

pub fn worker_status(worktree: &Path) -> WorkerStatus {
    let mut status = WorkerStatus {
        worktree: worktree.to_string_lossy().to_string(),
        present: false,
        pid: None,
        title: None,
        stale: false,
    };
    let Some(record) = read_json(&worker_record_path(worktree)) else {
        return status;
    };
    status.pid = record.get("pid").and_then(Value::as_u64).map(|p| p as u32);
    status.title = record
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string);
    // A worker's command line carries nothing distinctive — it is whatever agent the config
    // names — so the start time is the only anchor available here.
    let ps_started = record.get("psStarted").and_then(Value::as_str);
    match status.pid {
        Some(pid) if process_matches(pid, None, ps_started) => status.present = true,
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
    /// tells this worker from whatever the system later hands that pid to. `None` in a
    /// record written on a machine where `ps` would not answer at registration time.
    pub started: Option<String>,
    pub title: Option<String>,
}

/// What a worktree's record says, for a caller that is going to act destructively on it.
pub enum WorkerRecord {
    /// No record at all. Nobody registered here, or it has already been cleared.
    ///
    /// The hole this leaves, and it is a real one: a worker started by hand rather than
    /// through `adj work` never wrote a record, so it reads as free too. Nothing here can
    /// see such a process — a caller's own check for uncommitted and unpushed work is the
    /// only net under it.
    Absent,
    Named(WorkerIdentity),
    /// A record is there and cannot be read as naming a worker — unparseable, or parseable
    /// with no usable pid in it.
    ///
    /// `holder` reads a pid-less record as naming nobody, and that is the right reading of
    /// the question *it* asks: whether a hub's name is free to take. It is the wrong
    /// reading of "may this worktree be deleted", so the strict one lives here, beside the
    /// caller that needs it, and `holder` is left as it is.
    Unreadable,
}

/// Read a worktree's worker record once.
pub fn read_worker(worktree: &Path) -> WorkerRecord {
    let path = worker_record_path(worktree);
    // `try_exists` rather than `exists`, which answers "no" to every error it meets. It is
    // still not perfect — a symlink pointing nowhere is `Ok(false)` here — and the
    // difference has no way of arising for a file this tool writes itself.
    match path.try_exists() {
        Ok(false) => return WorkerRecord::Absent,
        Err(_) => return WorkerRecord::Unreadable,
        Ok(true) => {}
    }
    let Some(record) = read_json(&path) else {
        return WorkerRecord::Unreadable;
    };
    // Out of range as well as absent or the wrong type: `as u32` on a number this large
    // silently truncates, and a truncated pid names a live process that nobody asked about.
    let Some(pid) = record
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid > 0)
    else {
        return WorkerRecord::Unreadable;
    };
    WorkerRecord::Named(WorkerIdentity {
        pid,
        started: record
            .get("psStarted")
            .and_then(Value::as_str)
            .map(str::to_string),
        title: record
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
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
    match ps_answer(worker.pid, "lstart") {
        Answer::NoSuchProcess => Liveness::Gone,
        Answer::CannotTell => Liveness::CannotTell,
        Answer::Said(started) => match &worker.started {
            // Same pid, another start time: the pid has been handed to something else, and
            // the worker that recorded it is gone.
            Some(recorded) if &started != recorded => Liveness::Gone,
            _ => Liveness::Alive,
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
        WorkerRecord::Named(named) if &named == worker => {
            unregister_worker(worktree)?;
            Ok(Cleared::Yes)
        }
        // Already gone: whoever removed it wanted what this call wanted.
        WorkerRecord::Absent => Ok(Cleared::Yes),
        WorkerRecord::Named(_) => Ok(Cleared::AnotherWorker),
        WorkerRecord::Unreadable => Ok(Cleared::Unreadable),
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

fn remove_if_present(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
    }
}

pub fn status_json(status: &HubStatus) -> Value {
    json!({
        "hubName": status.hub_name,
        "slug": status.slug,
        "present": status.present,
        "stale": status.stale,
        "pid": status.pid,
        "cwd": status.cwd,
        "startedAt": status.started_at,
        "inbox": inbox_dir(&status.slug).to_string_lossy(),
    })
}

// ── messages ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Message {
    /// Who is speaking. A worker's session name, or whatever the sending agent calls itself.
    pub from: String,
    /// `report`, `question`, `answer`, `ack`, `done`, or anything the two sides agree on.
    pub kind: String,
    /// The one line a human will actually read.
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Delivery {
    pub path: PathBuf,
    pub present: bool,
}

/// Leave `message` for the hub. Returns where it landed and whether anyone was there to see
/// it arrive.
pub fn send(slug: &str, hub_name: &str, message: &Message) -> Result<Delivery, String> {
    let status = hub_status(slug, hub_name);
    let dir = inbox_dir(slug);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let kind = if message.kind.is_empty() {
        "report"
    } else {
        &message.kind
    };
    let stamp = utc_stamp(now_secs());
    let staged = stage(&dir, &render_message(message))?;
    let claimed = claim_link(&staged, &dir, |seq| match seq {
        0 => format!("{stamp}-{kind}.md"),
        seq => format!("{stamp}-{seq}-{kind}.md"),
    });
    // The staging name has served its purpose either way. Leaving it behind would be
    // invisible to `list`, which is worse than a stray file: an inbox that quietly grows.
    let _ = std::fs::remove_file(&staged);
    Ok(Delivery {
        path: claimed?,
        present: status.present,
    })
}

pub fn render_message(message: &Message) -> String {
    let subject = if message.subject.is_empty() {
        message
            .body
            .lines()
            .next()
            .unwrap_or("(no subject)")
            .to_string()
    } else {
        message.subject.clone()
    };
    format!(
        "---\nfrom: {}\nkind: {}\nsubject: {}\nat: {}\n---\n\n{}\n",
        one_line(&message.from),
        one_line(if message.kind.is_empty() {
            "report"
        } else {
            &message.kind
        }),
        one_line(&subject),
        utc_stamp(now_secs()),
        message.body.trim_end()
    )
}

/// Newlines in a header value would let a body forge extra headers, and a colon in a value
/// is harmless but confusing. Collapsing whitespace handles both.
fn one_line(text: &str) -> String {
    let collapsed: Vec<&str> = text.split_whitespace().collect();
    collapsed.join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub subject: String,
    pub from: String,
    pub kind: String,
}

pub fn list(slug: &str) -> Vec<Entry> {
    let dir = inbox_dir(slug);
    // Before answering, put back anything an ack was interrupted half way through. This is
    // the one place that reads the whole directory, so it is the one place that can see it.
    put_back_abandoned(&dir);
    let Ok(read) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut names: Vec<String> = read
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let text = std::fs::read_to_string(dir.join(&name)).unwrap_or_default();
            let header = |key: &str| header_value(&text, key).unwrap_or_default();
            Entry {
                subject: header("subject"),
                from: header("from"),
                kind: header("kind"),
                name,
            }
        })
        .collect()
}

pub fn header_value(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return None;
    }
    for line in lines {
        if line == "---" {
            return None;
        }
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

pub fn read(slug: &str, name: &str) -> Result<String, String> {
    let path = safe_join(&inbox_dir(slug), name)?;
    std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Move one message out of the way. The hub calls this once it has filed the report.
///
/// The archive is where a mishandled report is found again, so an archived name is never
/// reused: two messages acked in the same second are two files, not one file and one loss.
///
/// The message is *taken* before it is filed, not after. Filing first and unlinking second
/// is the order that loses one: two acks of a name each file a copy, the first unlink frees
/// the name, a send in the same second claims it, and the second unlink deletes that new
/// message — which nothing filed. So the inbox name is claimed in one step, by renaming it
/// onto a name only this call knows. A second acker's rename finds nothing and says so.
///
/// Between the two steps the message is real but hidden, which is a state this has to be
/// able to come back from: the name it was taken from is written into the holding name, and
/// `list` puts back anything it finds there that is too old to be in flight.
pub fn ack(slug: &str, name: &str) -> Result<PathBuf, String> {
    let inbox = inbox_dir(slug);
    let from = safe_join(&inbox, name)?;
    let dir = archive_dir(slug);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let held = hold(&inbox, name)?;
    if let Err(e) = std::fs::rename(&from, &held) {
        let _ = std::fs::remove_file(&held);
        return match e.kind() {
            std::io::ErrorKind::NotFound => Err(format!("no message called {name} is waiting")),
            _ => Err(format!("cannot move {}: {e}", from.display())),
        };
    }
    // From here the message exists only under the holding name, so it is unlinked from
    // there only once something else holds it. An ack that cannot file *and* cannot put
    // back leaves it where the sweep below will find it, rather than deleting it to keep
    // the directory tidy.
    match claim_link(&held, &dir, |seq| numbered(name, seq)) {
        Ok(to) => {
            let _ = std::fs::remove_file(&held);
            Ok(to)
        }
        Err(e) => match claim_link(&held, &inbox, |seq| numbered(name, seq)) {
            Ok(back) => {
                let _ = std::fs::remove_file(&held);
                Err(format!(
                    "{e} — the message is back in the inbox as {}",
                    back.file_name().unwrap_or_default().to_string_lossy()
                ))
            }
            Err(_) => Err(format!(
                "{e} — the message is held at {} and will be put back",
                held.display()
            )),
        },
    }
}

/// The prefix a message wears while it is being acked. A dotfile, so `list` does not offer
/// it and `safe_join` will not open it — it is mid-move, not waiting — and it carries the
/// name it came from so that a move interrupted half way can be undone.
const HOLDING: &str = ".acking-";

/// How long a message may be held before `list` decides nobody is coming back for it. An
/// ack holds one across two syscalls, so anything this old is from a process that died.
const HELD_STALE_SECS: u64 = 60;

fn hold(inbox: &Path, name: &str) -> Result<PathBuf, String> {
    for attempt in 0..CLAIM_ATTEMPTS {
        let path = inbox.join(format!("{HOLDING}{}-{attempt}-{name}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write {}: {e}", path.display())),
        }
    }
    Err(format!(
        "cannot hold a message in {} after {CLAIM_ATTEMPTS} tries",
        inbox.display()
    ))
}

/// Put back anything an interrupted ack left holding.
///
/// Without this, a process that died between taking a message and filing it left the only
/// copy under a name nothing lists, nothing reads and nothing acks — a report that exists
/// and cannot be reached, which is worse than the overwrite this whole arrangement replaced.
/// Age is what distinguishes an abandoned hold from one in flight, and the margin is wide:
/// an ack holds a message for two syscalls.
fn put_back_abandoned(inbox: &Path) {
    let Ok(read) = std::fs::read_dir(inbox) else {
        return;
    };
    for entry in read.flatten() {
        let held = entry.path();
        let Some(rest) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.strip_prefix(HOLDING).map(str::to_string))
        else {
            continue;
        };
        // `<pid>-<attempt>-<the name it came from>`.
        let Some(name) = rest.splitn(3, '-').nth(2).map(str::to_string) else {
            continue;
        };
        let too_old = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|at| at.elapsed().map(|d| d.as_secs()).unwrap_or(0) > HELD_STALE_SECS)
            .unwrap_or(false);
        if !too_old {
            continue;
        }
        if claim_link(&held, inbox, |seq| numbered(&name, seq)).is_ok() {
            let _ = std::fs::remove_file(&held);
        }
    }
}

/// A message name comes from an agent, so it is untrusted input used as a path. Anything
/// with a separator in it is rejected outright rather than sanitised — a name that needed
/// sanitising was not one of ours.
fn safe_join(dir: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return Err(format!("invalid message name: {name}"));
    }
    Ok(dir.join(name))
}

/// How many names are tried before a claim gives up. Same-second sends are normal (a
/// worker filing two findings at once), so the counter is not an edge case to skip; a
/// thousand of them in one second is not a collision but a runaway.
const CLAIM_ATTEMPTS: usize = 1000;

/// Give `staged` a second name in `dir`, the first one `name_for` offers that is free.
///
/// Two properties have to hold at once, and one primitive gives both. `hard_link` refuses
/// an existing name instead of replacing it, so two senders racing for the same second
/// cannot both win — which `exists()` followed by a write cannot promise, because the
/// answer is already stale by the time it is acted on. And because the name being claimed
/// points at a file that is *already written in full*, nobody can read half a message.
///
/// `rename` would give the second property and lose the first: it replaces silently, which
/// is exactly the overwrite being ruled out here. So the staged file gets linked, not moved,
/// and the caller unlinks the staging name afterwards.
fn claim_link(
    staged: &Path,
    dir: &Path,
    name_for: impl Fn(usize) -> String,
) -> Result<PathBuf, String> {
    for seq in 0..CLAIM_ATTEMPTS {
        let path = dir.join(name_for(seq));
        match std::fs::hard_link(staged, &path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write {}: {e}", path.display())),
        }
    }
    Err(format!(
        "cannot find an unused name in {} after {CLAIM_ATTEMPTS} tries",
        dir.display()
    ))
}

/// Write `text` into `dir` under a name `list` will not return, so the file can be linked
/// into place complete. Staging inside the destination directory rather than in a temporary
/// one is what keeps the link possible: `hard_link` cannot cross a filesystem.
fn stage(dir: &Path, text: &str) -> Result<PathBuf, String> {
    use std::io::Write;
    for attempt in 0..CLAIM_ATTEMPTS {
        // Concurrent stagers are threads as well as processes, so the pid alone is not
        // unique; `create_new` plus a counter settles both.
        let path = dir.join(format!(".staging-{}-{attempt}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                // A write that fails leaves a file behind — invisible to `list`, so it grows
                // unnoticed, and after a thousand of them nothing can be staged at all.
                // `sync_all` before the caller publishes it: the point of writing here and
                // linking there is that the name never points at an incomplete file, and
                // without this that holds for a crash but not for a power cut.
                let written = file
                    .write_all(text.as_bytes())
                    .and_then(|()| file.sync_all());
                return match written {
                    Ok(()) => Ok(path),
                    Err(e) => {
                        let _ = std::fs::remove_file(&path);
                        Err(format!("cannot write {}: {e}", path.display()))
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write {}: {e}", path.display())),
        }
    }
    Err(format!(
        "cannot stage a file in {} after {CLAIM_ATTEMPTS} tries",
        dir.display()
    ))
}

/// `report.md` with `seq` worked into it: `report-2.md`. Suffixing the stem rather than the
/// whole name keeps the extension where a reader (and an editor) expects it.
fn numbered(name: &str, seq: usize) -> String {
    if seq == 0 {
        return name.to_string();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}-{seq}.{ext}"),
        _ => format!("{name}-{seq}"),
    }
}

// ── plumbing ─────────────────────────────────────────────────────────

fn render_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string()) + "\n"
}

fn parent_dir(path: &Path) -> Result<&Path, String> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    Ok(parent)
}

/// Replace whatever is at `path` with `value`, in one step.
///
/// A record is read by a process other than the one writing it, and a reader that catches
/// a half-written file reads no record at all — which for a presence check means a live
/// session reported as absent. Writing beside the record and renaming over it means the
/// name never points at a partial file.
fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let parent = parent_dir(path)?;
    let staged = stage(parent, &render_json(value))?;
    std::fs::rename(&staged, path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("cannot write {}: {e}", path.display())
    })
}

enum CreateError {
    /// The name already exists. Not a failure — an answer.
    Taken,
    Failed(String),
}

/// Write `value` at `path` only if nothing is there, and say which of the two happened.
fn create_new_json(path: &Path, value: &Value) -> Result<(), CreateError> {
    let parent = parent_dir(path).map_err(CreateError::Failed)?;
    let staged = stage(parent, &render_json(value)).map_err(CreateError::Failed)?;
    let result = match std::fs::hard_link(&staged, path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(CreateError::Taken),
        Err(e) => Err(CreateError::Failed(format!(
            "cannot write {}: {e}",
            path.display()
        ))),
    };
    let _ = std::fs::remove_file(&staged);
    result
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `YYYYMMDDTHHMMSSZ`. UTC, and said so in the name: these strings sort, appear in filenames
/// and get copied into issues, and a local time with no offset in it is the kind of thing
/// that is wrong for half the year without anyone noticing.
pub fn utc_stamp(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86_400);
    let secs = epoch_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Howard Hinnant's days-from-civil, inverted. Shifting the era to start in March makes the
/// leap day the last day of the year, which is what removes the month-length special cases.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testing::Sandbox;

    /// The name this process answers to in `ps`. A record has to carry it for the presence
    /// guard to accept the test binary as the session it names.
    fn this_process_name() -> String {
        std::env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string()
    }

    /// Winners, losers, and launches that reached no answer at all.
    fn tally(outcomes: &[Result<Claim, String>]) -> (usize, usize, usize) {
        let count =
            |f: fn(&Result<Claim, String>) -> bool| outcomes.iter().filter(|o| f(o)).count();
        (
            count(|o| matches!(o, Ok(Claim::Ours))),
            count(|o| matches!(o, Ok(Claim::Taken(_)))),
            count(|o| o.is_err()),
        )
    }

    fn claim_ours(slug: &str, hub_name: &str) {
        match claim_hub(slug, hub_name, "/src/widget", true).unwrap() {
            Claim::Ours => {}
            Claim::Taken(status) => panic!("expected to win the claim, but {status:?} holds it"),
        }
    }

    fn a_message(body: &str) -> Message {
        Message {
            from: "w".into(),
            kind: "report".into(),
            subject: "s".into(),
            body: body.into(),
        }
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let Ok(read) = std::fs::read_dir(dir) else {
            return vec![];
        };
        let mut names: Vec<String> = read
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn stamp_is_utc_and_sorts() {
        assert_eq!(utc_stamp(0), "19700101T000000Z");
        assert_eq!(utc_stamp(1_767_225_600), "20260101T000000Z");
        assert_eq!(utc_stamp(1_757_306_100), "20250908T043500Z");
        assert!(utc_stamp(1) < utc_stamp(2));
    }

    #[test]
    fn stamp_handles_leap_days() {
        // 2024-02-29T12:00:00Z — a year that is a leap year despite the /100 rule biting in
        // the neighbouring centuries.
        assert_eq!(utc_stamp(1_709_208_000), "20240229T120000Z");
        // 2000-02-29T00:00:00Z — the /400 exception.
        assert_eq!(utc_stamp(951_782_400), "20000229T000000Z");
    }

    #[test]
    fn an_absent_hub_still_takes_delivery() {
        let _sandbox = Sandbox::empty();
        let delivery = send(
            "acme-widget",
            "adjutant-acme-widget",
            &Message {
                from: "wid-957-34".into(),
                kind: "report".into(),
                subject: "検索結果の画像が縦に潰れる".into(),
                body: "## Symptom\nthe image is squashed".into(),
            },
        )
        .unwrap();
        assert!(!delivery.present);
        assert!(delivery.path.exists());
        let entries = list("acme-widget");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].subject, "検索結果の画像が縦に潰れる");
        assert_eq!(entries[0].from, "wid-957-34");
    }

    #[test]
    fn two_sends_in_the_same_second_do_not_overwrite_each_other() {
        let _sandbox = Sandbox::empty();
        for _ in 0..3 {
            send(
                "acme-widget",
                "adjutant-acme-widget",
                &Message {
                    from: "w".into(),
                    kind: "report".into(),
                    subject: "s".into(),
                    body: "b".into(),
                },
            )
            .unwrap();
        }
        assert_eq!(list("acme-widget").len(), 3);
    }

    #[test]
    fn a_live_registration_reads_as_present() {
        let _sandbox = Sandbox::empty();
        // The current process is alive by definition; its command line is the test binary,
        // so that is the name the record has to carry for the guard to pass.
        let name = std::env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        claim_ours("acme-widget", &name);
        let status = hub_status("acme-widget", &name);
        assert!(status.present, "{status:?}");
        assert!(!status.stale);
        assert_eq!(status.pid, Some(std::process::id()));
    }

    #[test]
    fn a_recycled_pid_is_not_the_hub() {
        let _sandbox = Sandbox::empty();
        // Alive, and the pid is right — but it started at a different moment, which is what
        // a recycled pid looks like and the one case that would otherwise report "present"
        // and drop the report on the floor.
        write_json(
            &hub_record_path("acme-widget"),
            &json!({
                "pid": std::process::id(),
                "hubName": this_process_name(),
                "cwd": "/src/widget",
                "psStarted": "Thu Jan  1 00:00:00 1970",
            }),
        )
        .unwrap();
        let status = hub_status("acme-widget", &this_process_name());
        assert!(!status.present);
        assert!(status.stale);
    }

    #[test]
    fn no_record_at_all_is_absent_but_not_stale() {
        let _sandbox = Sandbox::empty();
        let status = hub_status("acme-widget", "adjutant-acme-widget");
        assert!(!status.present);
        assert!(!status.stale);
        assert_eq!(status.pid, None);
    }

    /// What `close` is allowed to conclude about a worktree, and every reading that used to
    /// let it conclude "free" without evidence.
    #[test]
    fn a_worker_record_is_only_read_as_nobody_there_when_it_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path();
        assert!(matches!(read_worker(worktree), WorkerRecord::Absent));

        let record = worker_record_path(worktree);
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        for content in [
            // Not JSON at all.
            "{ this is not json".to_string(),
            // Parseable, and naming nobody. `holder` calls this `Gone` for the hub's
            // question; for this one it is a fail-open.
            "{}".to_string(),
            json!({"pid": null}).to_string(),
            json!({"pid": "1234"}).to_string(),
            json!({"pid": 0}).to_string(),
            // `as u32` would truncate this to 1 and go looking at init.
            json!({"pid": 4294967297u64}).to_string(),
        ] {
            std::fs::write(&record, &content).unwrap();
            assert!(
                matches!(read_worker(worktree), WorkerRecord::Unreadable),
                "{content} was read as an answer"
            );
        }

        register_worker(worktree, "WID-957").unwrap();
        let WorkerRecord::Named(worker) = read_worker(worktree) else {
            panic!("a record this process just wrote does not name it");
        };
        assert_eq!(worker.pid, std::process::id());
        assert_eq!(worker.title.as_deref(), Some("WID-957"));
        assert_eq!(worker_liveness(&worker), Liveness::Alive);

        // The record going away says nothing about the process. This is the one that
        // matters: it is what a `close` command that removed the record rather than the tab
        // looks like, and reading it as death deletes a live worker's worktree.
        unregister_worker(worktree).unwrap();
        assert_eq!(worker_liveness(&worker), Liveness::Alive);

        // A pid that is alive but started at another moment is a recycled pid, which is
        // gone in the only sense that matters.
        let recycled = WorkerIdentity {
            started: Some("Thu Jan  1 00:00:00 1970".to_string()),
            ..worker.clone()
        };
        assert_eq!(worker_liveness(&recycled), Liveness::Gone);
    }

    #[test]
    fn a_record_is_only_cleared_while_it_still_names_the_worker_it_was_read_from() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path();
        register_worker(worktree, "WID-957").unwrap();
        let WorkerRecord::Named(worker) = read_worker(worktree) else {
            panic!("a record this process just wrote does not name it");
        };

        // The window this closes: a worker whose tab was closed is observed gone, and a new
        // worker registers in the same worktree before the record is cleared. Clearing it
        // then reports a free worktree, and the next caller finds no record and agrees.
        register_worker_as(worktree, 4321, "WID-958");
        assert!(matches!(
            unregister_worker_if(worktree, &worker).unwrap(),
            Cleared::AnotherWorker
        ));
        assert!(
            worker_record_path(worktree).exists(),
            "another worker's record was cleared"
        );

        // Its own record it may clear, and a record already gone is the outcome it wanted.
        register_worker(worktree, "WID-957").unwrap();
        let WorkerRecord::Named(worker) = read_worker(worktree) else {
            panic!("a record this process just wrote does not name it");
        };
        assert!(matches!(
            unregister_worker_if(worktree, &worker).unwrap(),
            Cleared::Yes
        ));
        assert!(!worker_record_path(worktree).exists());
        // A record already gone is the end state this was asking for.
        assert!(matches!(
            unregister_worker_if(worktree, &worker).unwrap(),
            Cleared::Yes
        ));

        // A record that cannot be read as anybody's is not anybody's to delete either, and
        // says so in its own words rather than borrowing the newcomer's.
        std::fs::write(worker_record_path(worktree), "{}").unwrap();
        assert!(matches!(
            unregister_worker_if(worktree, &worker).unwrap(),
            Cleared::Unreadable
        ));
        assert!(worker_record_path(worktree).exists());
    }

    /// A record for a worker that is not this process, which `register_worker` cannot write.
    fn register_worker_as(worktree: &Path, pid: u32, title: &str) {
        write_json(
            &worker_record_path(worktree),
            &json!({"pid": pid, "title": title, "psStarted": "Thu Jan  1 00:00:00 1970"}),
        )
        .unwrap();
    }

    #[test]
    fn unregister_is_idempotent() {
        let _sandbox = Sandbox::empty();
        unregister_hub("acme-widget").unwrap();
        claim_ours("acme-widget", "adjutant-acme-widget");
        unregister_hub("acme-widget").unwrap();
        unregister_hub("acme-widget").unwrap();
        assert!(!hub_record_path("acme-widget").exists());
    }

    #[test]
    fn ack_moves_the_message_out_of_the_way() {
        let _sandbox = Sandbox::empty();
        send(
            "acme-widget",
            "adjutant-acme-widget",
            &Message {
                from: "w".into(),
                kind: "report".into(),
                subject: "s".into(),
                body: "b".into(),
            },
        )
        .unwrap();
        let name = list("acme-widget")[0].name.clone();
        let moved = ack("acme-widget", &name).unwrap();
        assert!(moved.exists());
        assert!(list("acme-widget").is_empty());
    }

    #[test]
    fn a_message_name_cannot_walk_out_of_the_inbox() {
        let _sandbox = Sandbox::empty();
        assert!(read("acme-widget", "../../etc/passwd").is_err());
        assert!(ack("acme-widget", "..").is_err());
        assert!(read("acme-widget", "").is_err());
    }

    #[test]
    fn a_body_cannot_forge_headers() {
        let rendered = render_message(&Message {
            from: "w\n---\nkind: forged".into(),
            kind: "report".into(),
            subject: "one\ntwo".into(),
            body: "b".into(),
        });
        assert_eq!(
            header_value(&rendered, "from").unwrap(),
            "w --- kind: forged"
        );
        assert_eq!(header_value(&rendered, "subject").unwrap(), "one two");
        assert_eq!(header_value(&rendered, "kind").unwrap(), "report");
    }

    #[test]
    fn an_empty_subject_falls_back_to_the_first_body_line() {
        let rendered = render_message(&Message {
            from: "w".into(),
            kind: String::new(),
            subject: String::new(),
            body: "検索結果が潰れる\n\n詳細".into(),
        });
        assert_eq!(
            header_value(&rendered, "subject").unwrap(),
            "検索結果が潰れる"
        );
        assert_eq!(header_value(&rendered, "kind").unwrap(), "report");
    }

    /// The finding this comes from: forty concurrent `adj send` calls left thirty-six files,
    /// and the four that vanished had each been told they succeeded. Forty is the number
    /// that reproduced it, so forty is the number that guards it.
    #[test]
    fn forty_reports_sent_at_once_are_forty_reports() {
        let _sandbox = Sandbox::empty();
        let bodies: Vec<String> = (0..40).map(|i| format!("finding number {i}")).collect();
        std::thread::scope(|scope| {
            for body in &bodies {
                scope.spawn(move || {
                    send("acme-widget", "adjutant-acme-widget", &a_message(body)).unwrap();
                });
            }
        });

        let entries = list("acme-widget");
        assert_eq!(entries.len(), 40, "reports were lost: {entries:?}");

        // Every one of them whole, and every one of them still its own report: a delivery
        // that overwrites is as bad as one that drops, and counting alone would miss it.
        let mut seen: Vec<String> = entries
            .iter()
            .map(|e| {
                let text = read("acme-widget", &e.name).unwrap();
                assert!(
                    text.starts_with("---\nfrom: w\n"),
                    "half a message: {text:?}"
                );
                text.lines().last().unwrap().to_string()
            })
            .collect();
        seen.sort();
        let mut expected = bodies.clone();
        expected.sort();
        assert_eq!(seen, expected);

        // Nothing staged is left behind — invisible to `list`, so it would grow unnoticed.
        let staged: Vec<String> = names_in(&inbox_dir("acme-widget"))
            .into_iter()
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(staged.is_empty(), "staging files left over: {staged:?}");
    }

    /// Five launches at once used to produce three hubs, and a repository with three hubs
    /// makes it luck which one a report reaches.
    #[test]
    fn only_one_of_five_launches_becomes_the_hub() {
        let _sandbox = Sandbox::empty();
        let name = this_process_name();
        let outcomes: Vec<Result<Claim, String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..5)
                .map(|_| scope.spawn(|| claim_hub("acme-widget", &name, "/src/widget", true)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        // Every launch has to reach an *answer*. Counting only the winners would pass a
        // run where one won and the other four failed outright.
        assert_eq!(tally(&outcomes), (1, 4, 0), "{outcomes:?}");
        assert!(hub_status("acme-widget", &name).present);
    }

    /// A record whose process is gone is not a hub, and must not block the next one.
    #[test]
    fn a_dead_hubs_record_does_not_hold_the_name() {
        let _sandbox = Sandbox::empty();
        // A pid that really has exited, rather than this process wearing a wrong name —
        // "alive but not recognised" is a different thing entirely, and the test below is
        // the one that means it.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        write_json(
            &hub_record_path("acme-widget"),
            &json!({
                "pid": dead,
                "hubName": "adjutant-acme-widget",
                "cwd": "/src/widget",
                "psStarted": "whenever it was",
                "nameInCommand": true,
            }),
        )
        .unwrap();

        let name = this_process_name();
        claim_ours("acme-widget", &name);
        assert!(hub_status("acme-widget", &name).present);
    }

    /// A hub whose name never reaches its own command line is still a hub.
    ///
    /// The window between winning the claim and `exec`ing the agent is one way to be in
    /// that state; a `hubRunner` like `env NAME={name} agent …`, where `exec` keeps none of
    /// the wrapper's arguments, is a permanent one. Presence rests on the recorded start
    /// time, which `exec` preserves and a recycled pid cannot match, so neither case is
    /// mistaken for a session that has gone.
    #[test]
    fn a_hub_whose_name_is_not_in_its_command_line_is_still_present() {
        let _sandbox = Sandbox::empty();
        write_json(
            &hub_record_path("acme-widget"),
            &json!({
                "pid": std::process::id(),
                // Nothing on this machine answers to this name.
                "hubName": "adjutant-acme-widget",
                "cwd": "/src/widget",
                "psStarted": ps_started(std::process::id()),
                "nameInCommand": true,
            }),
        )
        .unwrap();
        let status = hub_status("acme-widget", "adjutant-acme-widget");
        assert!(status.present, "{status:?}");
        assert!(!status.stale);
        // And a launch arriving now is told someone holds it, rather than taking it.
        assert!(matches!(
            claim_hub("acme-widget", "adjutant-acme-widget", "/src/widget", true),
            Ok(Claim::Taken(_))
        ));
    }

    /// Taking over a dead hub's record is itself a race, and it used to be lost the same
    /// way the original was: both launchers read the dead record, the first deletes it and
    /// claims, and the second deletes *that* — a live record — and claims too.
    #[test]
    fn five_launches_inheriting_one_dead_record_still_leave_one_hub() {
        let _sandbox = Sandbox::empty();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        write_json(
            &hub_record_path("acme-widget"),
            &json!({
                "pid": dead,
                "hubName": "adjutant-acme-widget",
                "cwd": "/src/widget",
                "psStarted": "whenever it was",
            }),
        )
        .unwrap();

        let name = this_process_name();
        let outcomes: Vec<Result<Claim, String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..5)
                .map(|_| scope.spawn(|| claim_hub("acme-widget", &name, "/src/widget", true)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let (ours, taken, refused) = tally(&outcomes);
        // The property that matters is that two launches never both become the hub. A
        // launch may also legitimately reach no answer at all — `ps` is a process too, and
        // under load it can fail to start, which is deliberately read as "leave the name
        // alone" rather than as "the holder is gone".
        assert!(ours <= 1, "{outcomes:?}");
        assert_eq!(ours + taken + refused, 5, "{outcomes:?}");
        if refused == 0 {
            assert_eq!(ours, 1, "{outcomes:?}");
        }
    }

    /// Acking never overwrites the archive: the point of keeping a message is that a report
    /// that was mishandled can still be found, and an overwrite is exactly losing one.
    #[test]
    fn acking_the_same_name_twice_keeps_both() {
        let _sandbox = Sandbox::empty();
        let dir = inbox_dir("acme-widget");
        std::fs::create_dir_all(&dir).unwrap();
        // The same name twice on purpose — a send can reuse a name the moment the previous
        // message with it has been acked, and both then land on one archived name.
        for body in ["the first report", "the second report"] {
            std::fs::write(dir.join("20260908T112233Z-report.md"), body).unwrap();
            ack("acme-widget", "20260908T112233Z-report.md").unwrap();
        }
        let archived = names_in(&archive_dir("acme-widget"));
        assert_eq!(
            archived,
            vec![
                "20260908T112233Z-report-1.md".to_string(),
                "20260908T112233Z-report.md".to_string()
            ]
        );
        let bodies: Vec<String> = archived
            .iter()
            .map(|n| std::fs::read_to_string(archive_dir("acme-widget").join(n)).unwrap())
            .collect();
        assert!(bodies.contains(&"the first report".to_string()));
        assert!(bodies.contains(&"the second report".to_string()));
    }

    #[test]
    fn a_numbered_name_keeps_its_extension() {
        assert_eq!(numbered("report.md", 0), "report.md");
        assert_eq!(numbered("report.md", 2), "report-2.md");
        assert_eq!(numbered("a.b.md", 2), "a.b-2.md");
        assert_eq!(numbered("noext", 2), "noext-2");
    }

    /// Only one ack can succeed, because only one of them takes the message.
    ///
    /// Filing first and unlinking second let every acker "succeed": each filed a copy, and
    /// each then unlinked the inbox name — including the one that by then belonged to a
    /// message somebody had just sent, which nothing had filed.
    #[test]
    fn one_message_acked_five_times_at_once_is_acked_once() {
        let _sandbox = Sandbox::empty();
        let dir = inbox_dir("acme-widget");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("20260908T112233Z-report.md"), "the only report").unwrap();

        let outcomes: Vec<Result<PathBuf, String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..5)
                .map(|_| scope.spawn(|| ack("acme-widget", "20260908T112233Z-report.md")))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(
            outcomes.iter().filter(|r| r.is_ok()).count(),
            1,
            "{outcomes:?}"
        );
        for outcome in outcomes.iter().filter(|r| r.is_err()) {
            let message = outcome.as_ref().unwrap_err();
            assert!(message.contains("is waiting"), "{message}");
        }
        // Filed exactly once, and the inbox is empty rather than holding a copy.
        assert_eq!(names_in(&archive_dir("acme-widget")).len(), 1);
        assert!(list("acme-widget").is_empty());
        // Nothing staged left behind on any of the five paths.
        let staged: Vec<String> = names_in(&dir)
            .into_iter()
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(staged.is_empty(), "{staged:?}");
    }

    /// A reader looking into the inbox while senders are writing never sees half a message.
    ///
    /// The forty-send test reads only once every sender has finished, so it cannot see a
    /// partial file even if one were briefly published. This one reads *during*, which is
    /// the only way to look at the window the staging-then-linking exists to close.
    #[test]
    fn a_reader_during_delivery_never_sees_half_a_message() {
        let _sandbox = Sandbox::empty();
        let body = "x".repeat(64 * 1024);
        let done = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            let senders: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        for _ in 0..25 {
                            send("acme-widget", "adjutant-acme-widget", &a_message(&body)).unwrap();
                        }
                    })
                })
                .collect();
            let reader = scope.spawn(|| {
                let mut looked_at = 0usize;
                // At least one pass *after* the senders stop, so a reader the scheduler ran
                // late still reads the two hundred files they left. Otherwise a correct
                // implementation fails whenever this thread happens to start last.
                loop {
                    let finished = done.load(std::sync::atomic::Ordering::Relaxed);
                    for entry in list("acme-widget") {
                        // A name `list` returned can only be gone if something removed it,
                        // and nothing here does — so anything readable must be whole.
                        if let Ok(text) = read("acme-widget", &entry.name) {
                            assert!(text.starts_with("---\nfrom: w\n"), "half a header");
                            assert!(
                                text.trim_end().ends_with(&body),
                                "half a body: {} bytes",
                                text.len()
                            );
                            looked_at += 1;
                        }
                    }
                    if finished {
                        return looked_at;
                    }
                }
            });
            for sender in senders {
                sender.join().unwrap();
            }
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            assert!(reader.join().unwrap() >= 200);
        });
        assert_eq!(list("acme-widget").len(), 200);
    }

    /// An ack that stops half way must leave the message reachable.
    ///
    /// Taking it before filing it is what stops a concurrent ack deleting somebody else's
    /// new message — but between the two steps the only copy wears a name `list` does not
    /// offer and `read` will not open, so a process that dies there used to leave a report
    /// that exists and cannot be reached.
    #[test]
    fn a_message_left_held_by_an_interrupted_ack_comes_back() {
        let _sandbox = Sandbox::empty();
        let dir = inbox_dir("acme-widget");
        std::fs::create_dir_all(&dir).unwrap();
        // Exactly what an ack leaves behind when it dies after the rename: the content, a
        // holding name, and the name it came from written into it.
        let held = dir.join(format!("{HOLDING}424242-0-20260908T112233Z-report.md"));
        std::fs::write(&held, "the report nobody filed").unwrap();
        // Old enough that no ack could still be in flight.
        let long_ago =
            std::time::SystemTime::now() - std::time::Duration::from_secs(HELD_STALE_SECS * 2);
        std::fs::File::options()
            .write(true)
            .open(&held)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();

        let waiting = list("acme-widget");
        assert_eq!(waiting.len(), 1, "{waiting:?}");
        assert_eq!(waiting[0].name, "20260908T112233Z-report.md");
        assert_eq!(
            read("acme-widget", &waiting[0].name).unwrap(),
            "the report nobody filed"
        );
        assert!(!held.exists());
    }

    /// A hold that could still be in flight is left alone: putting it back while its ack is
    /// between the two steps would file it *and* leave a copy waiting.
    #[test]
    fn a_message_still_being_acked_is_not_put_back_underneath_it() {
        let _sandbox = Sandbox::empty();
        let dir = inbox_dir("acme-widget");
        std::fs::create_dir_all(&dir).unwrap();
        let held = dir.join(format!("{HOLDING}424242-0-20260908T112233Z-report.md"));
        std::fs::write(&held, "mid-flight").unwrap();

        assert!(list("acme-widget").is_empty());
        assert!(held.exists());
    }

    /// The takeover lock is held by the operating system, so a second claimer is turned
    /// away rather than left to decide for itself whether the first one is still alive.
    #[test]
    fn a_takeover_in_progress_turns_the_next_claim_away() {
        let _sandbox = Sandbox::empty();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        let record = hub_record_path("acme-widget");
        write_json(
            &record,
            &json!({"pid": dead, "hubName": "adjutant-acme-widget", "psStarted": "long ago"}),
        )
        .unwrap();

        // Someone else is inside the takeover.
        let lock_path = record.with_extension("claiming");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .unwrap();
        lock.lock().unwrap();

        let name = this_process_name();
        assert!(matches!(
            claim_hub("acme-widget", &name, "/src/widget", true),
            Ok(Claim::Taken(_))
        ));

        // And once they are done, the next claim gets it.
        drop(lock);
        claim_ours("acme-widget", &name);
    }
}
