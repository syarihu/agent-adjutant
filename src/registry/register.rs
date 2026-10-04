//! Recording this process as a worktree's worker.

use super::store::{worker_record_path, write_worker_record};
use super::*;
use serde_json::Map;

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
