//! The boards index.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::board::resident_board_url;
use crate::gate;
use crate::registry::addresses;
use crate::task;

/// What a worker's record says about the task it is on, for `board_counts`.
pub struct WorkerSeen {
    pub task: Option<String>,
    pub phase: Option<String>,
}

/// The worker record in `worktree`, if there is one. The task is read the way `worker_task`
/// reads it, minus the saved-session fallback: a record is all the page's join looks at.
fn worker_seen(worktree: &str) -> Option<WorkerSeen> {
    let crate::registry::Recorded::Found(record) =
        crate::registry::read_worker_record(Path::new(worktree))
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
pub fn board_counts(
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

/// A gate waiting on the person, as the boards index lists it. The fields are in alphabetical
/// order of their JSON names because the index used to be written with its keys sorted and
/// `/api/boards` serializes in field order.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateSummary {
    pub id: String,
    pub kind: crate::gate::Kind,
    pub opened_at: String,
    // Serialized as `null` when there is none, as it always was.
    pub task: Option<String>,
    pub title: String,
    pub worktree: String,
}

/// One board of the index. The fields are in alphabetical order of their JSON names because
/// the index used to be written with its keys sorted and `/api/boards` serializes in field
/// order.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSummary {
    pub finished: bool,
    pub gates: Vec<GateSummary>,
    pub hub: Option<String>,
    pub hub_id: String,
    pub hub_last_alive: Option<i64>,
    pub hub_present: bool,
    pub hub_stale: bool,
    pub hub_started_at: Option<String>,
    pub nwo: String,
    pub queued: usize,
    pub slug: String,
    pub title: Option<String>,
    pub url: String,
    pub waiting: usize,
    pub working: usize,
}

/// The board list as `/api/boards` and `adj server status` give it. The counts (waiting,
/// working, and queued: tasks still to be started) are read from
/// each board's records (see `board_counts`), so one `ps` and one `git worktree list` per
/// repository serve every board.
pub fn boards(root: &Path, port: u16, token: &str) -> Vec<BoardSummary> {
    let addresses = addresses(root);
    let table = crate::registry::ProcessTable::snapshot();
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
            if let Some(key) = crate::registry::worker_hub_key(Path::new(&w)) {
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
                .map(|name| crate::registry::hub_status_with(root, &table, &a.slug, &name));
            let present = status.as_ref().is_some_and(|s| s.present);
            let tasks = task::list(root, &a.slug);
            let mut gates = gate::list(root, &a.slug, gate::Shelf::Open);
            let (waiting, working) = board_counts(&tasks, &gates, worker_seen);
            let queued = tasks
                .iter()
                .filter(|t| t.status == task::Status::Queued)
                .count();
            gates.sort_by(|x, y| x.opened_at.cmp(&y.opened_at));
            let gates: Vec<GateSummary> = gates
                .iter()
                .map(|g| GateSummary {
                    id: g.id.clone(),
                    kind: g.kind,
                    opened_at: g.opened_at.clone(),
                    task: g.task.clone(),
                    title: g.title.clone(),
                    worktree: g.worktree.clone(),
                })
                .collect();
            // When the hub is not there, when it was last seen alive: the session it would
            // resume says which heartbeat is its own.
            let last_alive = if present {
                None
            } else {
                crate::registry::hub_session(root, &a.slug).and_then(|saved| {
                    crate::registry::hub_last_alive(root, &a.slug, &saved.session_id)
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
            BoardSummary {
                finished,
                gates,
                hub: a.hub.clone(),
                hub_id: match &a.hub {
                    Some(key) => format!("hub-{}", key.trim()),
                    None => "hub".to_string(),
                },
                hub_last_alive: last_alive,
                hub_present: present,
                hub_stale: status.as_ref().is_some_and(|s| s.stale),
                hub_started_at: status.as_ref().and_then(|s| s.started_at.clone()),
                nwo: a.nwo.clone(),
                queued,
                slug: a.slug.clone(),
                title: a
                    .hub
                    .as_ref()
                    .and_then(|_| crate::board::jobs::cached_title(root, &a.slug)),
                url: resident_board_url(port, &a.slug, token),
                waiting,
                working,
            }
        })
        .collect()
}
