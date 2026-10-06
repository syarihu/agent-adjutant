//! Whether the process a record names is still that process.

use super::store::{hub_record_path, read_record};
use super::*;

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
pub(super) enum Answer {
    Said(String),
    NoSuchProcess,
    CannotTell,
}

pub(super) fn ps_answer(pid: u32, field: &str) -> Answer {
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

/// Where a check for "when did this pid start" gets its answer: one `ps` per pid, or one `ps`
/// for the whole process table, asked the first time it is needed and kept after.
///
/// The table is for a caller that checks many records in one go, such as the board's poll,
/// where one `ps` per worker made the cost grow with the number of worktrees.
///
/// A `ps -A` that cannot be run or exits non-zero reads as "cannot tell" for every pid, on
/// purpose: a failure of the whole table says nothing about one pid. The per-pid call reads a
/// silent non-zero exit as "no such process", which is how `ps -p` answers for a pid that is gone.
pub struct ProcessTable {
    all: bool,
    snapshot: std::cell::OnceCell<Option<HashMap<u32, String>>>,
}

impl ProcessTable {
    /// Asks `ps` about each pid on its own.
    pub fn each() -> Self {
        ProcessTable {
            all: false,
            snapshot: std::cell::OnceCell::new(),
        }
    }

    /// Asks `ps` once, and only when a pid is first looked up.
    pub fn snapshot() -> Self {
        ProcessTable {
            all: true,
            snapshot: std::cell::OnceCell::new(),
        }
    }

    /// A table that answers from `starts` (`None` for a `ps` that could not be read), for tests.
    #[cfg(test)]
    pub(crate) fn fixed(starts: Option<HashMap<u32, String>>) -> Self {
        ProcessTable {
            all: true,
            snapshot: std::cell::OnceCell::from(starts),
        }
    }

    pub(super) fn lstart(&self, pid: u32) -> Answer {
        if !self.all {
            return ps_answer(pid, "lstart");
        }
        match self.snapshot.get_or_init(process_starts) {
            // `ps` could not be run: the answer the single call gives for every pid.
            None => Answer::CannotTell,
            Some(starts) => match starts.get(&pid) {
                Some(started) => Answer::Said(started.clone()),
                None => Answer::NoSuchProcess,
            },
        }
    }

    pub(super) fn started(&self, pid: u32) -> Option<String> {
        match self.lstart(pid) {
            Answer::Said(value) => Some(value).filter(|s| !s.is_empty()),
            _ => None,
        }
    }
}

/// Every process's start time, from one `ps`. `None` when `ps` cannot be run or fails, which
/// the single-pid call reads as "cannot tell" as well.
fn process_starts() -> Option<HashMap<u32, String>> {
    let out = Command::new("ps")
        .args(["-A", "-o", "pid=,lstart="])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| parse_process_starts(&String::from_utf8_lossy(&out.stdout)))
}

/// The pid and start time of each line of `ps -A -o pid=,lstart=`.
///
/// The start time is the rest of the line, trimmed: it holds spaces of its own, and a
/// localized day name can be multi-byte. Trimmed because that is what the single-pid call's
/// answer is, and the two are compared to the same recorded string.
pub(super) fn parse_process_starts(output: &str) -> HashMap<u32, String> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let digits = line.bytes().take_while(u8::is_ascii_digit).count();
            let pid = line[..digits].parse().ok()?;
            Some((pid, line[digits..].trim().to_string()))
        })
        .collect()
}

/// Whether this process is `pid` or runs somewhere below it: how a command run by an agent
/// tells that it was run by that agent, whatever shells sit between them.
pub fn is_self_or_descendant_of(pid: u32) -> bool {
    let mut curr = std::process::id();
    for _ in 0..32 {
        if curr == pid {
            return true;
        }
        match crate::infra::terminal::parent_of(curr) {
            Some(parent) if parent > 1 => curr = parent,
            _ => return false,
        }
    }
    false
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
pub(super) fn process_matches(
    pid: u32,
    expect_in_command: Option<&str>,
    started: Option<&str>,
) -> bool {
    process_matches_with(&ProcessTable::each(), pid, expect_in_command, started)
}

pub(super) fn process_matches_with(
    table: &ProcessTable,
    pid: u32,
    expect_in_command: Option<&str>,
    started: Option<&str>,
) -> bool {
    match started {
        Some(started) => table.started(pid).as_deref() == Some(started),
        None => match expect_in_command {
            Some(name) => ps_field(pid, "command").is_some_and(|c| c.contains(name)),
            None => ps_field(pid, "command").is_some(),
        },
    }
}

/// The three answers `claim_hub` acts on, for a caller that is going to act on "nobody is
/// there" without claiming anything.
///
/// `hub_status` folds "cannot tell" into `present: false`, which is the right reading for a
/// sender — a message left for a hub that may or may not be there waits in a file until
/// somebody reads it, so guessing wrong costs nothing. It is the wrong reading for a caller
/// about to *start* a hub, and `claim_hub` is where that is normally caught. A route that
/// starts a hub somewhere else, and so never reaches a claim, has to ask here instead.
///
/// Deliberately `holder`'s reading of a record that is *there* and not a stricter one. The
/// question being asked is whether the name is free to take, which is the question
/// `claim_hub` will ask about the same record a moment later; a caller that refused where
/// the claim proceeds would be two routes disagreeing about one record, which is the
/// disagreement this is here to close.
///
/// What has to be added to `holder` is the case it never sees. It is called by `claim_hub`
/// only once `create_new` has failed, so the file is known to exist and `read_json`
/// answering `None` can only mean unreadable. Asked cold, that same `None` is mostly the
/// ordinary "nothing has ever registered here" — so existence is established first, the way
/// `read_worker` establishes it, and only then is the record read.
pub fn hub_liveness(root: &Path, slug: &str) -> Liveness {
    let path = hub_record_path(root, slug);
    // `record_exists` rather than `exists`, which answers "no" to every error it meets — and
    // "no" is the answer that goes on to start a hub. A symlink pointing nowhere is
    // something here, the same thing the claim's own `hard_link` meets and refuses on.
    match record_exists(&path) {
        // No record is the same answer as a record whose process has gone: nobody holds
        // the name. It is what `create_new` is about to say by succeeding.
        Ok(false) => Liveness::Gone,
        Err(_) => Liveness::CannotTell,
        Ok(true) => holder(&path),
    }
}

/// The start time a record offers as an anchor, or `None` when what it offers cannot be one.
///
/// Four readers compare this against what `ps` says now, and a value that can match nothing
/// is worse than a missing one: blank, the comparison fails against every live process, and
/// each of them concludes "not the process I recorded". That is `Gone` at a hub's claim,
/// absent at either presence check, and a live worker's worktree offered up for deletion.
///
/// `ps_started` never writes a blank one, so this is about records written or edited by
/// something else. Interpreted here, once, so that an anchor which cannot anchor is the
/// same answer everywhere as no anchor at all — and each reader then falls back to whatever
/// it uses when a record has none, rather than asserting an absence it never established.
pub(super) fn recorded_anchor(record: &HubRecord) -> Option<&str> {
    anchor_of(record.ps_started.as_deref())
}

/// Whether the process a record names is still there — with "cannot tell" kept separate.
///
/// Start time and pid only, never the name in the command line: a record is written by a
/// launcher that has not yet `exec`ed the agent, so for that moment the name is genuinely
/// absent from a perfectly live hub, and mistaking that for a dead one is how a second hub
/// gets started.
pub(super) fn holder(path: &Path) -> Liveness {
    let Recorded::Found(record) = read_record(path, HubRecord::from_value) else {
        // Unreadable rather than absent, or not a JSON object — a record that cannot be
        // read cannot be shown to belong to anybody. Refusing to act on it is the answer
        // that never takes a live hub's name.
        return Liveness::CannotTell;
    };
    let Some(pid) = record.pid else {
        // A record with no pid names nobody. Nothing to be careful of.
        return Liveness::Gone;
    };
    let pid = pid as u32;
    let recorded = recorded_anchor(&record);
    match ps_answer(pid, "lstart") {
        Answer::NoSuchProcess => Liveness::Gone,
        Answer::CannotTell => Liveness::CannotTell,
        Answer::Said(started) => match recorded {
            // Same pid, different start time: the pid was recycled onto something else.
            Some(recorded) if started != recorded => Liveness::Gone,
            // Nothing to compare, so nothing here says this pid was recycled — and this
            // answer is the one that decides whether a name is free to take. The trade is
            // deliberate: an anchorless record over a pid that is alive but unrelated makes
            // the name look taken for as long as that process lives, and `adj hub-stop`
            // clears it. Read the other way, a running hub loses its name to the next
            // launcher and both then answer to the same address, which is the one failure
            // this protocol exists to prevent. It is the same trade `close` makes on the
            // worker side: what cannot be established is left alone, and a person finishes
            // the job.
            _ => Liveness::Alive,
        },
    }
}

/// Whether the process a hub record named is still that process, without going through the
/// record: for a caller that has read the record once and must go on asking about the same
/// hub after another has claimed the name. A record with no anchor names a live pid as
/// alive, as `holder` reads it.
pub fn hub_process_liveness(pid: u32, started: Option<&str>) -> Liveness {
    match ps_answer(pid, "lstart") {
        Answer::NoSuchProcess => Liveness::Gone,
        Answer::CannotTell => Liveness::CannotTell,
        Answer::Said(now) => match started.map(str::trim).filter(|s| !s.is_empty()) {
            Some(recorded) if recorded != now => Liveness::Gone,
            _ => Liveness::Alive,
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

/// How long a row with no pid may be quiet before it is taken for a process that died without
/// `Stop` or `SessionEnd`. A live session sends events far more often than this.
const QUIET_ROW_SECS: i64 = 86_400;

/// Whether an agent session's process is gone, so the sweep removes its row and readers leave
/// it out.
///
/// A row with a pid is dead when the pid is gone or its start time differs, and a table that
/// cannot be read judges nothing. A row with no start time cannot tell a reused pid from its
/// own process, so it is judged by the pid and also dropped once quiet for 24 hours (a live
/// session reads the start time again on every event). So is a row without a pid, whatever its
/// status; one with no time at all has nothing to show it was ever alive.
pub(super) fn agent_session_dead(row: &AgentSession, table: &ProcessTable, now: i64) -> bool {
    match row.pid {
        Some(pid) => match table.lstart(pid) {
            Answer::NoSuchProcess => true,
            Answer::CannotTell => false,
            Answer::Said(started) => match anchor_of(row.ps_started.as_deref()) {
                Some(recorded) => recorded != started,
                None => quiet(row, now),
            },
        },
        None => quiet(row, now),
    }
}

fn quiet(row: &AgentSession, now: i64) -> bool {
    match row.last_event_at.or(row.created_at) {
        Some(seen) => now - seen >= QUIET_ROW_SECS,
        None => true,
    }
}
