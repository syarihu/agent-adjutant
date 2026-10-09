//! The work under way across every repository the resident server serves, in one document.
//!
//! One carrier board answers for each repository: the
//! repository's own board, which lists the sessions and tasks of its parent-task hubs too, else
//! each parent-task board. So a poll reads one board per repository, not one per hub.
//!
//! Read-only, and from what `state` already reads. The page asks for it every couple of seconds
//! from several tabs at most, so the resident keeps the serialized document for a moment
//! (`Resident::work_cache`).

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::board::{Resident, Server, Session};
use crate::gate;
use crate::mail::RepoHub;
use crate::registry::{Address, addresses};
use crate::task;

use super::index::boards;
use super::parents::ParentGroup;
use super::rate_limits::RateLimitsState;
use super::state::{GateCard, Lines, TaskCard, state};

/// The document `GET /api/work` sends.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkState {
    pub now: i64,
    /// The first repository's: they are read from the agent session ledger, which no board owns.
    pub rate_limits: RateLimitsState,
    pub repos: Vec<WorkRepo>,
}

/// One repository: the sessions that are listed with the task each is on, and what the page
/// needs to draw them (hubs, parents).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkRepo {
    pub nwo: String,
    /// The slug of the board that answered first, which the page opens a terminal through when
    /// nothing says better.
    pub carrier: String,
    pub hubs: Vec<RepoHub>,
    pub parents: Vec<ParentGroup>,
    /// Every worker session and the repository's own hub, each with its task. The page draws the
    /// own hub as a row heading the group of the tasks with no parent (and as a chip on the
    /// repository's heading while that is folded); a parent-task hub is in `hub_sessions` and is a
    /// row of the group it runs.
    pub rows: Vec<WorkRow>,
    /// The sessions of the parent-task hubs, which a parent's terminal is.
    pub hub_sessions: Vec<WorkHubSession>,
    /// What waits on the person that a row may not show: the open gates of the carriers and the
    /// tasks whose PR is the person's turn, with or without a session. Not deduplicated against
    /// `rows`: the page merges them, as a gate can also be a session's `waiting`.
    pub turns: Vec<WorkTurn>,
    /// Set when no board of the repository could be read: not the same as it having no work.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A row of the list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkRow {
    /// The slug of the board the page opens the row on: the hub that owns the task, or the hub
    /// itself for a hub's row. Its actions and its task panel are that board's.
    pub board: String,
    pub session: Session,
    /// Absent for a hub and for a session with no task, or one whose record cannot be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<WorkTask>,
}

/// A parent-task hub's session and the board it belongs to.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkHubSession {
    pub board: String,
    pub session: Session,
}

/// What waits on the person on one board: a task (or none) and the open gates that are its.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkTurn {
    /// The slug of the board that owns the task, or of the carrier whose gates these are.
    pub board: String,
    /// Absent for gates that name no task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<WorkTask>,
    pub gates: Vec<WorkGate>,
}

/// An open gate, as much of it as the list shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkGate {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// UTC stamp, as a session's `waiting.openedAt`.
    pub opened_at: String,
    /// The board whose gate directory holds it.
    pub slug: String,
    /// The task it names, which may be one no turn above carries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

/// As much of a task as a row shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkTask {
    pub id: String,
    pub title: String,
    pub status: task::Status,
    /// The `key` of the parent issue the board joins children by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    /// `open`, `draft`, `merged` or `closed`, as the last read of the PR said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_turn: Option<task::PrTurn>,
    pub waits_on_person: bool,
    /// When a gate of this task was last answered, UTC stamp: acting on the task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_answered_at: Option<String>,
    /// When the PR's turn last changed, UTC stamp; absent on a record that has not seen it change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_turn_at: Option<String>,
    /// Set aside on purpose; absent for a done or cancelled task and for a blank reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parked: Option<task::Park>,
}

/// What a carrier board says that the document is made of.
#[derive(Clone)]
pub(super) struct Carried {
    pub slug: String,
    pub sessions: Vec<Session>,
    pub hubs: Vec<RepoHub>,
    pub tasks: Vec<TaskCard>,
    pub hub_tasks: Vec<TaskCard>,
    pub parents: Vec<ParentGroup>,
    pub gates: Vec<GateCard>,
    /// The open gates of the parent-task hubs a repository's own board lists the tasks of, with
    /// the slug of the hub whose directory holds each: the board's `gates` are its own.
    pub hub_gates: Vec<(String, GateCard)>,
}

/// The repositories of `addresses` with the boards that answer for each: the repository's own
/// board when there is one, else every parent-task board. Repositories in order of name.
pub(super) fn carriers(addresses: &[Address]) -> Vec<(String, Vec<String>)> {
    let mut by: Vec<(String, Vec<&Address>)> = Vec::new();
    for a in addresses {
        match by.iter_mut().find(|(nwo, _)| *nwo == a.nwo) {
            Some((_, list)) => list.push(a),
            None => by.push((a.nwo.clone(), vec![a])),
        }
    }
    by.sort_by(|a, b| a.0.cmp(&b.0));
    by.into_iter()
        .map(|(nwo, list)| {
            let slugs = match list.iter().find(|a| a.hub.is_none()) {
                Some(own) => vec![own.slug.clone()],
                None => list.iter().map(|a| a.slug.clone()).collect(),
            };
            (nwo, slugs)
        })
        .collect()
}

/// `addresses` without the boards that are finished, as the sidebar leaves them out: a parent-task
/// hub that is over has nothing to show.
pub(super) fn unfinished(addresses: Vec<Address>, finished: &[String]) -> Vec<Address> {
    addresses
        .into_iter()
        .filter(|a| !finished.contains(&a.slug))
        .collect()
}

pub fn work(resident: &Resident) -> WorkState {
    let mut rate_limits = None;
    // `finished` is the boards index's own answer, so the sidebar and this list cannot disagree.
    // Only a parent-task hub can be finished, so the index is not read when there is none.
    let all = addresses(&resident.root);
    let finished: Vec<String> = if all.iter().any(|a| a.hub.is_some()) {
        boards(&resident.root, resident.port, &resident.token)
            .into_iter()
            .filter(|b| b.finished)
            .map(|b| b.slug)
            .collect()
    } else {
        Vec::new()
    };
    let repos = carriers(&unfinished(all, &finished))
        .into_iter()
        .map(|(nwo, slugs)| {
            let mut carried = Vec::new();
            for slug in &slugs {
                let Some(server) = resident.board(slug) else {
                    continue;
                };
                let board = state(&server, true, Lines::None);
                rate_limits.get_or_insert_with(|| board.rate_limits.clone());
                let hub_gates = hub_gates(&server, &board.hubs);
                carried.push(Carried {
                    slug: slug.clone(),
                    sessions: board.sessions,
                    hubs: board.hubs,
                    tasks: board.tasks,
                    hub_tasks: board.hub_tasks,
                    parents: board.parents,
                    gates: board.gates,
                    hub_gates,
                });
            }
            repo_of(nwo, carried)
        })
        .collect();
    WorkState {
        now: crate::infra::clock::now_secs(),
        rate_limits: rate_limits.unwrap_or_default(),
        repos,
    }
}

/// The open gates of the parent-task hubs whose tasks the repository's own board lists, each with
/// the slug of its hub. A hub whose directory cannot be read has none, as for its task cards.
fn hub_gates(server: &Server, hubs: &[RepoHub]) -> Vec<(String, GateCard)> {
    let repo = &server.ctx.repo;
    if repo.hub.is_some() {
        return Vec::new();
    }
    hubs.iter()
        .filter(|h| h.parent && h.slug != repo.slug)
        .flat_map(|h| {
            gate::list(&server.ctx.state, &h.slug, gate::Shelf::Open)
                .into_iter()
                .map(|g| (h.slug.clone(), GateCard::of(g)))
        })
        .collect()
}

/// One repository out of what its carriers said. Everything is listed once however many
/// carriers name it: a parent-task board lists its repository's workers too.
pub(super) fn repo_of(nwo: String, carried: Vec<Carried>) -> WorkRepo {
    let Some(first) = carried.first().map(|c| c.slug.clone()) else {
        return WorkRepo {
            nwo,
            carrier: String::new(),
            hubs: Vec::new(),
            parents: Vec::new(),
            rows: Vec::new(),
            hub_sessions: Vec::new(),
            turns: Vec::new(),
            error: Some("no board of this repository could be read".to_string()),
        };
    };
    let mut hubs: Vec<RepoHub> = Vec::new();
    for hub in carried.iter().flat_map(|c| c.hubs.iter()) {
        if !hubs.iter().any(|h| h.id == hub.id) {
            hubs.push(hub.clone());
        }
    }
    // A task is found by the hub that owns it and its id: an id is unique only within one hub.
    let mut tasks: HashMap<(&str, &str), &TaskCard> = HashMap::new();
    for c in &carried {
        for card in &c.tasks {
            tasks.entry((&c.slug, &card.task.id)).or_insert(card);
        }
        for card in &c.hub_tasks {
            if let Some(owner) = &card.owner_hub {
                tasks.entry((&owner.slug, &card.task.id)).or_insert(card);
            }
        }
    }
    let hub_slug = |id: &str| hubs.iter().find(|h| h.id == id).map(|h| h.slug.clone());
    let mut seen: HashSet<&str> = HashSet::new();
    let mut rows = Vec::new();
    let mut hub_sessions = Vec::new();
    for c in &carried {
        for session in &c.sessions {
            if !seen.insert(&session.id) {
                continue;
            }
            if session.kind == "hub" {
                let board = hub_slug(&session.id).unwrap_or_else(|| c.slug.clone());
                // `parent` says it for a parent-task hub whose record carried no key when it started.
                let parent = hubs
                    .iter()
                    .find(|h| h.id == session.id)
                    .map_or(session.key.is_some(), |h| h.parent);
                if parent {
                    hub_sessions.push(WorkHubSession {
                        board,
                        session: session.clone(),
                    });
                } else {
                    rows.push(WorkRow {
                        board,
                        session: session.clone(),
                        task: None,
                    });
                }
                continue;
            }
            let board = session
                .hub
                .as_deref()
                .and_then(hub_slug)
                .unwrap_or_else(|| c.slug.clone());
            let task = session
                .task
                .as_deref()
                .and_then(|id| tasks.get(&(board.as_str(), id)))
                .map(|card| work_task(card));
            rows.push(WorkRow {
                board,
                session: session.clone(),
                task,
            });
        }
    }
    let turns = turns_of(&carried);
    // A parent seen by two carriers is the one that knows more of its children.
    let mut parents: Vec<ParentGroup> = Vec::new();
    for group in carried.into_iter().flat_map(|c| c.parents) {
        match parents.iter_mut().find(|p| p.key == group.key) {
            Some(have) if have.children.len() < group.children.len() => *have = group,
            Some(_) => {}
            None => parents.push(group),
        }
    }
    parents.sort_by(|a, b| a.key.cmp(&b.key));
    WorkRepo {
        nwo,
        carrier: first,
        hubs,
        parents,
        rows,
        hub_sessions,
        turns,
        error: None,
    }
}

/// The open gates that wait on a person, grouped by the board and the task they name, and a turn
/// for each task whose PR is the person's or that is parked (so a parked task with nothing
/// waiting and no session still has a row), each on the board that owns it. A gate that is only
/// recorded for the board (`wait: false`) is not a turn.
fn turns_of(carried: &[Carried]) -> Vec<WorkTurn> {
    let mut turns: Vec<WorkTurn> = Vec::new();
    let mut named: HashSet<(&str, &str)> = HashSet::new();
    for c in carried {
        let owned = c.tasks.iter().map(|card| (c.slug.as_str(), card));
        let others = c
            .hub_tasks
            .iter()
            .filter_map(|card| Some((card.owner_hub.as_ref()?.slug.as_str(), card)));
        for (board, card) in owned.chain(others) {
            if (card.waits_on_person || card.task.park().is_some())
                && named.insert((board, card.task.id.as_str()))
            {
                turns.push(WorkTurn {
                    board: board.to_string(),
                    task: Some(work_task(card)),
                    gates: Vec::new(),
                });
            }
        }
    }
    // The card of a task of the board `slug`: the board's own, or one of a parent-task hub it lists.
    let card_of = |slug: &str, id: &str| {
        carried.iter().find_map(|c| {
            let own = (c.slug == slug).then(|| c.tasks.iter().find(|card| card.task.id == id));
            own.flatten().or_else(|| {
                c.hub_tasks.iter().find(|card| {
                    card.task.id == id && card.owner_hub.as_ref().is_some_and(|o| o.slug == slug)
                })
            })
        })
    };
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    let gates = carried.iter().flat_map(|c| {
        let own = c.gates.iter().map(|g| (c.slug.as_str(), g));
        own.chain(c.hub_gates.iter().map(|(slug, g)| (slug.as_str(), g)))
    });
    for (board, GateCard { gate, .. }) in gates {
        if !gate.wait || !seen.insert((board, &gate.id)) {
            continue;
        }
        let at = turns.iter().position(|t| {
            t.board == board
                && match &t.task {
                    Some(task) => gate.task.as_deref() == Some(task.id.as_str()),
                    None => t.gates.iter().any(|g| g.task == gate.task),
                }
        });
        let at = at.unwrap_or_else(|| {
            // A task this board does not list leaves the turn without one.
            let task = gate
                .task
                .as_deref()
                .and_then(|id| card_of(board, id))
                .map(work_task);
            turns.push(WorkTurn {
                board: board.to_string(),
                task,
                gates: Vec::new(),
            });
            turns.len() - 1
        });
        turns[at].gates.push(WorkGate {
            id: gate.id.clone(),
            kind: gate.kind.as_str().to_string(),
            title: Some(gate.title.clone()).filter(|t| !t.is_empty()),
            opened_at: gate.opened_at.clone(),
            slug: board.to_string(),
            task: gate.task.clone(),
        });
    }
    turns
}

fn work_task(card: &TaskCard) -> WorkTask {
    WorkTask {
        id: card.task.id.clone(),
        title: card.task.title.clone(),
        status: card.task.status,
        parent: card.parent_issue.as_ref().map(|p| p.key.clone()),
        pr: card.task.pr.clone(),
        pr_state: card.task.pr_status.as_ref().map(|p| p.state.clone()),
        pr_turn: card.live.as_ref().and_then(|l| l.pr_turn),
        waits_on_person: card.waits_on_person,
        gate_answered_at: card.task.gate_answered_at.clone(),
        pr_turn_at: card.task.pr_turn_at.clone(),
        parked: card.task.park().cloned(),
    }
}

#[cfg(test)]
mod tests;
