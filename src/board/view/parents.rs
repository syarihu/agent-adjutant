//! Which parent issue each task belongs to, and what each parent has of its children.
//!
//! The tracker's word comes first: an issue that is a sub-issue on GitHub is a child of its
//! parent there, however the record was filled in. The record's own `parent` is what is left
//! when the tracker has none, was never asked, or could not be reached. The tracker never clears
//! a record's value: it only outranks it.
//!
//! Read-only and from memory (the poll keeps what the tracker said), so a poll of `/api/state`
//! never reaches GitHub.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::kernel::identity::Worktree;
use crate::task::{self, TrackerParent};

use super::state::TaskCard;

/// Where the parent a card shows came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ParentSource {
    Tracker,
    Record,
}

/// The parent issue of one card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentIssue {
    /// What the board joins children by: `owner/repo#N` for an issue on GitHub, else the value.
    pub key: String,
    /// A link when the parent is a URL; the record's own text when it is a key that names no
    /// repository this board knows.
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub source: ParentSource,
    /// The slug of the hub that owns the task, which is what `parents[].children[].hub` names it by:
    /// the page joins on it and does not work it out from where it was loaded.
    pub hub: String,
    /// The record's own `parent` as a `key`, when it names an issue (a URL, or a key `issueKeys`
    /// resolves). The page says the record differs when this is present and is not `key`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_key: Option<String>,
}

/// One parent with its children, as the board's `parents` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentGroup {
    pub key: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// In stack order, root first, when the children are stacked; the rest in queue order.
    pub children: Vec<ParentChild>,
    /// How many of the children have a merged pull request.
    pub merged: usize,
    /// How many children the parent has: what the tracker counts when it counts, but never fewer
    /// than the board lists.
    pub total: usize,
    /// Whether some child branches from another's branch.
    pub stacked: bool,
    /// The slug of the hub that runs the children: a parent-task hub among the children's when
    /// there is one, else the repository's own. Which terminal the page opens for the parent.
    pub hub: String,
}

/// How far a child has got, for the one bar a parent draws with a segment per child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Progress {
    /// Its pull request is merged, or the task is done.
    Merged,
    /// A pull request is set (or the task says `pr`) and is not merged.
    Pr,
    /// A worker was handed it and has not made a pull request.
    Working,
    /// Anything else: not handed over yet, or cancelled.
    NotStarted,
}

/// A child, joined by the hub that owns it and its id: an id is unique only within one hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentChild {
    pub hub: String,
    pub id: String,
    pub merged: bool,
    pub progress: Progress,
    /// The branch its work is on, which the page names a stack's steps by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// What it is cut from, without a leading `origin/`: the root of a stack names the branch
    /// the series starts from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The sibling whose branch this one is cut from: its id, and `on_hub` the hub that owns it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_hub: Option<String>,
}

/// What `attach` reads besides the cards.
pub(super) struct Inputs<'a> {
    /// The slug of the hub the board's own cards belong to.
    pub slug: &'a str,
    pub issue_keys: &'a serde_json::Map<String, serde_json::Value>,
    /// The linked worktrees, which say which branch a task's worktree is on.
    pub linked: &'a [Worktree],
    /// The slugs of the parent-task hubs of the repository, which a parent's children may be
    /// run by.
    pub parent_hubs: &'a [String],
    /// What the tracker last said the issue at a URL has for a parent.
    pub tracker: &'a dyn Fn(&str) -> Option<TrackerParent>,
}

/// A card's parent, and how many sub-issues the tracker counts under it.
struct Resolved {
    parent: ParentIssue,
    tracker_total: Option<u32>,
}

/// The record's `parent` as the issue it names: its URL, key and number. A key `issueKeys` does
/// not resolve is shown as written and names no issue (`None` in the second place).
fn record_parent(task: &task::Task, inputs: &Inputs) -> Option<(ParentIssue, Option<String>)> {
    let raw = task
        .parent
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())?;
    // A key is turned into the issue it names where `issueKeys` says which repository that is.
    let url = match crate::kernel::brief::is_key(raw) {
        true => task::parent_issue_url(raw, inputs.issue_keys),
        false => Some(raw.to_string()),
    };
    Some(match url {
        Some(url) => {
            let key = task::parent_key(&url);
            let parent = ParentIssue {
                key: key.clone(),
                number: task::issue_number(&url),
                url,
                title: None,
                source: ParentSource::Record,
                hub: String::new(),
                record_key: Some(key.clone()),
            };
            (parent, Some(key))
        }
        None => {
            let parent = ParentIssue {
                key: raw.to_string(),
                url: raw.to_string(),
                number: None,
                title: None,
                source: ParentSource::Record,
                hub: String::new(),
                record_key: None,
            };
            (parent, None)
        }
    })
}

/// The parent of `task`: the tracker's, else the record's.
fn resolve(task: &task::Task, inputs: &Inputs) -> Option<Resolved> {
    let record = record_parent(task, inputs);
    let tracked = task::issue_to_fetch(task).and_then(|url| (inputs.tracker)(url));
    if let Some(t) = tracked {
        return Some(Resolved {
            parent: ParentIssue {
                key: task::parent_key(&t.url),
                url: t.url,
                number: Some(t.number),
                title: Some(t.title).filter(|t| !t.is_empty()),
                source: ParentSource::Tracker,
                hub: String::new(),
                record_key: record.and_then(|(_, key)| key),
            },
            tracker_total: Some(t.total),
        });
    }
    Some(Resolved {
        parent: record?.0,
        tracker_total: None,
    })
}

/// A child while the groups are built.
struct Kid {
    hub: String,
    id: String,
    order: u32,
    created_at: String,
    /// The branch its worktree is on, else the one its pull request is from.
    branch: Option<String>,
    /// What it is cut from, without a leading `origin/`.
    base: Option<String>,
    merged: bool,
    progress: Progress,
}

struct Acc {
    parent: ParentIssue,
    tracker_total: Option<u32>,
    kids: Vec<Kid>,
}

/// Set `parent_issue` on every card that has a parent, and return the parents with their
/// children: the board's own cards and those of the parent-task hubs it lists, which a
/// parent's children may be spread across.
pub(super) fn attach(
    tasks: &mut [TaskCard],
    hub_tasks: &mut [TaskCard],
    inputs: &Inputs,
) -> Vec<ParentGroup> {
    let mut groups: BTreeMap<String, Acc> = BTreeMap::new();
    for card in tasks.iter_mut().chain(hub_tasks.iter_mut()) {
        let Some(resolved) = resolve(&card.task, inputs) else {
            continue;
        };
        let hub = card
            .owner_hub
            .as_ref()
            .map_or(inputs.slug, |o| o.slug.as_str())
            .to_string();
        let t = &card.task;
        let kid_hub = hub.clone();
        let kid = Kid {
            hub,
            id: t.id.clone(),
            order: t.order,
            created_at: t.created_at.clone(),
            branch: branch_of(t, inputs.linked),
            base: t
                .base
                .as_deref()
                .map(str::trim)
                .map(|b| b.strip_prefix("origin/").unwrap_or(b))
                .filter(|b| !b.is_empty())
                .map(str::to_string),
            merged: t.pr_status.as_ref().is_some_and(|p| p.state == "merged"),
            progress: progress_of(t),
        };
        let acc = groups
            .entry(resolved.parent.key.clone())
            .or_insert_with(|| Acc {
                parent: resolved.parent.clone(),
                tracker_total: None,
                kids: Vec::new(),
            });
        // The tracker's description of the parent, with its title and number, outranks the
        // record's spelling of the same issue.
        if resolved.parent.source == ParentSource::Tracker
            && acc.parent.source != ParentSource::Tracker
        {
            acc.parent = resolved.parent.clone();
        }
        acc.tracker_total = acc.tracker_total.or(resolved.tracker_total);
        acc.kids.push(kid);
        let mut shown = resolved.parent;
        shown.hub = kid_hub;
        card.parent_issue = Some(shown);
    }
    groups
        .into_values()
        .map(|acc| group_of(acc, inputs))
        .collect()
}

fn progress_of(task: &task::Task) -> Progress {
    let merged = task.pr_status.as_ref().is_some_and(|p| p.state == "merged");
    if merged || task.status == task::Status::Done {
        Progress::Merged
    } else if task.pr.is_some() || task.status == task::Status::Pr {
        Progress::Pr
    } else if task.status == task::Status::Dispatched {
        Progress::Working
    } else {
        Progress::NotStarted
    }
}

/// The branch a task's work is on: its worktree's, when that is still there, else the head of
/// its pull request.
fn branch_of(task: &task::Task, linked: &[Worktree]) -> Option<String> {
    let from_worktree = task.worktree.as_deref().and_then(|path| {
        linked
            .iter()
            .find(|w| Path::new(&w.path) == Path::new(path))
            .and_then(|w| w.branch.clone())
    });
    from_worktree.or_else(|| task.pr_status.as_ref().and_then(|p| p.head.clone()))
}

fn group_of(mut acc: Acc, inputs: &Inputs) -> ParentGroup {
    acc.kids
        .sort_by(|a, b| (a.order, &a.created_at).cmp(&(b.order, &b.created_at)));
    // Whose branch each child is cut from, among the others; the first when two share a branch.
    // A base no sibling is on (a branch of the repository, a sibling that is gone) is no step.
    let on: Vec<Option<usize>> = (0..acc.kids.len())
        .map(|i| {
            let base = acc.kids[i].base.as_deref()?;
            (0..acc.kids.len()).find(|&j| j != i && acc.kids[j].branch.as_deref() == Some(base))
        })
        .collect();
    let in_stack = |i: usize| on[i].is_some() || on.contains(&Some(i));
    // Each chain root first, then everything cut from it, depth first; children that share a base
    // come in queue order. A cycle has no root, so what the walk did not reach follows in queue
    // order rather than being lost.
    fn walk(at: usize, on: &[Option<usize>], seen: &mut Vec<usize>) {
        if seen.contains(&at) {
            return;
        }
        seen.push(at);
        for next in (0..on.len()).filter(|&j| on[j] == Some(at)) {
            walk(next, on, seen);
        }
    }
    let mut order: Vec<usize> = Vec::new();
    for root in (0..acc.kids.len()).filter(|&i| on[i].is_none() && in_stack(i)) {
        walk(root, &on, &mut order);
    }
    let rest: Vec<usize> = (0..acc.kids.len())
        .filter(|&i| in_stack(i) && !order.contains(&i))
        .collect();
    order.extend(rest);
    order.extend((0..acc.kids.len()).filter(|&i| !in_stack(i)));
    let merged = acc.kids.iter().filter(|k| k.merged).count();
    // Never fewer than the children listed: a child only the record knows is not among the
    // tracker's count, and "3 / 2" would read as an error.
    let total = acc
        .tracker_total
        .map_or(acc.kids.len(), |total| (total as usize).max(acc.kids.len()));
    // A parent-task hub that runs one of the children runs the parent; otherwise the children
    // are the repository's own.
    let hub = acc
        .kids
        .iter()
        .map(|k| k.hub.as_str())
        .find(|hub| inputs.parent_hubs.iter().any(|p| p == hub))
        .unwrap_or(inputs.slug)
        .to_string();
    ParentGroup {
        key: acc.parent.key,
        url: acc.parent.url,
        number: acc.parent.number,
        title: acc.parent.title,
        children: order
            .iter()
            .map(|&i| ParentChild {
                hub: acc.kids[i].hub.clone(),
                id: acc.kids[i].id.clone(),
                merged: acc.kids[i].merged,
                progress: acc.kids[i].progress,
                branch: acc.kids[i].branch.clone(),
                base: acc.kids[i].base.clone(),
                on: on[i].map(|j| acc.kids[j].id.clone()),
                on_hub: on[i].map(|j| acc.kids[j].hub.clone()),
            })
            .collect(),
        merged,
        total,
        stacked: on.iter().any(Option::is_some),
        hub,
    }
}

#[cfg(test)]
mod tests;
