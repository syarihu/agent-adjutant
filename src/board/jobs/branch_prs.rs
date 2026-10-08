//! The pull request of the branch each session with no task works on, kept in memory.
//!
//! A task's pull request is on its card, written by the poll's own reads. A session with no task
//! has no card, so what GitHub says of its branch is asked of it here, in the poll's thread and
//! round, and kept only in memory: nothing is written to a record, and `/api/state` only reads
//! what is held.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::board::SessionPr;
use crate::registry::Context;
use crate::task::{self, BranchPr, BranchRef};

/// How often the pull request of a branch is asked about again when no notification names its
/// repository: GitHub does not notify the person who does a thing.
pub(super) const BRANCH_REREAD: Duration = Duration::from_secs(5 * 60);

/// A repository as GitHub compares it (lower case) and a branch, which is case-sensitive.
type Key = (String, String);

fn key_of(r: &BranchRef) -> Key {
    (format!("{}/{}", r.owner, r.repo), r.branch.clone())
}

/// What was last found of one branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub read_at: Instant,
    /// The answer, kept through a read that failed.
    pub pr: Option<SessionPr>,
    /// What the last read said when it failed, so that it is said once per change.
    pub error: Option<String>,
}

/// The branches asked of, which of them are due, and what a round found.
#[derive(Default)]
pub struct BranchPrs {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<Key, Entry>,
    /// Why the last round could not ask at all, said once per change.
    round_error: Option<String>,
}

/// The branches of `wanted` that are due at `now`: never read, read `BRANCH_REREAD` ago or
/// longer, or in a repository `news_repos` names. Positions in `wanted`.
pub(super) fn due(
    entries: &HashMap<Key, Entry>,
    wanted: &[BranchRef],
    now: Instant,
    news_repos: &HashSet<String>,
) -> Vec<usize> {
    (0..wanted.len())
        .filter(|&i| {
            let key = key_of(&wanted[i]);
            match entries.get(&key) {
                None => true,
                Some(entry) => {
                    now.saturating_duration_since(entry.read_at) >= BRANCH_REREAD
                        || news_repos.contains(&key.0)
                }
            }
        })
        .collect()
}

/// What `read` of `asked` (the branches `due` named) did to `entries`: a found answer replaces
/// the old one, and a branch that could not be read keeps it. A branch that was asked of stays
/// out of `due` for a round either way. The lines to say on stderr: one for each change of why
/// a branch cannot be read, and none for a round that failed as a whole — that is `round_error`
/// to say.
pub(super) fn apply(
    entries: &mut HashMap<Key, Entry>,
    asked: &[BranchRef],
    read: task::BranchPrsRead,
    now: Instant,
) -> Vec<String> {
    let mut said = Vec::new();
    for (r, answer) in asked.iter().zip(read.answers) {
        let key = key_of(r);
        let before = entries.get(&key).cloned();
        let entry = match answer {
            Ok(pr) => Entry {
                read_at: now,
                pr: pr.map(session_pr),
                error: None,
            },
            Err(why) => {
                if read.failed.is_none()
                    && before.as_ref().and_then(|b| b.error.as_ref()) != Some(&why)
                {
                    said.push(format!(
                        "adj server: cannot read the pull request of {} in {}: {why}",
                        r.branch, key.0
                    ));
                }
                Entry {
                    read_at: now,
                    pr: before.and_then(|b| b.pr),
                    error: Some(why),
                }
            }
        };
        entries.insert(key, entry);
    }
    said
}

fn session_pr(pr: BranchPr) -> SessionPr {
    SessionPr {
        number: pr.number,
        url: pr.url,
        state: pr.state,
    }
}

/// The branches worth asking about on `ctx`'s board, if its origin is on `HOST`: a linked
/// worktree on a branch, with no task, and not the branch the main checkout is on.
pub(super) fn branches_of(ctx: &Context, host: &str) -> Vec<BranchRef> {
    let repo = &ctx.repo;
    if repo.nwo_source == "dirname"
        || crate::kernel::identity::origin_host(&repo.main).as_deref() != Some(host)
    {
        return Vec::new();
    }
    let Some((owner, name)) = repo.nwo.split_once('/') else {
        return Vec::new();
    };
    // A worktree list that cannot be read asks about nothing: the answers held stay as they are.
    let Ok(listed) = crate::kernel::identity::worktrees(&repo.main) else {
        return Vec::new();
    };
    let main = Path::new(&repo.main);
    let main_branch = listed
        .iter()
        .find(|w| Path::new(&w.path) == main)
        .and_then(|w| w.branch.clone());
    let mut out: Vec<BranchRef> = Vec::new();
    for w in listed.iter().filter(|w| Path::new(&w.path) != main) {
        let Some(branch) = w.branch.clone().filter(|b| Some(b) != main_branch.as_ref()) else {
            continue;
        };
        if crate::registry::worker_task(Path::new(&w.path)).is_some() {
            continue;
        }
        let r = BranchRef {
            host: host.to_string(),
            owner: owner.to_ascii_lowercase(),
            repo: name.to_ascii_lowercase(),
            branch,
        };
        if !out.contains(&r) {
            out.push(r);
        }
    }
    out
}

impl BranchPrs {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The pull request last found for `branch` of the repository `nwo`, if there is one.
    pub fn look(&self, nwo: &str, branch: &str) -> Option<SessionPr> {
        self.lock()
            .entries
            .get(&(nwo.to_ascii_lowercase(), branch.to_string()))?
            .pr
            .clone()
    }

    /// One round: ask about the branches of `boards` that are due, and forget those that are
    /// gone. `news_repos` are the repositories this round's notifications name, or `all_named`
    /// when there may be news for any.
    pub(super) fn round(
        &self,
        boards: &[Context],
        host: &str,
        news_repos: &HashSet<String>,
        all_named: bool,
    ) {
        let mut wanted: Vec<BranchRef> = Vec::new();
        for ctx in boards {
            for r in branches_of(ctx, host) {
                if !wanted.contains(&r) {
                    wanted.push(r);
                }
            }
        }
        let all: HashSet<String>;
        let news_repos = match all_named {
            true => {
                all = wanted.iter().map(|r| key_of(r).0).collect();
                &all
            }
            false => news_repos,
        };
        let now = Instant::now();
        let asked: Vec<BranchRef> = {
            let mut inner = self.lock();
            let keep: HashSet<Key> = wanted.iter().map(key_of).collect();
            inner.entries.retain(|key, _| keep.contains(key));
            due(&inner.entries, &wanted, now, news_repos)
                .into_iter()
                .map(|i| wanted[i].clone())
                .collect()
        };
        if asked.is_empty() {
            return;
        }
        // Not under the lock: reading runs `gh`.
        let read = task::find_branch_prs(&asked, Instant::now() + super::pr_poll::ROUND_TIMEOUT);
        let failed = read.failed.clone();
        let mut inner = self.lock();
        let said = apply(&mut inner.entries, &asked, read, Instant::now());
        let round_said = (failed != inner.round_error)
            .then(|| {
                failed.as_ref().map(|why| {
                    format!("adj server: cannot read the pull requests of the branches: {why}")
                })
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
