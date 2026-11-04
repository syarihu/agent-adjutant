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
    /// The slug of the parent-task hub whose own read of the parent issue gave this parent its
    /// title (and its count, when the tracker gave none): the hub the page puts the title's state
    /// and its 「いま読む」 button beside. Absent when no hub is for this issue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_hub: Option<String>,
}

/// What a parent-task hub's own read of its parent issue holds.
pub(super) struct HubParent {
    /// The issue's key as `parent_key` writes it, which is what groups are keyed by.
    pub key: String,
    pub slug: String,
    pub title: Option<String>,
    pub total: Option<u32>,
}

/// How far a child has got, for the one bar a parent draws with a segment per child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Progress {
    /// Its pull request is merged: what `ParentGroup.merged` counts.
    Merged,
    /// The task is done and has no merged pull request.
    Done,
    /// A pull request is set (or the task says `pr`) and is not merged.
    Pr,
    /// A worker was handed it and has not made a pull request.
    Working,
    /// Anything else: not handed over yet, or cancelled (whatever pull request it had).
    NotStarted,
}

/// A child, joined by the hub that owns it and its id: an id is unique only within one hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentChild {
    pub hub: String,
    pub id: String,
    pub title: String,
    pub merged: bool,
    pub progress: Progress,
    /// When its PR's turn last changed, UTC stamp: a merged child has no row on the page, so this
    /// is all the page has to say when it merged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_turn_at: Option<String>,
    /// The branch its work is on, which the page names a stack's steps by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// What it is cut from, without a leading `origin/`. The base that names the sibling it is
    /// `on`; otherwise its pull request's base, else the record's, so the root of a stack names
    /// the branch the series starts from as GitHub has it now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The sibling whose branch this one is cut from, by the record's base else its pull
    /// request's: its id, and `on_hub` the hub that owns it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_hub: Option<String>,
    /// The siblings it comes after without being stacked on them (the record's `after`, kept to
    /// those that name a sibling of the same parent): their ids, and `after_hub` the hub that
    /// owns each, in the same order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub after_hub: Vec<String>,
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
    /// The parent-task hubs that have an issue, by that issue's key.
    pub hub_parents: &'a [HubParent],
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
    title: String,
    pr_turn_at: Option<String>,
    order: u32,
    created_at: String,
    /// The branch its worktree is on, else the one its pull request is from.
    branch: Option<String>,
    /// What the record says it is cut from, without a leading `origin/`. A record may hold
    /// `origin/main` here while its pull request is on a sibling's branch, so it does not win
    /// unless it names a sibling.
    record_base: Option<String>,
    /// The branch its pull request is based on, normalised the same way: how a stack built on
    /// GitHub after dispatch shows.
    pr_base: Option<String>,
    merged: bool,
    progress: Progress,
    /// The issue the task works on, which a sibling's `after` names it by.
    issue_url: Option<String>,
    /// The record's `after`, as written.
    after: Vec<String>,
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
            title: t.title.clone(),
            pr_turn_at: t.pr_turn_at.clone(),
            order: t.order,
            created_at: t.created_at.clone(),
            branch: branch_of(t, inputs.linked),
            record_base: branch_name(t.base.as_deref()),
            pr_base: branch_name(t.pr_status.as_ref().and_then(|p| p.base.as_deref())),
            merged: t.pr_status.as_ref().is_some_and(|p| p.state == "merged"),
            progress: progress_of(t),
            issue_url: t
                .issue_url
                .as_deref()
                .or(t.issue.as_deref())
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_string),
            after: t.after.clone(),
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
    if task.status == task::Status::Cancelled {
        Progress::NotStarted
    } else if merged {
        Progress::Merged
    } else if task.status == task::Status::Done {
        Progress::Done
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

/// A base as a branch name: trimmed, without a leading `origin/`, and nothing when empty.
fn branch_name(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .map(|b| b.strip_prefix("origin/").unwrap_or(b))
        .filter(|b| !b.is_empty())
        .map(str::to_string)
}

/// Whether `entry` (an `after` value: an issue URL, or a key) names the issue at `url`. A key
/// `issueKeys` resolves is turned into a URL the way a parent's is and compared as that URL
/// alone. Only a key it does not resolve is compared with the key the sibling's URL carries
/// (a Jira `/browse/KEY` or a Linear `/issue/KEY/...`, read as `adj task brief` reads it), so a
/// tracker's URL cannot stand in for the GitHub issue a configured key names.
fn names_issue(entry: &str, url: &str, inputs: &Inputs) -> bool {
    let entry = entry.trim();
    if !crate::kernel::brief::is_key(entry) {
        return task::parent_key(entry) == task::parent_key(url);
    }
    if let Some(resolved) = task::parent_issue_url(entry, inputs.issue_keys) {
        return task::parent_key(&resolved) == task::parent_key(url);
    }
    // The key is all the sibling's URL is read for: which tracker it is does not matter, so no
    // task sources are needed.
    crate::kernel::brief::tracker_and_key(url.trim(), &[], inputs.issue_keys)
        .is_some_and(|(_, key)| key.eq_ignore_ascii_case(entry))
}

/// For each child, the siblings (indexes) its `after` names: the ones that are in this group,
/// not itself, and not an edge that would close a loop. Children are taken in queue order and
/// an edge is kept only when its target does not already come after the child, so a cycle
/// loses its last-written edge and no child is lost.
fn siblings_before(kids: &[Kid], inputs: &Inputs) -> Vec<Vec<usize>> {
    fn reaches(from: usize, to: usize, edges: &[Vec<usize>], seen: &mut Vec<usize>) -> bool {
        if from == to {
            return true;
        }
        if seen.contains(&from) {
            return false;
        }
        seen.push(from);
        edges[from]
            .iter()
            .any(|&next| reaches(next, to, edges, seen))
    }
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); kids.len()];
    for i in 0..kids.len() {
        for entry in &kids[i].after {
            let Some(j) = (0..kids.len()).find(|&j| {
                j != i
                    && kids[j]
                        .issue_url
                        .as_deref()
                        .is_some_and(|url| names_issue(entry, url, inputs))
            }) else {
                continue;
            };
            if edges[i].contains(&j) || reaches(j, i, &edges, &mut Vec::new()) {
                continue;
            }
            edges[i].push(j);
        }
    }
    edges
}

fn group_of(mut acc: Acc, inputs: &Inputs) -> ParentGroup {
    acc.kids
        .sort_by(|a, b| (a.order, &a.created_at).cmp(&(b.order, &b.created_at)));
    // Whose branch each child is cut from, among the others; the first when two share a branch.
    // The record's base is taken when it names a sibling, else the pull request's. A base no
    // sibling is on (a branch of the repository, a sibling that is gone) is no step.
    let sibling_on = |i: usize, base: &str| {
        (0..acc.kids.len()).find(|&j| j != i && acc.kids[j].branch.as_deref() == Some(base))
    };
    let on: Vec<Option<usize>> = (0..acc.kids.len())
        .map(|i| {
            let kid = &acc.kids[i];
            [&kid.record_base, &kid.pr_base]
                .into_iter()
                .flatten()
                .find_map(|b| sibling_on(i, b))
        })
        .collect();
    // What each is shown cut from: the branch of the sibling it is on, else what its pull
    // request is based on now, else the record's.
    let bases: Vec<Option<String>> = (0..acc.kids.len())
        .map(|i| match on[i] {
            Some(j) => acc.kids[j].branch.clone(),
            None => acc.kids[i]
                .pr_base
                .clone()
                .or_else(|| acc.kids[i].record_base.clone()),
        })
        .collect();
    let before = siblings_before(&acc.kids, inputs);
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
    // The hub's own read counts only when the tracker was never asked about a child.
    let from_hub = inputs.hub_parents.iter().find(|h| h.key == acc.parent.key);
    let total = acc
        .tracker_total
        .or(from_hub.and_then(|h| h.total))
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
        title: acc.parent.title.or(from_hub.and_then(|h| h.title.clone())),
        children: order
            .iter()
            .map(|&i| ParentChild {
                hub: acc.kids[i].hub.clone(),
                id: acc.kids[i].id.clone(),
                title: acc.kids[i].title.clone(),
                merged: acc.kids[i].merged,
                progress: acc.kids[i].progress,
                pr_turn_at: acc.kids[i].pr_turn_at.clone(),
                branch: acc.kids[i].branch.clone(),
                base: bases[i].clone(),
                on: on[i].map(|j| acc.kids[j].id.clone()),
                on_hub: on[i].map(|j| acc.kids[j].hub.clone()),
                after: before[i].iter().map(|&j| acc.kids[j].id.clone()).collect(),
                after_hub: before[i].iter().map(|&j| acc.kids[j].hub.clone()).collect(),
            })
            .collect(),
        merged,
        total,
        stacked: on.iter().any(Option::is_some),
        hub,
        title_hub: from_hub.map(|h| h.slug.clone()),
    }
}

#[cfg(test)]
mod tests;
