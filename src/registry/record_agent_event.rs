//! What an agent's hook says about its session, written into the session's row.

use super::store::{
    agent_session_broken_path, agent_session_lock_path, agent_session_path, agent_sessions_dir,
    read_agent_session, write_agent_session,
};
use super::*;
use crate::infra::fs::{lock_checked, try_lock_checked};
use std::time::{Duration, SystemTime};

/// One hook call, as the receiver parsed it. The agent's own payload shape stays in the
/// transport; this is what the ledger needs from it.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentEvent {
    /// `claude` first.
    pub agent: String,
    pub session_id: String,
    pub hook: HookEvent,
    /// Set when a sub-agent sent the event, or when it is a `SubagentStart` / `SubagentStop`.
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub cwd: Option<String>,
    /// The tool in use, as `Edit: src/lib.rs`.
    pub summary: Option<String>,
    /// The agent's process (`CLAUDE_PID`).
    pub pid: Option<u32>,
    pub config_dir: Option<String>,
    /// When it was received, epoch seconds: the only clock `apply` reads.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
    PostToolUse,
    PostToolUseFailure,
    PermissionRequest,
    Notification {
        kind: Option<String>,
        message: Option<String>,
    },
    Stop,
    StopFailure,
    SessionEnd,
    SubagentStart,
    SubagentStop,
    /// One draw of the status line, not a hook: the figures it showed, each absent when the draw
    /// did not carry it. It only fills a row that exists and sets no status, though like any
    /// event it still runs the sub-agent tidy, which can apply a held `pendingStatus`.
    StatusLine {
        model: Option<String>,
        context_percent: Option<f64>,
        five_hour: Option<RateWindow>,
        seven_day: Option<RateWindow>,
    },
    /// An event this version does not know. It records nothing.
    Other(String),
}

/// How many times the row is read again under its lock for a lookup it turned out to need.
const ROUNDS: usize = 3;

/// A `.json.broken` this old is not going to be looked at.
const BROKEN_KEPT: Duration = Duration::from_secs(7 * 86_400);

/// Apply `event` to its session's row under the row's lock, and sweep rows whose process has
/// gone when the event is one that comes seldom enough to pay for it.
pub fn record_agent_event(root: &Path, event: &AgentEvent) -> Result<(), String> {
    record_agent_event_with(root, event, &ProcessTable::snapshot())
}

/// `record_agent_event`, asking `table` when a process started (rule 7).
pub fn record_agent_event_with(
    root: &Path,
    event: &AgentEvent,
    table: &ProcessTable,
) -> Result<(), String> {
    // Before anything touches the disk: the id is a file name.
    if !valid_session_id(&event.session_id) {
        return Err(format!("not a usable session id: {:?}", event.session_id));
    }
    let id = event.session_id.as_str();
    let path = agent_session_path(root, id);

    // Read without the lock, only to see what the event costs: git and `ps` are run before the
    // lock is taken, and only when the row does not already hold their answer.
    let first = match read_agent_session(&path) {
        Recorded::Found(row) => Some(row),
        Recorded::Absent | Recorded::Unreadable => None,
    };
    // Nothing to record, so nothing to look up: git and `ps` are for events that write.
    if matches!(event.hook, HookEvent::Other(_) | HookEvent::SessionEnd)
        || (first.is_none() && !can_create(event))
    {
        return remove_or_leave(root, event, first.is_some());
    }
    let mut lookups = Lookups::default();
    look_up(&wanted(first.as_ref(), event), &mut lookups, table);

    for round in 1..=ROUNDS {
        let guard = lock_checked(&agent_session_lock_path(root, id))?;
        let current = match read_agent_session(&path) {
            Recorded::Found(row) => Some(row),
            Recorded::Absent => None,
            Recorded::Unreadable => {
                // Moved aside, as proctor does with its ledger, so a new row can be started.
                std::fs::rename(&path, agent_session_broken_path(root, id))
                    .map_err(|e| format!("cannot move {} aside: {e}", path.display()))?;
                None
            }
        };
        let missing = wanted(current.as_ref(), event).unanswered(&lookups);
        if round == ROUNDS {
            // Another event has moved the row under it each time: answer with nothing rather
            // than look again.
            answer_with_nothing(&missing, &mut lookups);
        }
        match apply(current, event, &lookups) {
            Applied::NeedsLookup => {
                drop(guard);
                look_up(&missing, &mut lookups, table);
                continue;
            }
            Applied::Write(row) => write_agent_session(root, &row)?,
            Applied::Remove => remove_if_present(&path)?,
            Applied::Unchanged => {}
        }
        drop(guard);
        return match event.hook {
            HookEvent::SessionStart | HookEvent::Stop => sweep(root, table, event.at, id),
            _ => Ok(()),
        };
    }
    Err(format!("cannot settle the row for session {id}"))
}

/// `SessionEnd` removes the row under its lock and needs no lookup; the rest end here.
fn remove_or_leave(root: &Path, event: &AgentEvent, row_exists: bool) -> Result<(), String> {
    if !(matches!(event.hook, HookEvent::SessionEnd) && row_exists) {
        return Ok(());
    }
    let _guard = lock_checked(&agent_session_lock_path(root, &event.session_id))?;
    remove_if_present(&agent_session_path(root, &event.session_id))
}

fn look_up(wanted: &Wanted, lookups: &mut Lookups, table: &ProcessTable) {
    if let Some(cwd) = &wanted.cwd {
        let worktree = current_worktree(Some(Path::new(cwd)));
        lookups.worktree = Some((cwd.clone(), worktree));
    }
    if let Some(pid) = wanted.pid {
        lookups.started = Some((pid, table.started(pid)));
    }
}

fn answer_with_nothing(wanted: &Wanted, lookups: &mut Lookups) {
    if let Some(cwd) = &wanted.cwd {
        lookups.worktree = Some((cwd.clone(), None));
    }
    if let Some(pid) = wanted.pid {
        lookups.started = Some((pid, None));
    }
}

/// Remove the rows of sessions whose process has gone, the locks nobody has a row for, and
/// `.json.broken` files a week old. `except` is the session whose event this is.
///
/// Another session's row that cannot be read is left alone: it is not shown to be dead. Every
/// failure is collected rather than stopping the sweep, and goes back after the rest is done.
fn sweep(root: &Path, table: &ProcessTable, now: i64, except: &str) -> Result<(), String> {
    let dir = agent_sessions_dir(root);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("cannot list {}: {e}", dir.display())),
    };
    let mut errors = Vec::new();
    let mut locks = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                errors.push(format!("cannot list {}: {e}", dir.display()));
                continue;
            }
        };
        let path = entry.path();
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if name.ends_with(".json.broken") {
            let old = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .is_some_and(|age| age > BROKEN_KEPT);
            if old && let Err(e) = remove_if_present(&path) {
                errors.push(e);
            }
        } else if let Some(id) = name.strip_suffix(".json") {
            if id != except
                && valid_session_id(id)
                && let Err(e) = sweep_row(root, table, now, id)
            {
                errors.push(e);
            }
        } else if let Some(id) = name.strip_suffix(".lock") {
            locks.push(id.to_string());
        }
    }
    // After the rows, so the lock of a row swept just now is an orphan by the time it is looked at.
    for id in locks {
        if let Err(e) = sweep_lock(root, &id) {
            errors.push(e);
        }
    }
    match errors.is_empty() {
        true => Ok(()),
        false => Err(errors.join("; ")),
    }
}

/// Remove row `id` if it is dead, judged again under its lock. A lock held by somebody else
/// means a writer is at it, and the row is left for the next sweep.
fn sweep_row(root: &Path, table: &ProcessTable, now: i64, id: &str) -> Result<(), String> {
    let path = agent_session_path(root, id);
    let dead = |path: &Path| match read_agent_session(path) {
        Recorded::Found(row) => agent_session_dead(&row, table, now),
        Recorded::Absent | Recorded::Unreadable => false,
    };
    if !dead(&path) {
        return Ok(());
    }
    let Some(_held) = try_lock_checked(&agent_session_lock_path(root, id))? else {
        return Ok(());
    };
    match dead(&path) {
        true => remove_if_present(&path),
        false => Ok(()),
    }
}

/// Remove the lock of a row that is gone, if nobody holds it. Unlinking a lock somebody holds
/// open would hand two callers two locks (see `open_lock`), which is why this takes it first and
/// looks for the row again, and why no writer ever removes its own. A writer that had opened the
/// file and not yet locked it is not seen by `try_lock`; it finds the path replaced when it
/// does lock, and opens it again (`lock_checked`). The sweep's own lock is checked the same way,
/// so that two sweeps cannot unlink each other's replacement. Where inodes cannot be compared
/// (not unix) no lock file is removed.
fn sweep_lock(root: &Path, id: &str) -> Result<(), String> {
    if cfg!(not(unix)) || !valid_session_id(id) || agent_session_path(root, id).exists() {
        return Ok(());
    }
    let lock_path = agent_session_lock_path(root, id);
    let Some(_held) = try_lock_checked(&lock_path)? else {
        return Ok(());
    };
    match agent_session_path(root, id).exists() {
        true => Ok(()),
        false => remove_if_present(&lock_path),
    }
}

#[cfg(test)]
mod tests;
