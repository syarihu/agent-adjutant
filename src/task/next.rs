//! The queue pick: which task a free worker slot takes next.

use super::*;

/// Pick the next task to start from `tasks`, which is expected in queue order (as `list`
/// returns it). `gated` holds the ids of tasks that already have an open `dispatch` gate.
///
/// A held task at the head must not keep everything behind it from starting, so a task
/// with `autoStart: false` — or one the hub has not read yet (`needsReading`), whatever its
/// `autoStart` says — is skipped until its gate is open, and a task with a note
/// starting with "Could not start:" is skipped until a person has looked at it.
pub fn next(tasks: Vec<Task>, gated: &std::collections::HashSet<String>) -> Next {
    let mut picked = None;
    let mut needs_dispatch_gate = Vec::new();
    for task in tasks {
        if task.status != Status::Queued {
            continue;
        }
        if task
            .note
            .as_deref()
            .is_some_and(|n| n.starts_with(COULD_NOT_START))
        {
            continue;
        }
        if !task.auto_start || task.needs_reading {
            if !gated.contains(&task.id) {
                needs_dispatch_gate.push(task);
            }
            continue;
        }
        if picked.is_none() {
            picked = Some(task);
        }
    }
    Next {
        task: picked,
        needs_dispatch_gate,
    }
}
