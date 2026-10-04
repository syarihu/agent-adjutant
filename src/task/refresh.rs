//! Bringing task records up to date with their pull requests.

use super::*;

use crate::registry::Context;

/// How long a whole refresh may spend waiting on `gh`. The hub asks in the block it starts
/// with, and the MCP server answers one request at a time, so a `gh` that hangs — no network,
/// a login prompt — would hold up every other answer in that block with it. One deadline for
/// the lot rather than one per PR, so the wait does not grow with the number of records.
const GH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// One record `refresh` looked at, and what came of it.
pub struct Checked {
    pub task: Task,
    pub state: PrState,
    /// Whether this refresh moved the record to `done`. `false` for a merged PR whose record
    /// somebody else changed while `gh` was being asked.
    pub moved: bool,
    /// Why a merged PR's record could not be moved: it was removed while `gh` was being
    /// asked, or it could not be read back or written. One record that fails does not stop
    /// the rest, so the ones already moved are still reported as moved.
    pub failed: Option<String>,
}

/// The records a refresh looks at: every one with a `pr` that is not finished.
pub fn candidates(ctx: &Context) -> Vec<Task> {
    list(&ctx.state, &ctx.repo.slug)
        .into_iter()
        .filter(|t| !matches!(t.status, Status::Done | Status::Cancelled))
        .filter(|t| t.pr.is_some())
        .collect()
}

/// The pull request each record's `pr` names. A bare number is read against this repository,
/// on the host its origin is on: only when both came from the remote, since a directory name
/// is not a repository GitHub knows, and a number on a host this cannot name would be asked
/// of the wrong one. Origin is asked once, and only if some record holds a bare number.
pub fn pr_refs(ctx: &Context, tasks: &[Task]) -> Vec<Option<PrRef>> {
    let default = std::cell::OnceCell::new();
    tasks
        .iter()
        .map(|t| {
            let pr = t.pr.as_deref()?;
            let default = default.get_or_init(|| {
                (ctx.repo.nwo_source != "dirname")
                    .then(|| crate::kernel::identity::origin_host(&ctx.repo.main))
                    .flatten()
            });
            pr_ref(
                pr,
                default.as_deref().map(|host| (host, ctx.repo.nwo.as_str())),
            )
        })
        .collect()
}

/// Why `pr` could not be read as a pull request, for a record whose `pr_refs` entry is `None`.
pub(super) fn unreadable_pr(pr: &str) -> String {
    if pr_ref(pr, Some(("github.com", "o/r"))).is_some() {
        format!(
            "a bare pull request number needs a remote on a known host to be read against: {pr}"
        )
    } else {
        format!("not a pull request this can read: {pr}")
    }
}

/// Bring the records up to date with their pull requests: every one that has a `pr` and is
/// not finished is asked about, all in one query, and the ones whose PR was merged are moved
/// to `done`.
///
/// Nothing else is changed. A PR still open is still in review; one closed without merging
/// may have been replaced by another, which only a person knows; one `gh` cannot read is
/// not evidence of anything. Those are returned for the caller to report.
pub fn refresh(ctx: &Context) -> Result<Vec<Checked>, String> {
    let candidates = candidates(ctx);
    let deadline = std::time::Instant::now() + GH_TIMEOUT;
    let refs = pr_refs(ctx, &candidates);
    let readable: Vec<PrRef> = refs.iter().flatten().cloned().collect();
    let mut read = read_prs(&readable, deadline).answers.into_iter();
    let answers = candidates
        .iter()
        .zip(&refs)
        .map(|(t, r)| match r {
            Some(_) => read
                .next()
                .unwrap_or_else(|| (PrState::Unreadable("not read".to_string()), None)),
            None => (
                PrState::Unreadable(unreadable_pr(t.pr.as_deref().unwrap_or_default())),
                None,
            ),
        })
        .collect();
    Ok(apply(ctx, candidates, answers))
}

/// Act on what GitHub said about `candidates`, one answer each: a merged PR moves its record
/// to `done`, and any other answer is kept on the record when it differs from what is stored.
/// A PR closed without merging is never cancelled here: the work may have gone on elsewhere.
pub fn apply(
    ctx: &Context,
    candidates: Vec<Task>,
    answers: Vec<(PrState, Option<PrStatus>)>,
) -> Vec<Checked> {
    let mut checked = Vec::new();
    for (task, (state, summary)) in candidates.into_iter().zip(answers) {
        if state != PrState::Merged {
            // Kept for the board, and nothing else about the record changes. Written only
            // when GitHub's answer differs from what is stored, so a quiet PR costs no write
            // and the page, which redraws when the JSON changes, stays still.
            let task = match summary.filter(|s| task.pr_status.as_ref() != Some(s)) {
                Some(summary) => store_pr_status(ctx, task, summary),
                None => task,
            };
            checked.push(Checked {
                task,
                state,
                moved: false,
                failed: None,
            });
            continue;
        }
        // Read again under the lock: `gh` took a while, and a record somebody moved or
        // pointed at another PR in the meantime is theirs, not this answer's.
        let move_it = || -> Result<(Task, bool), String> {
            let _lock = store::lock(ctx, &task.id)?;
            let mut now = get(&ctx.state, &ctx.repo.slug, &task.id)?;
            let moved = now.pr == task.pr && now.status == task.status;
            if moved {
                now.status = Status::Done;
                now.pr_status = summary.clone().or(now.pr_status);
                now.updated_at = store::stamp();
                store::save(ctx, &now)?;
            }
            Ok((now, moved))
        };
        checked.push(match move_it() {
            Ok((now, moved)) => Checked {
                task: now,
                state,
                moved,
                failed: None,
            },
            Err(why) => Checked {
                task,
                state,
                moved: false,
                failed: Some(why),
            },
        });
    }
    checked
}

/// Keep what GitHub said about `task`'s PR on its record, if the record still points at that
/// PR once the lock is held. The summary is a cache, so a record that cannot be written is
/// returned as it was rather than failing the refresh. `updatedAt` stays: nobody changed the
/// task, and the board reads that stamp as the last time somebody did.
fn store_pr_status(ctx: &Context, task: Task, summary: PrStatus) -> Task {
    let write = || -> Result<Option<Task>, String> {
        let _lock = store::lock(ctx, &task.id)?;
        let mut now = get(&ctx.state, &ctx.repo.slug, &task.id)?;
        if now.pr != task.pr || now.pr_status.as_ref() == Some(&summary) {
            return Ok(None);
        }
        now.pr_status = Some(summary);
        store::save(ctx, &now)?;
        Ok(Some(now))
    };
    match write() {
        Ok(Some(now)) => now,
        Ok(None) => task,
        Err(why) => {
            eprintln!("could not keep the PR summary of {}: {why}", task.id);
            task
        }
    }
}
