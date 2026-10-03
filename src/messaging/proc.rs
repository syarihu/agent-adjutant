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
        match parent_of(curr) {
            Some(parent) if parent > 1 => curr = parent,
            _ => return false,
        }
    }
    false
}

fn parent_of(pid: u32) -> Option<u32> {
    ps_field(pid, "ppid")?.parse().ok()
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
