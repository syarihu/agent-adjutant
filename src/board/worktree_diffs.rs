use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use crate::kernel::worktree_state::{Uncommitted, uncommitted_of};

/// How soon the diff of a worktree whose session is on the board is read again.
pub const DIFF_REREAD_LISTED: Duration = Duration::from_secs(10);

/// How soon one whose session is not running is: nobody is editing it, so it is looked at
/// less often.
pub const DIFF_REREAD_IDLE: Duration = Duration::from_secs(60);

/// How long one of the git calls of a read may take. A worktree on a slow disk holds up the
/// reads behind it for no longer than this.
const DIFF_GIT_SECS: u64 = 3;

/// What was last read of one worktree.
#[derive(Default)]
struct Diff {
    /// Whether its session runs, as the last poll that listed it said.
    present: bool,
    read_at: Option<Instant>,
    /// The last good count, kept through a read that failed.
    counts: Option<Uncommitted>,
    error: Option<String>,
}

#[derive(Default)]
struct Inner {
    by_path: HashMap<String, Diff>,
    /// A thread is reading the due worktrees one after another.
    reading: bool,
}

/// The uncommitted files and lines of the worktrees a page lists, by worktree path.
///
/// A poll asks for a worktree (`want`) and takes what is there (`look`); the reading is done by
/// one thread of its own (`refresh`), never on the request's, so that a slow disk makes the
/// numbers late and not the page. A cache: the worktree is the answer.
#[derive(Default)]
pub struct WorktreeDiffs {
    inner: Mutex<Inner>,
}

/// The worktrees of `diffs` that are due again at `now`, whose session is running (`present`)
/// or not.
fn due(diffs: &HashMap<String, Diff>, now: Instant) -> Vec<String> {
    let mut paths: Vec<String> = diffs
        .iter()
        .filter(|(_, diff)| {
            let every = match diff.present {
                true => DIFF_REREAD_LISTED,
                false => DIFF_REREAD_IDLE,
            };
            diff.read_at
                .is_none_or(|at| now.saturating_duration_since(at) >= every)
        })
        .map(|(path, _)| path.clone())
        .collect();
    paths.sort();
    paths
}

impl WorktreeDiffs {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// What was last read of `path`: its counts, none before the first read, and the error of
    /// the last read when it failed, the counts of an earlier one staying beside it.
    pub fn look(&self, path: &str) -> (Option<Uncommitted>, Option<String>) {
        match self.lock().by_path.get(path) {
            Some(diff) => (diff.counts.clone(), diff.error.clone()),
            None => (None, None),
        }
    }

    /// Asks for `path` to be read, and again as it falls due while it is asked for.
    pub fn want(&self, path: &str, present: bool) {
        self.lock()
            .by_path
            .entry(path.to_string())
            .or_default()
            .present = present;
    }

    /// What a read of `path` at `at` found: a count, nothing for a directory that is gone, or
    /// why it could not tell. A failure leaves the last count in place. A path nobody asks for
    /// any more is not brought back.
    pub fn store(&self, path: &str, at: Instant, result: Result<Option<Uncommitted>, String>) {
        let mut inner = self.lock();
        let Some(diff) = inner.by_path.get_mut(path) else {
            return;
        };
        diff.read_at = Some(at);
        match result {
            Ok(counts) => {
                diff.counts = counts;
                diff.error = None;
            }
            Err(error) => diff.error = Some(error),
        }
    }

    /// Forgets the worktrees that are no longer listed.
    pub fn keep_only(&self, listed: &HashSet<String>) {
        self.lock().by_path.retain(|path, _| listed.contains(path));
    }

    /// Starts reading what is due at `now`, unless a read is already out. The thread reads the
    /// worktrees one after another and stops when the server that owns this is gone.
    pub fn refresh(self: &Arc<Self>, now: Instant) {
        let paths = {
            let mut inner = self.lock();
            if inner.reading {
                return;
            }
            let paths = due(&inner.by_path, now);
            if paths.is_empty() {
                return;
            }
            inner.reading = true;
            paths
        };
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || read_all(&weak, &paths));
    }
}

/// Lets the next read start when the thread ends, however it ends.
struct Reading(Weak<WorktreeDiffs>);

impl Drop for Reading {
    fn drop(&mut self) {
        if let Some(diffs) = self.0.upgrade() {
            diffs.lock().reading = false;
        }
    }
}

fn read_all(weak: &Weak<WorktreeDiffs>, paths: &[String]) {
    let _reading = Reading(weak.clone());
    for path in paths {
        // The server that owns the cache going away ends the reading: no git call is made for
        // a worktree nobody can see the answer for.
        if weak.strong_count() == 0 {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(DIFF_GIT_SECS);
        let result = uncommitted_of(Path::new(path), deadline);
        match weak.upgrade() {
            Some(diffs) => diffs.store(path, Instant::now(), result),
            None => return,
        }
    }
}

#[cfg(test)]
mod tests;
