//! Reading the records: who is there, and what a worker's record says.

use super::store::{hub_record_path, read_record, worker_record_path};
use super::*;

pub fn read_hub_record(slug: &str) -> Recorded<HubRecord> {
    read_record(&hub_record_path(slug), HubRecord::from_value)
}

/// Every readable `hubs/*.json`, by slug (the file stem). A missing directory is no records.
pub fn hub_records() -> Vec<(String, HubRecord)> {
    let Ok(entries) = std::fs::read_dir(state_dir().join("hubs")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                return None;
            }
            let slug = path.file_stem()?.to_str().filter(|s| !s.is_empty())?;
            match read_record(&path, HubRecord::from_value) {
                Recorded::Found(record) => Some((slug.to_string(), record)),
                _ => None,
            }
        })
        .collect()
}

pub fn hub_status(slug: &str, hub_name: &str) -> HubStatus {
    hub_status_with(&ProcessTable::each(), slug, hub_name)
}

/// `hub_status`, asking `table` when the hub's process started.
pub fn hub_status_with(table: &ProcessTable, slug: &str, hub_name: &str) -> HubStatus {
    let mut status = HubStatus {
        slug: slug.to_string(),
        hub_name: hub_name.to_string(),
        present: false,
        pid: None,
        cwd: None,
        started_at: None,
        stale: false,
    };
    let Recorded::Found(record) = read_hub_record(slug) else {
        return status;
    };
    status.pid = record.pid.map(|p| p as u32);
    status.cwd = record.cwd.clone();
    status.started_at = record.started_at.clone();
    let ps_started = recorded_anchor(&record);
    // Looking for the hub's name in its command line is the stronger of the two anchors,
    // but it only works if the name is *there* — a `hubRunner` with no `{name}` in it, or
    // one that `exec`s something that keeps none of its arguments, produces a live hub that
    // fails this check forever. So the launcher records whether the name it was about to
    // run actually carried it, and a session that could never match is matched on its
    // start time alone, exactly as a worker is. A record from before this was written has
    // no answer, and the old behaviour is the safe reading of that: it was started by a
    // template that did carry the name, or it would not have been found at all.
    let named = record.name_in_command.unwrap_or(true);
    let recorded_name = record.hub_name.as_deref().unwrap_or(hub_name);
    let expect = named.then_some(recorded_name);
    match status.pid {
        Some(pid) if process_matches_with(table, pid, expect, ps_started) => status.present = true,
        _ => status.stale = true,
    }
    status
}

pub fn read_worker_record(worktree: &Path) -> Recorded<WorkerRecord> {
    read_record(&worker_record_path(worktree), WorkerRecord::from_value)
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

/// The hub a checkout's worker reports to: the key its record says, and failing that the one
/// its saved session says. The record wins because `adj worker --hub` rewrites it on a
/// resume, while the session is only written when a session starts.
pub fn worker_hub_key(worktree: &Path) -> Option<String> {
    let clean = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    match read_worker_record(worktree) {
        Recorded::Found(record) => record.hub.as_deref().and_then(clean),
        _ => None,
    }
    .or_else(|| worker_session(worktree).and_then(|saved| saved.hub.as_deref().and_then(clean)))
}

/// The task a checkout's worker is on: what its record says, and only when it has no record
/// what its saved session says. A record without a task is a session that has none, and a
/// session saved before it was linked must not give it back one it has since been moved off.
pub fn worker_task(worktree: &Path) -> Option<String> {
    let clean = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    match read_worker_record(worktree) {
        Recorded::Found(record) => record.task.as_deref().and_then(clean),
        Recorded::Absent | Recorded::Unreadable => {
            worker_session(worktree).and_then(|saved| saved.task.as_deref().and_then(clean))
        }
    }
}
