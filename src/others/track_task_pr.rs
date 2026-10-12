//! Reading a task's PR into its record, so the read-through of it can be asked for like a request's.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::task::PrRef;

use super::github::{self, Gh, Hit};
use super::model::{Record, TaskLink};
use super::scope::scope;
use super::store;
use super::sync::{apply, files_of};

/// How long reading the one PR may take: the click that asks for it is waited on.
const TRACK_TIMEOUT: Duration = Duration::from_secs(30);

/// The host a record can be about: the same one the sync searches.
const HOST: &str = "github.com";

/// Why a task's PR was not read into its record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackError {
    /// A sync is running. It writes this record too, from what GitHub said a moment ago, so the
    /// two are not interleaved: nothing was read or written.
    Syncing,
    /// GitHub or the record could not be read or written, and why.
    Failed(String),
}

/// Bring the record of `pr` up to date with GitHub, or make it, as the PR of `task`, and answer it.
/// `repo` is the repository as GitHub spells it. The record stays one record when the PR is also
/// requested from you: it keeps its state, its read-through and its queue place, and takes the link.
/// A record made for another task is moved to this one.
///
/// `gh` and the time are handed in, like the sync's. GitHub is asked before anything is written, so
/// a PR that cannot be read leaves no record behind. The sync's lock is held from the read to the
/// save, so an older answer cannot overwrite what a sync has saved since (a merged PR made open
/// again); the record's own lock is taken only inside `apply`, after GitHub has answered. Nothing
/// the sync runs calls this.
pub(crate) fn track_task_pr_with(
    root: &Path,
    pr: &PrRef,
    task: &TaskLink,
    repo: &str,
    now: i64,
    gh: Gh,
) -> Result<Record, TrackError> {
    if pr.host != HOST {
        return Err(TrackError::Failed(format!(
            "not a pull request on {HOST}: {}",
            store::id_of(pr)
        )));
    }
    let _sync = match store::try_lock_sync(root) {
        Ok(Some(lock)) => lock,
        Ok(None) => return Err(TrackError::Syncing),
        Err(e) => return Err(TrackError::Failed(e)),
    };
    // GitHub is asked outside the record's lock; the record is changed in one step under it.
    let deadline = Instant::now() + TRACK_TIMEOUT;
    let me = github::viewer(gh, deadline).map_err(TrackError::Failed)?;
    let facts = github::read_details(gh, &me, &[pr], deadline)
        .into_iter()
        .next()
        .ok_or_else(|| {
            TrackError::Failed("GitHub gave no answer for the pull request".to_string())
        })?
        .map_err(TrackError::Failed)?;
    let files = files_of(root, gh, pr, &facts, deadline);
    let hit = Hit {
        pr: pr.clone(),
        repo: repo.to_string(),
        url: format!("https://{HOST}/{repo}/pull/{}", pr.number),
    };
    apply(root, &scope(root), now, &hit, facts, files, Some(task)).map_err(TrackError::Failed)?;
    store::load(root, pr)
        .map_err(TrackError::Failed)?
        .ok_or_else(|| TrackError::Failed(format!("{} was not saved", store::id_of(pr))))
}
