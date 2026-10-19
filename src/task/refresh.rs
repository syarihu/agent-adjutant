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

/// How long moving a task to `done` may wait on `gh` to read its PR once. Short: a person is
/// waiting on the move, and a PR that cannot be read in time is simply left as recorded.
const FINISH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The records a refresh acts on: every one with a `pr` that is not finished. Finished ones
/// are not asked about for a move (see `stale_finished` for the ones read only to be corrected).
pub fn candidates(ctx: &Context) -> Vec<Task> {
    list(&ctx.state, &ctx.repo.slug)
        .into_iter()
        .filter(|t| !matches!(t.status, Status::Done | Status::Cancelled))
        .filter(|t| t.pr.is_some())
        .collect()
}

/// The done records whose kept PR state may be out of date: a `pr`, and no summary or one that
/// is neither `merged` nor `closed`. A task moved to done by hand after its PR merged keeps
/// the state it was last read with, which `candidates` never corrects.
///
/// Cancelled ones are left out: a cancelled task stays cancelled whatever its PR did, so its
/// state is nobody's concern. So are records already `merged` or `closed`: those states
/// cannot change, which also keeps the cost bounded, since every record is asked about until
/// it reaches one of them and never after.
pub fn stale_finished(ctx: &Context) -> Vec<Task> {
    list(&ctx.state, &ctx.repo.slug)
        .into_iter()
        .filter(|t| t.status == Status::Done && t.pr.is_some())
        .filter(|t| !pr_state_is_final(t))
        .collect()
}

/// Whether the PR state kept on `task` is one that cannot change any more.
fn pr_state_is_final(task: &Task) -> bool {
    task.pr_status
        .as_ref()
        .is_some_and(|s| matches!(s.state.as_str(), "merged" | "closed"))
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
/// not finished is asked about, and the ones whose PR was merged are moved to `done`. Done
/// records whose kept PR state is not yet final are asked about in the same query, only to
/// correct that state (`record_finished`): they are never moved and not returned.
///
/// Nothing else is changed. A PR still open is still in review; one closed without merging
/// may have been replaced by another, which only a person knows; one `gh` cannot read is
/// not evidence of anything. Those are returned for the caller to report.
pub fn refresh(ctx: &Context) -> Result<Vec<Checked>, String> {
    let live = candidates(ctx);
    let stale = stale_finished(ctx);
    let deadline = std::time::Instant::now() + GH_TIMEOUT;
    // One query for both, the records still in play first: they are what the shared deadline
    // protects.
    let all: Vec<Task> = live.iter().chain(&stale).cloned().collect();
    let refs = pr_refs(ctx, &all);
    let readable: Vec<PrRef> = refs.iter().flatten().cloned().collect();
    let mut read = read_prs(&readable, deadline).answers.into_iter();
    let mut answers: Vec<(PrState, Option<PrStatus>)> = all
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
    let stale_answers = answers.split_off(live.len());
    record_finished(ctx, stale, stale_answers);
    Ok(apply(ctx, live, answers))
}

/// Keep what GitHub said on done records whose PR state was out of date, one answer each. The
/// record is only corrected: its status, park and `updatedAt` stay (it is not `apply`, which
/// would stamp the record as just finished), and the turn is not stamped, as nobody saw the
/// change happen. An answer that could not be read leaves the record for the next refresh.
pub(super) fn record_finished(
    ctx: &Context,
    tasks: Vec<Task>,
    answers: Vec<(PrState, Option<PrStatus>)>,
) {
    for (task, (_, summary)) in tasks.into_iter().zip(answers) {
        if let Some(summary) = summary.filter(|s| task.pr_status.as_ref() != Some(s)) {
            store_pr_status(ctx, task, summary, Seen::Finished);
        }
    }
}

/// Read the PR of a task that has just been moved to `done`, once, and keep what GitHub said
/// on the record, so a card moved after its PR merged does not keep `open`. Not stamping the
/// turn: the record never saw the change happen. Never fails the move: a PR `gh` cannot read in
/// time leaves the record as it was, for a later refresh to correct. Call it without the
/// task's lock held, since `gh` is slow.
pub(super) fn read_on_finish(ctx: &Context, task: Task) -> Task {
    let Some(pr) = pr_refs(ctx, std::slice::from_ref(&task))
        .into_iter()
        .flatten()
        .next()
    else {
        return task;
    };
    let deadline = std::time::Instant::now() + FINISH_TIMEOUT;
    match read_prs(&[pr], deadline).answers.into_iter().next() {
        Some((_, Some(summary))) if task.pr_status.as_ref() != Some(&summary) => {
            store_pr_status(ctx, task, summary, Seen::Finished)
        }
        _ => task,
    }
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
                Some(summary) => store_pr_status(ctx, task, summary, Seen::Now),
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
                now.parked = None;
                note_turn(&mut now, summary.as_ref());
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

/// How a PR summary reached a record.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Seen {
    /// Read while the record was in play: a changed turn is stamped on `pr_turn_at`.
    Now,
    /// Read of a record already `done`, which never saw the change happen: no stamp, and
    /// nothing is written unless the record is still `done` with the summary it had when `gh`
    /// was asked, once the lock is held (it may have been cancelled or reopened meanwhile, or
    /// another read may have stored a newer answer, which a slow one must not undo).
    Finished,
}

/// Keep what GitHub said about `task`'s PR on its record, if the record still points at that
/// PR once the lock is held. The summary is a cache, so a record that cannot be written is
/// returned as it was rather than failing the refresh. `updatedAt` stays: nobody changed the
/// task, and the board reads that stamp as the last time somebody did. `seen` says
/// how the answer was come by (see `Seen`).
pub(super) fn store_pr_status(ctx: &Context, task: Task, summary: PrStatus, seen: Seen) -> Task {
    let write = || -> Result<Option<Task>, String> {
        let _lock = store::lock(ctx, &task.id)?;
        let mut now = get(&ctx.state, &ctx.repo.slug, &task.id)?;
        if now.pr != task.pr || now.pr_status.as_ref() == Some(&summary) {
            return Ok(None);
        }
        if seen == Seen::Finished && (now.status != Status::Done || now.pr_status != task.pr_status)
        {
            return Ok(None);
        }
        if seen == Seen::Now {
            note_turn(&mut now, Some(&summary));
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

/// Stamp `pr_turn_at` when `next` stands for a different turn than the record's summary does.
/// Called before the summary is replaced, under the record's lock.
fn note_turn(task: &mut Task, next: Option<&PrStatus>) {
    let Some(next) = next else {
        return;
    };
    if pr_turn(next) != task.pr_status.as_ref().and_then(pr_turn) {
        task.pr_turn_at = Some(store::stamp());
    }
}
