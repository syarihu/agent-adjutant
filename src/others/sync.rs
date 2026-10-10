//! The sync: search for the PRs that ask for your review, read each of them (and every PR still
//! tracked) by number, and bring the records up to date.
//!
//! Runs only when a person asks for it. Reading is done first, outside any lock; each record is
//! then changed under its own lock, one step each.

use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::infra::clock::{now_secs, utc_stamp};
use crate::task::{PrRef, pr_ref};

use super::github::{self, Facts, Gh, Hit};
use super::list::list;
use super::model::{Events, FileStat, LastSync, PrOpenState, Record, State, SyncFailure, derive};
use super::scope::{Scope, scope};
use super::store;

/// How long the whole sync may take. The click that asks for it is waited on, and this is the
/// round the PR poll gives itself.
const SYNC_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a record that is done stays, in seconds.
const DONE_KEPT_SECS: i64 = 86_400;

/// What a sync came to.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Synced {
    pub last_sync: LastSync,
    /// The records whose request, or re-request, is new since the last sync.
    pub arrived: Vec<String>,
    /// The records taken away for having been done for a day.
    pub removed: Vec<String>,
    pub records: Vec<Record>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// Another sync holds the lock.
    Busy,
    /// The sync could not be done at all, and why.
    Failed(String),
}

/// Sync with the real `gh` and the real clock.
pub fn sync(root: &Path) -> Result<Synced, SyncError> {
    sync_with(root, &scope(root), now_secs(), &|args, deadline| {
        crate::infra::gh::run(None, args, deadline)
    })
}

/// `sync`, with the scope, the time and the way to run `gh` handed in.
pub(crate) fn sync_with(root: &Path, scope: &Scope, now: i64, gh: Gh) -> Result<Synced, SyncError> {
    let _sync = match store::try_lock_sync(root) {
        Ok(Some(lock)) => lock,
        Ok(None) => return Err(SyncError::Busy),
        Err(e) => return Err(SyncError::Failed(e)),
    };
    let deadline = Instant::now() + SYNC_TIMEOUT;
    let stamp = utc_stamp(now);
    let whole = |why: String| failed_as_a_whole(root, &stamp, why);

    let me = github::viewer(gh, deadline).map_err(&whole)?;
    // With no owner there is nothing to search: no `--owner` would search all of GitHub.
    let (hits, truncated) = if scope.owners.is_empty() {
        (Vec::new(), false)
    } else {
        github::search(gh, &scope.owners, deadline).map_err(&whole)?
    };

    // The hits, and every record that is not done: GitHub stops listing you as a reviewer once
    // you have reviewed, so a PR is read by number whether or not the search found it.
    let mut targets = hits;
    targets.sort_by_key(|h| store::id_of(&h.pr));
    let mut tracked: Vec<Hit> = store::list(root)
        .into_iter()
        .filter(|r| r.state != State::Done)
        .filter_map(|r| {
            Some(Hit {
                pr: pr_ref(&r.url, None)?,
                repo: r.repo,
                url: r.url,
            })
        })
        .collect();
    tracked.sort_by_key(|h| store::id_of(&h.pr));
    targets.extend(tracked);
    let mut seen = std::collections::HashSet::new();
    targets.retain(|h| seen.insert(store::id_of(&h.pr)));

    let refs: Vec<&PrRef> = targets.iter().map(|h| &h.pr).collect();
    let read = github::read_details(gh, &me, &refs, deadline);

    let mut failed: Vec<SyncFailure> = Vec::new();
    let mut arrived: Vec<String> = Vec::new();
    for (target, read) in targets.iter().zip(read) {
        let id = store::id_of(&target.pr);
        let applied = read.and_then(|facts| {
            let files = files_of(root, gh, &target.pr, &facts, deadline);
            apply(root, scope, now, target, facts, files)
        });
        match applied {
            Ok(true) => arrived.push(id),
            Ok(false) => {}
            Err(why) => failed.push(SyncFailure { id, why }),
        }
    }

    let removed = remove_done(root, now);
    let mut last = store::read_last_sync(root).unwrap_or_default();
    last.at = Some(stamp.clone());
    last.complete = failed.is_empty();
    last.owners = scope.owners.clone();
    last.truncated = truncated;
    last.failed = failed;
    last.error = None;
    last.error_at = None;
    store::write_last_sync(root, &last).map_err(SyncError::Failed)?;
    Ok(Synced {
        last_sync: last,
        arrived,
        removed,
        records: list(root),
    })
}

/// A sync that could not be done: only the reason is written, so what the last good sync left
/// (its time, its owners, what it could not read) still says what the records are as of.
fn failed_as_a_whole(root: &Path, stamp: &str, why: String) -> SyncError {
    let mut last = store::read_last_sync(root).unwrap_or_default();
    last.error = Some(why.clone());
    last.error_at = Some(stamp.to_string());
    match store::write_last_sync(root, &last) {
        Ok(()) => SyncError::Failed(why),
        Err(e) => SyncError::Failed(format!("{why} ({e})")),
    }
}

/// The files of a PR whose first page was not all of them: `None` when the stored ones are
/// still the right ones (the head has not moved and they were complete), else the first page
/// with whatever more pages could be read.
pub(super) fn files_of(
    root: &Path,
    gh: Gh,
    pr: &PrRef,
    facts: &Facts,
    deadline: Instant,
) -> Option<Vec<FileStat>> {
    let Some(after) = facts.files_after.as_deref() else {
        return Some(facts.files.clone());
    };
    // A record that cannot be read is the one `apply` fails on; here it only means reading again.
    if let Ok(Some(prev)) = store::load(root, pr)
        && prev.files_complete
        && prev.head_sha == facts.head_sha
        // A change of base can change the list without moving the head.
        && (prev.changed_files, prev.additions, prev.deletions)
            == (facts.changed_files, facts.additions, facts.deletions)
    {
        return None;
    }
    let mut files = facts.files.clone();
    if Instant::now() < deadline
        && let Ok((more, _)) = github::more_files(gh, pr, after, deadline)
    {
        files.extend(more);
    }
    Some(files)
}

/// Bring the record of `target` up to date with `facts`. `Ok(true)` when a request or a
/// re-request arrived with it. A PR that is not recorded and was not asked of you (a team's
/// request) is left out.
fn apply(
    root: &Path,
    scope: &Scope,
    now: i64,
    target: &Hit,
    facts: Facts,
    files: Option<Vec<FileStat>>,
) -> Result<bool, String> {
    let stem = store::stem(&target.pr);
    let _lock = store::lock(root, &stem)?;
    // Loaded again under the lock: what the read-through wrote since the listing is kept.
    let prev = store::load(root, &target.pr)?;
    if prev.is_none() && !facts.requested {
        return Ok(false);
    }
    let stamp = utc_stamp(now);
    let mut next = match &prev {
        Some(prev) => prev.clone(),
        None => Record {
            id: store::id_of(&target.pr),
            repo: target.repo.clone(),
            number: target.pr.number,
            url: target.url.clone(),
            registered: false,
            title: String::new(),
            author: None,
            base: String::new(),
            head: String::new(),
            head_sha: String::new(),
            draft: false,
            pr_state: PrOpenState::Open,
            additions: 0,
            deletions: 0,
            changed_files: 0,
            files: Vec::new(),
            files_complete: false,
            ci: Default::default(),
            reviewers: Vec::new(),
            my_review: None,
            commits_since_review: None,
            requested: false,
            requested_at: None,
            rerequest: false,
            state: State::Requested,
            done_reason: None,
            done_at: None,
            events: Events::default(),
            ai: None,
            first_seen_at: stamp.clone(),
            read_at: stamp.clone(),
            extra: Default::default(),
        },
    };
    next.registered = scope.registered.contains(&target.pr.nwo());
    next.title = facts.title;
    next.author = facts.author;
    next.base = facts.base;
    next.head = facts.head;
    next.head_sha = facts.head_sha;
    next.draft = facts.draft;
    next.pr_state = facts.pr_state;
    next.additions = facts.additions;
    next.deletions = facts.deletions;
    next.changed_files = facts.changed_files;
    if let Some(files) = files {
        next.files_complete = files.len() as u64 >= facts.changed_files;
        next.files = files;
    }
    next.ci = facts.ci;
    next.reviewers = facts.reviewers;
    next.my_review = facts.my_review;
    next.commits_since_review = facts.commits_since_review;
    let first_push_at = facts.first_push_at;
    next.requested = facts.requested;
    next.requested_at = facts.requested_at;
    next.read_at = stamp.clone();
    // A request the timeline did not show (past its last hundred) still happened: the time it
    // was first seen stands for it, and is kept once it is set.
    if next.requested_at.is_none() && next.requested {
        next.requested_at = Some(
            prev.as_ref()
                .and_then(|p| p.requested_at.clone())
                .unwrap_or_else(|| stamp.clone()),
        );
    }
    // After `requested_at` is final: a request newer than your last review is a re-request.
    next.rerequest = next.requested
        && next.my_review.as_ref().is_some_and(|m| {
            next.requested_at
                .as_deref()
                .is_some_and(|at| at > m.submitted_at.as_str())
        });

    let mut arrived = false;
    if next.requested {
        let new_request = |was_flag: fn(&Record) -> bool| {
            prev.as_ref()
                .is_none_or(|p| p.requested_at != next.requested_at || !was_flag(p))
        };
        if !next.rerequest && new_request(|p| p.requested) {
            next.events.requested = next.requested_at.clone();
            arrived = true;
        }
        if next.rerequest && new_request(|p| p.rerequest) {
            next.events.rerequested = next.requested_at.clone();
            arrived = true;
        }
    }
    // Every later push is news again, so the time moves with each head seen: the time of the
    // sync that saw it, as GitHub gives no push time that a rebase does not move. A push that was
    // already there when the record was first made (or never got a time, or only one from before
    // your latest review) takes the commit date of
    // the first commit after your review, else now; a head you reviewed has no push to show.
    if let Some(m) = &next.my_review {
        if next.head_sha == m.commit {
            next.events.pushed = None;
        } else if prev.as_ref().is_some_and(|p| p.head_sha != next.head_sha) {
            next.events.pushed = Some(stamp.clone());
        } else if next
            .events
            .pushed
            .as_deref()
            .is_none_or(|at| at <= m.submitted_at.as_str())
        {
            // A commit can be dated before the review it came after (committed, reviewed, then
            // pushed), so a date not after the review is no better than now.
            next.events.pushed = Some(
                first_push_at
                    .filter(|at| at.as_str() > m.submitted_at.as_str())
                    .unwrap_or_else(|| stamp.clone()),
            );
        }
    }

    next.done_at = (derive(&next).0 == State::Done).then(|| {
        prev.as_ref()
            .filter(|p| p.state == State::Done)
            .and_then(|p| p.done_at.clone())
            .unwrap_or_else(|| stamp.clone())
    });
    // Saved on every read: `read_at` moves, and this is one small file per PR per click.
    store::save(root, &mut next)?;
    Ok(arrived)
}

/// Remove the records that have been done for a day. Each is loaded again under its lock and
/// checked again: it may have been written, or read by a sync, since the listing.
fn remove_done(root: &Path, now: i64) -> Vec<String> {
    let cutoff = utc_stamp(now - DONE_KEPT_SECS);
    let expired = |r: &Record| {
        r.state == State::Done && r.done_at.as_deref().is_some_and(|at| at < cutoff.as_str())
    };
    let mut removed = Vec::new();
    for record in store::list(root).into_iter().filter(|r| expired(r)) {
        let Some(pr) = pr_ref(&record.url, None) else {
            continue;
        };
        let stem = store::stem(&pr);
        let Ok(_lock) = store::lock(root, &stem) else {
            continue;
        };
        let still = matches!(store::load(root, &pr), Ok(Some(r)) if expired(&r));
        if still && store::remove(root, &stem).is_ok() {
            removed.push(record.id);
        }
    }
    removed.sort();
    removed
}
