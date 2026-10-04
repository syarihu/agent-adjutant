//! The boards index.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};

use crate::{gate, messaging, task};

use super::registry::{addresses, note_board, resident_board_url};

/// What a worker's record says about the task it is on, for `board_counts`.
pub(super) struct WorkerSeen {
    pub(super) task: Option<String>,
    pub(super) phase: Option<String>,
}

/// The worker record in `worktree`, if there is one. The task is read the way `worker_task`
/// reads it, minus the saved-session fallback: a record is all the page's join looks at.
fn worker_seen(worktree: &str) -> Option<WorkerSeen> {
    let messaging::Recorded::Found(record) = messaging::read_worker_record(Path::new(worktree))
    else {
        return None;
    };
    let text = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Some(WorkerSeen {
        task: text(&record.task),
        phase: text(&record.phase),
    })
}

/// How many things wait on the person and how many workers are at work on one board, from its
/// records alone — no `ps`, no git, no tmux — so the sidebar can ask every board at once.
///
/// This mirrors `humanColOf` in `src/ui/core.js` and the board's own "waiting" and "workers"
/// counts; keep the two in step. A task waits when it has an open gate, or when its pull
/// request is the person's ball: a Jules task with a pull request for which
/// `task::jules_pr_waits_on_person` says so, or a worker task for which `task::pr_waits_on_person`
/// says so (the state of its PR, then its worker's phase). A gate
/// whose task is not on the board waits too. Workers at work are the dispatched and `pr` tasks that
/// do not wait. The page's Jules check also looks at the session's live state, which only the
/// board's own poll has, so a Jules task that is still working counts as waiting here.
pub(super) fn board_counts(
    tasks: &[task::Task],
    gates: &[gate::Gate],
    worker: impl Fn(&str) -> Option<WorkerSeen>,
) -> (usize, usize) {
    let mut waiting = 0;
    let mut working = 0;
    for t in tasks {
        let waits = if gates
            .iter()
            .any(|g| g.task.as_deref() == Some(t.id.as_str()))
        {
            true
        } else if matches!(t.status, task::Status::Done | task::Status::Cancelled) {
            false
        } else if t.jules_session.is_some() && t.pr.is_some() {
            task::jules_pr_waits_on_person(t.pr_status.as_ref())
        } else {
            // Only a task with a PR has a worker record that matters here.
            let seen =
                t.pr.as_ref()
                    .and(t.worktree.as_deref())
                    .and_then(&worker)
                    .filter(|w| w.task.as_deref().is_none_or(|id| id == t.id));
            task::pr_waits_on_person(
                t.status,
                t.pr.is_some(),
                t.pr_status.as_ref(),
                seen.as_ref().map(|w| w.phase.as_deref()),
            )
        };
        if waits {
            waiting += 1;
        } else if matches!(t.status, task::Status::Dispatched | task::Status::Pr) {
            working += 1;
        }
    }
    waiting += gates
        .iter()
        .filter(|g| {
            g.task
                .as_deref()
                .is_none_or(|id| !tasks.iter().any(|t| t.id == id))
        })
        .count();
    (waiting, working)
}

/// The board list as `/api/boards` and `adj server status` give it. The counts (waiting,
/// working, and queued: tasks still to be started) are read from
/// each board's records (see `board_counts`), so one `ps` and one `git worktree list` per
/// repository serve every board.
pub(super) fn boards_json(root: &Path, port: u16, token: &str) -> Vec<Value> {
    let addresses = addresses(root);
    let table = messaging::ProcessTable::snapshot();
    // Workers per parent-task hub, counted by the hub their record reports to. Only asked of
    // a repository that has a parent-task hub listed.
    let mut children: HashMap<String, usize> = HashMap::new();
    let mut seen_mains: Vec<&str> = Vec::new();
    // Repositories whose worktree listing failed: their workers are unknown, so none of their
    // parent hubs may be called finished.
    let mut unlisted: Vec<&str> = Vec::new();
    for a in addresses.iter().filter(|a| a.hub.is_some()) {
        if seen_mains.contains(&a.main.as_str()) {
            continue;
        }
        seen_mains.push(&a.main);
        let listed = crate::kernel::identity::linked_worktrees(&a.main).unwrap_or_else(|_| {
            unlisted.push(&a.main);
            Vec::new()
        });
        for w in listed {
            if let Some(key) = messaging::worker_hub_key(Path::new(&w)) {
                *children
                    .entry(crate::kernel::identity::slug_for(&a.nwo, Some(&key)))
                    .or_insert(0) += 1;
            }
        }
    }
    addresses
        .iter()
        .map(|a| {
            let status = crate::kernel::identity::hub_name(&a.nwo, a.hub.as_deref())
                .ok()
                .map(|name| messaging::hub_status_with(root, &table, &a.slug, &name));
            let present = status.as_ref().is_some_and(|s| s.present);
            let tasks = task::list(&task::dir(root, &a.slug));
            let mut gates = gate::list(&gate::dir(root, &a.slug));
            let (waiting, working) = board_counts(&tasks, &gates, worker_seen);
            let queued = tasks
                .iter()
                .filter(|t| t.status == task::Status::Queued)
                .count();
            gates.sort_by(|x, y| x.opened_at.cmp(&y.opened_at));
            let gates: Vec<Value> = gates
                .iter()
                .map(|g| {
                    json!({
                        "id": g.id,
                        "kind": g.kind,
                        "title": g.title,
                        "openedAt": g.opened_at,
                        "task": g.task,
                        "worktree": g.worktree,
                    })
                })
                .collect();
            // When the hub is not there, when it was last seen alive: the session it would
            // resume says which heartbeat is its own.
            let last_alive = if present {
                None
            } else {
                messaging::hub_session(root, &a.slug).and_then(|saved| {
                    messaging::hub_last_alive(root, &a.slug, &saved.session_id)
                })
            };
            let finished = a.hub.is_some()
                && !present
                && !unlisted.contains(&a.main.as_str())
                && children.get(&a.slug).copied().unwrap_or(0) == 0
                && waiting == 0
                // A queued or backlog task has no worker or gate yet but is still to be done; a hub with
                // no task at all has not been used yet, and is still there to start.
                && !tasks.is_empty()
                && tasks
                    .iter()
                    .all(|t| matches!(t.status, task::Status::Done | task::Status::Cancelled));
            json!({
                "slug": a.slug,
                "nwo": a.nwo,
                "hub": a.hub,
                "url": resident_board_url(port, &a.slug, token),
                "hubPresent": present,
                "hubId": match &a.hub {
                    Some(key) => format!("hub-{}", key.trim()),
                    None => "hub".to_string(),
                },
                "hubStale": status.as_ref().is_some_and(|s| s.stale),
                "hubStartedAt": status.as_ref().and_then(|s| s.started_at.clone()),
                "hubLastAlive": last_alive,
                "title": a.hub.as_ref().and_then(|_| crate::cmd::hub_title::cached_title(root, &a.slug)),
                "waiting": waiting,
                "working": working,
                "queued": queued,
                "gates": gates,
                "finished": finished,
            })
        })
        .collect()
}

/// The repository this process stands in, if it stands in one. Whether it is one is not the
/// business of the commands that ask.
pub(super) fn checkout_here() -> Option<crate::kernel::identity::RepoInfo> {
    crate::cmd::resolve(None, None).ok()
}

/// Seed the address book from where this process stands and from the hub records already on
/// disk, so that the boards of hubs started before the resident are there from the first
/// request.
pub(super) fn seed_boards(root: &Path) {
    if let Some(repo) = checkout_here() {
        note_board(root, &repo);
    }
    for (slug, record) in messaging::hub_records(root) {
        let Some(cwd) = record.cwd.as_deref() else {
            continue;
        };
        if let Ok(repo) =
            crate::kernel::identity::resolve_in(Some(Path::new(cwd)), None, record.hub.as_deref())
            && repo.slug == slug
        {
            note_board(root, &repo);
        }
    }
}
