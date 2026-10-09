//! The parent issue GitHub says each task's issue has, kept in memory.
//!
//! A task's record may name its parent (`parent`), but the tracker has its own word on it: the
//! issue's parent when it is a sub-issue. The board prefers the tracker's, so it is asked here, in
//! the poll's thread and round, and kept only in memory: nothing is written to a record, and
//! `/api/state` only reads what is held.
//!
//! Three things can be known of an issue, and they are not the same: never read (the record is
//! used), read and found to have no parent (the record is used), and read before and failing now
//! (the last answer is kept).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::registry::Context;
use crate::task::{self, IssueRef, TrackerParent};

/// How often the parent of an issue is asked about again. Sub-issues are changed by a person on
/// GitHub, which sends the board nothing, so there is no news to wait for.
pub(super) const PARENT_REREAD: Duration = Duration::from_secs(10 * 60);

/// How soon an issue whose reads have never succeeded, the last for a failure of the round as a
/// whole, is asked about again: nothing is known of it, so waiting out the long interval would keep
/// the record's word for ten minutes on a hiccup. An issue GitHub itself refuses waits the long one.
pub(super) const PARENT_RETRY: Duration = Duration::from_secs(2 * 60);

/// What was last found of one issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub read_at: Instant,
    /// The answer, kept through a read that failed.
    pub parent: Option<TrackerParent>,
    /// What the last read said when it failed, so that it is said once per change.
    pub error: Option<String>,
    /// Whether any read of this issue has succeeded.
    pub ever_read: bool,
    /// Whether the last read failed as a whole (the network, a login, a rate limit), as against
    /// one issue GitHub would not give: that one will not change by asking soon again.
    pub round_failed: bool,
}

/// An issue a task names, and whether every task that names it is finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Wanted {
    pub issue: IssueRef,
    pub finished: bool,
}

/// The issues asked of, which of them are due, and what a round found.
#[derive(Default)]
pub struct IssueParents {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<IssueRef, Entry>,
    /// Why the last round could not ask at all, said once per change.
    round_error: Option<String>,
}

/// The issues of `wanted` that are due at `now`: never read, or read `PARENT_REREAD` ago or
/// longer (`PARENT_RETRY` while no read has succeeded and the last one failed as a whole). An issue only finished tasks name is
/// read once while the server lives, since nobody sub-issues a finished task's issue under
/// another and then cares; one whose read failed is asked again like any other. Positions in
/// `wanted`.
pub(super) fn due(
    entries: &HashMap<IssueRef, Entry>,
    wanted: &[Wanted],
    now: Instant,
) -> Vec<usize> {
    (0..wanted.len())
        .filter(|&i| match entries.get(&wanted[i].issue) {
            None => true,
            Some(entry) => {
                let age = now.saturating_duration_since(entry.read_at);
                let soon = !entry.ever_read && entry.round_failed;
                let old = age >= if soon { PARENT_RETRY } else { PARENT_REREAD };
                old && (!wanted[i].finished || entry.error.is_some())
            }
        })
        .collect()
}

/// What `read` of `asked` (the issues `due` named) did to `entries`: a found answer, a parent or
/// none, replaces the old one, and an issue that could not be read keeps it. An issue that was
/// asked of stays out of `due` for a while either way. The lines to say on stderr: one for each
/// change of why an issue cannot be read, and none for a round that failed as a whole, which is
/// `round_error` to say.
pub(super) fn apply(
    entries: &mut HashMap<IssueRef, Entry>,
    asked: &[IssueRef],
    read: task::IssueParentsRead,
    now: Instant,
) -> Vec<String> {
    let mut said = Vec::new();
    for (issue, answer) in asked.iter().zip(read.answers) {
        let before = entries.get(issue).cloned();
        let entry = match answer {
            Ok(parent) => Entry {
                read_at: now,
                parent,
                error: None,
                ever_read: true,
                round_failed: false,
            },
            Err(why) => {
                if read.failed.is_none()
                    && before.as_ref().and_then(|b| b.error.as_ref()) != Some(&why)
                {
                    said.push(format!(
                        "adj server: cannot read the parent of {}/{}#{}: {why}",
                        issue.owner, issue.repo, issue.number
                    ));
                }
                Entry {
                    read_at: now,
                    ever_read: before.as_ref().is_some_and(|b| b.ever_read),
                    round_failed: read.failed.is_some(),
                    parent: before.and_then(|b| b.parent),
                    error: Some(why),
                }
            }
        };
        entries.insert(issue.clone(), entry);
    }
    said
}

/// The issues the tasks of `ctx`'s board name on `host`.
pub(super) fn wanted_of(ctx: &Context, host: &str) -> Vec<Wanted> {
    let mut out: Vec<Wanted> = Vec::new();
    for t in task::list(&ctx.state, &ctx.repo.slug) {
        let Some(issue) = task::issue_to_fetch(&t)
            .and_then(task::issue_ref_of)
            .filter(|r| r.host == host)
        else {
            continue;
        };
        let finished = matches!(t.status, task::Status::Done | task::Status::Cancelled);
        match out.iter_mut().find(|w| w.issue == issue) {
            Some(w) => w.finished &= finished,
            None => out.push(Wanted { issue, finished }),
        }
    }
    out
}

impl IssueParents {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The parent the tracker last said the issue at `url` has. `None` when it was never read,
    /// when it has none, and when `url` is not an issue; the board uses the record then. A read
    /// that failed leaves the answer before it.
    pub fn look(&self, url: &str) -> Option<TrackerParent> {
        let issue = task::issue_ref_of(url)?;
        self.lock()
            .entries
            .get(&issue)
            .and_then(|entry| entry.parent.clone())
    }

    /// One round: ask about the issues of `boards` that are due, and forget those no task names.
    pub(super) fn round(&self, boards: &[Context], host: &str) {
        let mut wanted: Vec<Wanted> = Vec::new();
        for ctx in boards {
            for w in wanted_of(ctx, host) {
                match wanted.iter_mut().find(|x| x.issue == w.issue) {
                    Some(x) => x.finished &= w.finished,
                    None => wanted.push(w),
                }
            }
        }
        let now = Instant::now();
        let asked: Vec<IssueRef> = {
            let mut inner = self.lock();
            let keep: HashSet<&IssueRef> = wanted.iter().map(|w| &w.issue).collect();
            inner.entries.retain(|issue, _| keep.contains(issue));
            due(&inner.entries, &wanted, now)
                .into_iter()
                .map(|i| wanted[i].issue.clone())
                .collect()
        };
        if asked.is_empty() {
            return;
        }
        // Not under the lock: reading runs `gh`.
        let read = task::read_issue_parents(&asked, Instant::now() + super::pr_poll::ROUND_TIMEOUT);
        let failed = read.failed.clone();
        let mut inner = self.lock();
        let said = apply(&mut inner.entries, &asked, read, Instant::now());
        let round_said = (failed != inner.round_error)
            .then(|| {
                failed
                    .as_ref()
                    .map(|why| format!("adj server: cannot read the parents of the issues: {why}"))
            })
            .flatten();
        inner.round_error = failed;
        drop(inner);
        // Not `eprintln!`, which panics when stderr is closed: the poll's thread has to
        // outlive whatever the server's log is doing.
        use std::io::Write;
        for line in said.into_iter().chain(round_said) {
            let _ = writeln!(std::io::stderr(), "{line}");
        }
    }
}

#[cfg(test)]
mod tests;
