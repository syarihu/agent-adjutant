use std::path::Path;
use std::sync::atomic::Ordering;

use crate::board::view::find_session;
use crate::board::{NEXT, Server, settings_now};
use crate::infra::paths::same_path;
use crate::kernel::config::Settings;
use crate::kernel::worktree_state::GitState;
use crate::mail;
use crate::registry::{self, Context};
use crate::task::{self, Executor, Status};

// ── cleanup ──────────────────────────────────────────────────────────

/// What the person asked for when removing a session's worktree.
pub struct CleanUpRequest {
    /// Remove although there is work that would be lost.
    pub force: bool,
    /// The worktree's name typed back; a forced removal needs it.
    pub confirm: Option<String>,
}

/// How a clean-up came out when nothing refused it outright.
pub enum CleanUp {
    /// Nothing was removed because there is work that would be lost.
    Kept {
        reasons: Vec<Loss>,
        git: Option<GitState>,
        /// Whether the session was running and has been closed; `None` on the first look, which
        /// closes nothing.
        closed: Option<bool>,
    },
    Removed(Removed),
}

/// What a removal did.
pub struct Removed {
    pub forced: bool,
    /// Whether the session was running and has been closed.
    pub closed: bool,
    /// `None` when no branch is known for the session.
    pub branch: Option<BranchRemoval>,
    /// The tasks finished with the worktree.
    pub tasks: Vec<String>,
    /// The tasks that could not be finished, as (id, error).
    pub task_errors: Vec<(String, String)>,
    pub hooks: Vec<HookRun>,
}

/// The local branch's removal: it can fail without undoing the worktree's.
pub struct BranchRemoval {
    pub name: String,
    pub deleted: Result<(), String>,
}

pub use crate::kernel::worktree_hooks::HookRun;

/// Work that removing the worktree would lose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loss {
    /// A git that could not answer: not knowing is not the same as nothing to lose.
    Git(String),
    Uncommitted {
        files: usize,
        insertions: usize,
        deletions: usize,
    },
    Untracked(usize),
    Unpushed(usize),
}

impl Loss {
    pub fn describe(&self) -> String {
        match self {
            Loss::Git(e) => e.clone(),
            Loss::Uncommitted {
                files,
                insertions,
                deletions,
            } => format!("{files} changed file(s), +{insertions} -{deletions} lines"),
            Loss::Untracked(n) => format!("{n} untracked path(s)"),
            Loss::Unpushed(n) => format!("{n} commit(s) no remote has"),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Loss::Git(_) => "git",
            Loss::Uncommitted { .. } => "uncommitted",
            Loss::Untracked(_) => "untracked",
            Loss::Unpushed(_) => "unpushed",
        }
    }
}

impl serde::Serialize for Loss {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("Loss", 2)?;
        s.serialize_field("kind", self.kind())?;
        s.serialize_field("detail", &self.describe())?;
        s.end()
    }
}

/// How branch deletion and task completion are handled upon removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanUpPolicy {
    /// The board's policy: forced removal allowed, branch deleted always, dispatched and pr tasks set done.
    Board,
    /// `adj worktree rm` policy: never forced, branch deleted only if merged, pr tasks left in pr if not merged.
    WorktreeRm,
}

/// How a clean-up execution came out when nothing refused it outright.
pub enum CleanUpExecution {
    Kept {
        reasons: Vec<Loss>,
        git: Option<GitState>,
        closed: Option<bool>,
    },
    Removed(CleanUpRemoved),
}

/// What a clean-up execution did.
pub struct CleanUpRemoved {
    pub path: String,
    pub forced: bool,
    pub closed: bool,
    pub branch: Option<CleanUpBranch>,
    pub tasks: Vec<String>,
    pub kept_tasks: Vec<String>,
    pub task_errors: Vec<(String, String)>,
    pub hooks: Vec<HookRun>,
}

/// Branch removal or retention outcome.
pub struct CleanUpBranch {
    pub name: String,
    pub deleted: Result<(), String>,
    pub kept: Option<String>,
}

/// Remove the worktree of the worker session `id` and its local branch, closing the session
/// first if it runs.
///
/// The hub's procedure says it is the only route by which anything is removed; from here the
/// board is a second one, and makes the same checks itself rather than asking the hub, which
/// may not be running. Refusals that cannot be forced (the ground under a worker still to
/// come) are errors. What would be lost is `Kept` with the reasons, not an error: the reasons
/// are what the person needs to decide whether to force. `Removed` says what was done.
pub fn clean_up(server: &Server, id: &str, request: CleanUpRequest) -> Result<CleanUp, String> {
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    if session.kind != "worker" {
        return Err("only a worker session has a worktree to remove".to_string());
    }
    let base = session.task.as_deref().and_then(|task_id| {
        let slug =
            crate::board::view::worker_hub_slug(&server.ctx.repo, Path::new(&session.worktree));
        task::get(&server.ctx.state, &slug, task_id).ok()?.base
    });
    let outcome = clean_up_worktree(
        &server.ctx,
        &settings,
        &session.worktree,
        session.branch.clone(),
        base,
        request,
        CleanUpPolicy::Board,
    )?;
    Ok(match outcome {
        CleanUpExecution::Kept {
            reasons,
            git,
            closed,
        } => CleanUp::Kept {
            reasons,
            git,
            closed,
        },
        CleanUpExecution::Removed(r) => CleanUp::Removed(Removed {
            forced: r.forced,
            closed: r.closed,
            branch: r.branch.map(|b| BranchRemoval {
                name: b.name,
                deleted: b.deleted,
            }),
            tasks: r.tasks,
            task_errors: r.task_errors,
            hooks: r.hooks,
        }),
    })
}

/// The core worktree clean-up logic, runnable without a `board::Server`.
pub fn clean_up_worktree(
    ctx: &Context,
    settings: &Settings,
    worktree: &str,
    branch_hint: Option<String>,
    base: Option<String>,
    request: CleanUpRequest,
    policy: CleanUpPolicy,
) -> Result<CleanUpExecution, String> {
    let CleanUpRequest { force, confirm } = request;
    let repo = &ctx.repo;
    if same_path(worktree, &repo.main) {
        return Err("the main checkout is not a worktree to remove".to_string());
    }
    let listed = crate::kernel::identity::linked_worktrees(&repo.main)?;
    if !listed.iter().any(|w| same_path(w, worktree)) {
        return Err(format!("not a worktree of this repository: {worktree}"));
    }
    let name = Path::new(worktree)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if force && confirm.as_deref() != Some(name.as_str()) {
        return Err(format!("confirm has to be the worktree's name: {name}"));
    }

    // The tasks that name this worktree, in whichever hub they were made.
    let hubs = mail::all_repo_hubs(&ctx.state, repo);
    let mut tasks: Vec<(String, task::Task)> = Vec::new();
    for hub in &hubs {
        for t in task::list(&ctx.state, &hub.slug) {
            if t.worktree
                .as_deref()
                .is_some_and(|w| same_path(w, worktree))
            {
                tasks.push((hub.slug.clone(), t));
            }
        }
    }
    let is_starting = registry::is_starting(Path::new(worktree), crate::infra::clock::now_secs());
    let task_slices: Vec<task::Task> = tasks.iter().map(|(_, t)| t.clone()).collect();
    let refusals = task_and_session_refusals(&task_slices, is_starting);
    if let Some(first) = refusals.into_iter().next() {
        return Err(first);
    }

    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(crate::board::view::GIT_CHECK_SECS);
    let state = crate::kernel::worktree_state::worktree_git_state(
        Path::new(worktree),
        base.as_deref(),
        deadline,
    );
    let reasons = loss_reasons(&state);
    if !reasons.is_empty() && !force {
        return Ok(CleanUpExecution::Kept {
            reasons,
            git: state.ok().flatten(),
            closed: None,
        });
    }
    let state = state.ok().flatten();

    let main = Path::new(&repo.main);
    let was_running = registry::worker_status(Path::new(worktree)).present;
    // `close` finds the worktree free when no worker is there, so a stopped session is not
    // refused; `false` is a worker that may still be running, or a record that cannot be
    // read, and removing under either is what this must not do. Outside the dispatch lock,
    // which others give up waiting for after ten seconds.
    //
    // Only when something may be running: closing a worker that is gone clears its record, and
    // a removal that then fails would have taken the session's phase history for nothing.
    let may_run = match registry::read_worker(Path::new(worktree)) {
        registry::Recorded::Absent => false,
        registry::Recorded::Unreadable => true,
        registry::Recorded::Found(worker) => {
            registry::worker_liveness(&worker) != registry::Liveness::Gone
        }
    };
    if may_run && !crate::lifecycle::worker::close(settings, Path::new(worktree), false)?.is_free()
    {
        return Err("the session could not be closed; nothing was removed".to_string());
    }
    // Looked at again now that nothing is running: a worker that committed between the first
    // look and its tab closing has made work the first look did not see, and `branch -D`
    // below would drop it.
    if !force {
        let again_deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(crate::board::view::GIT_CHECK_SECS);
        let again = crate::kernel::worktree_state::worktree_git_state(
            Path::new(worktree),
            base.as_deref(),
            again_deadline,
        );
        let reasons = loss_reasons(&again);
        if !reasons.is_empty() {
            return Ok(CleanUpExecution::Kept {
                reasons,
                git: again.ok().flatten(),
                closed: Some(was_running),
            });
        }
    }
    // The lock covers only the last look at the worker slots and the marker that says the
    // worktree is going, which `claim_worker_slot` refuses on under the same lock: a worker
    // that starts here from now on is turned away. The removal itself, which can take long on
    // a big worktree, runs after the lock is released, so that a dispatch elsewhere is not
    // made to wait for it.
    let removing = registry::with_dispatch_lock(main, || -> Result<_, String> {
        if registry::holds_worker_slot(Path::new(worktree), crate::infra::clock::now_secs()) {
            return Err("a session started in it meanwhile; nothing was removed".to_string());
        }
        registry::mark_worktree_removing(main, Path::new(worktree))
    })??;
    let removed = remove_worktree(&ctx.state, &repo.main, worktree, force);
    // Released only now, whatever the removal came to.
    drop(removing);
    removed?;

    // The worker's record, its saved session and its outbox live inside the worktree, so the
    // session is off the board with it; nothing of it is kept in the state directory.
    let branch_name = state
        .as_ref()
        .and_then(|s| s.branch.clone())
        .or(branch_hint);
    let branch = match (policy, branch_name) {
        (CleanUpPolicy::Board, Some(name)) => {
            let deleted = delete_branch(&repo.main, &name);
            Some(CleanUpBranch {
                name,
                deleted,
                kept: None,
            })
        }
        (CleanUpPolicy::WorktreeRm, Some(name)) => {
            let is_merged = state.as_ref().and_then(|s| s.merged.merged) == Some(true);
            if is_merged {
                let deleted = delete_branch(&repo.main, &name);
                Some(CleanUpBranch {
                    name,
                    deleted,
                    kept: None,
                })
            } else {
                let kept_reason = match state
                    .as_ref()
                    .and_then(|s| s.merged.base.as_deref())
                    .or(base.as_deref())
                {
                    Some(b) => format!("not merged into {b}"),
                    None => "merged state unknown".to_string(),
                };
                Some(CleanUpBranch {
                    name,
                    deleted: Ok(()),
                    kept: Some(kept_reason),
                })
            }
        }
        (_, None) => None,
    };

    let hooks = crate::kernel::worktree_hooks::run_remove_hooks(
        &ctx.repo.nwo,
        &ctx.repo.main,
        worktree,
        &name,
    );

    // After the removal, as the hub does: a card left in progress for a worktree that is gone
    // would stay on the board for good.
    let mut done = Vec::new();
    let mut kept_tasks = Vec::new();
    let mut task_errors = Vec::new();
    let is_merged = state.as_ref().and_then(|s| s.merged.merged) == Some(true);
    for (slug, t) in tasks
        .iter()
        .filter(|(_, t)| matches!(t.status, Status::Dispatched | Status::Pr))
    {
        if policy == CleanUpPolicy::WorktreeRm && t.status == Status::Pr && !is_merged {
            kept_tasks.push(t.id.clone());
            continue;
        }
        let Some(hub) = hubs.iter().find(|h| &h.slug == slug) else {
            continue;
        };
        let mut addressed = repo.clone();
        addressed.slug = hub.slug.clone();
        addressed.hub_name = hub.name.clone();
        let task_ctx = Context {
            repo: addressed,
            resolved: ctx.resolved.clone(),
            state: ctx.state.clone(),
            settings: settings.clone(),
        };
        let done_patch = task::TaskPatch {
            status: Some(Status::Done),
            ..task::TaskPatch::default()
        };
        match task::update(&task_ctx, &t.id, &done_patch, false) {
            Ok(_) => done.push(t.id.clone()),
            Err(e) => task_errors.push((t.id.clone(), e)),
        }
    }
    Ok(CleanUpExecution::Removed(CleanUpRemoved {
        path: worktree.to_string(),
        forced: force,
        closed: was_running,
        branch,
        tasks: done,
        kept_tasks,
        task_errors,
        hooks,
    }))
}

/// Refusals that prevent worktree removal due to tasks or a starting session.
pub fn task_and_session_refusals(tasks: &[task::Task], is_starting: bool) -> Vec<String> {
    let mut refusals = Vec::new();
    if tasks.iter().any(|t| t.status == Status::Queued) {
        refusals.push("a queued task is waiting in it".to_string());
    }
    // A Jules plan is written in a detached worktree with no commits, which looks finished.
    if tasks.iter().any(|t| {
        t.executor == Executor::Jules && t.status == Status::Dispatched && t.jules_session.is_none()
    }) {
        refusals.push("a Jules plan is being written in it".to_string());
    }
    if is_starting {
        refusals.push("the session is starting".to_string());
    }
    refusals
}

/// Check if a worktree has any blocker that would prevent removal.
pub fn worktree_blockers(
    tasks: &[task::Task],
    is_starting: bool,
    git_state: &Result<Option<GitState>, String>,
) -> Vec<String> {
    let mut blockers = task_and_session_refusals(tasks, is_starting);
    for loss in loss_reasons(git_state) {
        blockers.push(loss.describe());
    }
    blockers
}

/// What removing the worktree would lose. A git that could not answer is a reason of its own.
pub fn loss_reasons(state: &Result<Option<GitState>, String>) -> Vec<Loss> {
    let state = match state {
        Err(e) => return vec![Loss::Git(e.clone())],
        // The directory is gone: there is nothing left to lose.
        Ok(None) => return Vec::new(),
        Ok(Some(state)) => state,
    };
    let mut reasons = Vec::new();
    let uncommitted = &state.uncommitted;
    if uncommitted.files > 0 {
        reasons.push(Loss::Uncommitted {
            files: uncommitted.files,
            insertions: uncommitted.insertions,
            deletions: uncommitted.deletions,
        });
    }
    if uncommitted.untracked > 0 {
        reasons.push(Loss::Untracked(uncommitted.untracked));
    }
    if state.unpushed.count > 0 {
        reasons.push(Loss::Unpushed(state.unpushed.count));
    }
    reasons
}

/// `git worktree remove`, forced only when the person forced it.
///
/// Adjutant's own files in the worktree (the worker's record, its saved session, the brief) are
/// untracked in a repository that does not ignore `.claude`, and git refuses to remove a
/// worktree with untracked files. Only when git says no are the ones no commit tracks moved out
/// of the way for a second try, and they are put back if that fails too: until the worktree is
/// gone they are what lets the session be seen and resumed. A directory that is already gone
/// is removed by name, which drops git's record of that one worktree and no other.
fn remove_worktree(root: &Path, main: &str, worktree: &str, force: bool) -> Result<(), String> {
    let mut args = vec!["-C", main, "worktree", "remove"];
    if force || !Path::new(worktree).is_dir() {
        args.push("--force");
    }
    args.push(worktree);
    let first = match git_ok(&args) {
        Ok(()) => return Ok(()),
        Err(e) if force => return Err(e),
        Err(e) => e,
    };
    // git's own message is the one that says why, so it is what is returned.
    let aside = set_aside_own_files(root, worktree)
        .map_err(|e| format!("{first} (and adjutant's files could not be moved aside: {e})"))?;
    match git_ok(&args) {
        Ok(()) => {
            aside.discard();
            Ok(())
        }
        Err(e) => match aside.restore() {
            Ok(()) => Err(e),
            Err(kept) => Err(format!("{e} ({kept})")),
        },
    }
}

/// Files moved out of a worktree: where each was, and the copy kept outside it.
struct Aside {
    files: Vec<(std::path::PathBuf, std::path::PathBuf)>,
}

impl Aside {
    /// Put back every file whose place is empty. The directory holding the copies goes only
    /// when all of them are back; otherwise it stays and the error says where.
    fn restore(self) -> Result<(), String> {
        let mut failed = false;
        for (original, kept) in &self.files {
            // A file that is there is the worker's own, possibly newer: never overwritten.
            if !original.exists() && move_file(kept, original).is_err() {
                failed = true;
            }
        }
        match (failed, self.dir()) {
            (false, dir) => {
                if let Some(dir) = dir {
                    let _ = std::fs::remove_dir_all(dir);
                }
                Ok(())
            }
            (true, dir) => Err(format!(
                "adjutant's files could not all be put back; they are kept in {}",
                dir.map(|d| d.display().to_string()).unwrap_or_default()
            )),
        }
    }

    fn discard(self) {
        if let Some(dir) = self.dir() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn dir(&self) -> Option<&Path> {
        self.files.first().and_then(|(_, kept)| kept.parent())
    }
}

/// Rename, or copy and remove where the two places are on different file systems. A copy that
/// fails part way is removed, so that it cannot later stand in for the original.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if let Err(e) = std::fs::copy(from, to) {
        let _ = std::fs::remove_file(to);
        return Err(e);
    }
    // A copy that stays beside a source that could not be removed would be a file the list of
    // what was moved does not know.
    std::fs::remove_file(from).inspect_err(|_| {
        let _ = std::fs::remove_file(to);
    })
}

/// Move `.claude/adjutant-*` and `.claude/task-brief.md` out of `worktree` unless git tracks
/// them, into a directory of the state directory made when the first one is moved.
fn set_aside_own_files(root: &Path, worktree: &str) -> Result<Aside, String> {
    let listed =
        crate::infra::git::git(&["-C", worktree, "ls-files", "-z", "--", ".claude"], None)?;
    if !listed.status.success() {
        return Err(format!(
            "cannot list the tracked files: {}",
            String::from_utf8_lossy(&listed.stderr).trim()
        ));
    }
    let tracked = String::from_utf8_lossy(&listed.stdout).to_string();
    let tracked: Vec<&str> = tracked.split('\0').collect();
    let mut aside = Aside { files: Vec::new() };
    let Ok(entries) = std::fs::read_dir(Path::new(worktree).join(".claude")) else {
        return Ok(aside);
    };
    // Read to the end before anything is moved, so that the listing is not of a directory
    // that is changing under it.
    let entries: Vec<_> = entries.flatten().collect();
    let dir = root.join(format!(
        "cleanup-{}-{}-{}",
        std::process::id(),
        crate::infra::clock::now_secs(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let mut made = false;
    for entry in entries {
        let file = entry.file_name().to_string_lossy().to_string();
        let ours = file.starts_with("adjutant-") || file == "task-brief.md";
        if !ours || tracked.contains(&format!(".claude/{file}").as_str()) {
            continue;
        }
        // `create_dir` for the first: a directory already there is not ours and is not written
        // into.
        let step = (if made {
            Ok(())
        } else {
            std::fs::create_dir_all(dir.parent().unwrap_or(&dir))
                .and_then(|_| std::fs::create_dir(&dir))
                .inspect(|_| made = true)
        })
        .and_then(|_| move_file(&entry.path(), &dir.join(&file)));
        match step {
            Ok(()) => aside.files.push((entry.path(), dir.join(&file))),
            Err(e) => {
                let why = format!("{}: {e}", entry.path().display());
                // The directory may have been made for a file that never got into it.
                if made && aside.files.is_empty() {
                    let _ = std::fs::remove_dir(&dir);
                }
                return match aside.restore() {
                    Ok(()) => Err(why),
                    Err(kept) => Err(format!("{why}; {kept}")),
                };
            }
        }
    }
    Ok(aside)
}

fn git_ok(args: &[&str]) -> Result<(), String> {
    let out = crate::infra::git::git(args, None)?;
    match out.status.success() {
        true => Ok(()),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// The local branch only. A remote ref is never touched from here: the branch may be the
/// head of a PR that is still open.
fn delete_branch(main: &str, branch: &str) -> Result<(), String> {
    git_ok(&["-C", main, "branch", "-D", "--", branch])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::worktree_state::{MergedInto, Uncommitted, Unpushed};

    fn state(uncommitted: Uncommitted, unpushed: usize) -> GitState {
        GitState {
            branch: Some("retry-upload".to_string()),
            head: Some("abc1234".to_string()),
            uncommitted,
            upstream: None,
            unpushed: Unpushed {
                count: unpushed,
                commits: Vec::new(),
                against: "remotes",
            },
            merged: MergedInto {
                base: None,
                reference: None,
                merged: None,
                reason: None,
            },
        }
    }

    #[test]
    fn uncommitted_untracked_and_unpushed_work_are_three_losses() {
        let dirty = state(
            Uncommitted {
                files: 3,
                untracked: 1,
                insertions: 10,
                deletions: 2,
                binary: 0,
            },
            2,
        );
        assert_eq!(
            loss_reasons(&Ok(Some(dirty))),
            vec![
                Loss::Uncommitted {
                    files: 3,
                    insertions: 10,
                    deletions: 2
                },
                Loss::Untracked(1),
                Loss::Unpushed(2)
            ]
        );
        assert_eq!(
            loss_reasons(&Err("no git".to_string())),
            vec![Loss::Git("no git".to_string())]
        );
        assert_eq!(loss_reasons(&Ok(None)), Vec::new());
    }
}
