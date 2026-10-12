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

/// Bring the record of `pr` up to date with GitHub, or make it, as the PR of `task`, and answer it.
/// `repo` is the repository as GitHub spells it. The record stays one record when the PR is also
/// requested from you: it keeps its state, its read-through and its queue place, and takes the link.
/// A record made for another task is moved to this one.
///
/// `gh` and the time are handed in, like the sync's. GitHub is asked before anything is written, so
/// a PR that cannot be read leaves no record behind.
pub(crate) fn track_task_pr_with(
    root: &Path,
    pr: &PrRef,
    task: &TaskLink,
    repo: &str,
    now: i64,
    gh: Gh,
) -> Result<Record, String> {
    if pr.host != HOST {
        return Err(format!(
            "not a pull request on {HOST}: {}",
            store::id_of(pr)
        ));
    }
    // GitHub is asked outside any lock; the record is changed in one step under its own.
    let deadline = Instant::now() + TRACK_TIMEOUT;
    let me = github::viewer(gh, deadline)?;
    let facts = github::read_details(gh, &me, &[pr], deadline)
        .into_iter()
        .next()
        .ok_or_else(|| "GitHub gave no answer for the pull request".to_string())??;
    let files = files_of(root, gh, pr, &facts, deadline);
    let hit = Hit {
        pr: pr.clone(),
        repo: repo.to_string(),
        url: format!("https://{HOST}/{repo}/pull/{}", pr.number),
    };
    apply(root, &scope(root), now, &hit, facts, files, Some(task))?;
    store::load(root, pr)?.ok_or_else(|| format!("{} was not saved", store::id_of(pr)))
}
