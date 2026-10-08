//! The work under way across every repository the resident server serves, in one document.
//!
//! One carrier board answers for each repository, the way the page's セッション tab asks: the
//! repository's own board, which lists the sessions and tasks of its parent-task hubs too, else
//! each parent-task board. So a poll reads one board per repository, not one per hub.
//!
//! Read-only, and from what `state` already reads. The page asks for it every couple of seconds
//! from several tabs at most, so the resident keeps the serialized document for a moment
//! (`Resident::work_cache`).

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::board::{Resident, Session};
use crate::mail::RepoHub;
use crate::registry::{Address, addresses};
use crate::task;

use super::index::boards;
use super::parents::ParentGroup;
use super::rate_limits::RateLimitsState;
use super::state::{Lines, TaskCard, state};

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
    /// Every worker session and the repository's own hub, each with its task. A parent-task
    /// hub is not a row (it is reached from its parent) and is in `hub_sessions`.
    pub rows: Vec<WorkRow>,
    /// The sessions of the parent-task hubs, which a parent's terminal is.
    pub hub_sessions: Vec<WorkHubSession>,
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
}

/// What a carrier board says that the document is made of.
pub(super) struct Carried {
    pub slug: String,
    pub sessions: Vec<Session>,
    pub hubs: Vec<RepoHub>,
    pub tasks: Vec<TaskCard>,
    pub hub_tasks: Vec<TaskCard>,
    pub parents: Vec<ParentGroup>,
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
                carried.push(Carried {
                    slug: slug.clone(),
                    sessions: board.sessions,
                    hubs: board.hubs,
                    tasks: board.tasks,
                    hub_tasks: board.hub_tasks,
                    parents: board.parents,
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
        error: None,
    }
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
    }
}

#[cfg(test)]
mod tests;
