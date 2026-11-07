//! Listing the repository's worktrees and their state.

use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::board::session::worktree_blockers;
use crate::infra::paths::same_path;
use crate::kernel::identity;
use crate::kernel::worktree_state::GitState;
use crate::mail;
use crate::registry::{self, Context};
use crate::task;

/// The list of worktrees and their readiness for removal.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListOutcome {
    pub worktrees: Vec<WorktreeRow>,
}

/// One worktree's state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRow {
    pub path: String,
    pub branch: Option<String>,
    pub detached: bool,
    pub git: Option<GitState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tasks: Vec<WorktreeTaskRow>,
    pub session: WorktreeSessionRow,
    pub removable: bool,
    pub blockers: Vec<String>,
}

/// A task record associated with a worktree.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeTaskRow {
    pub id: String,
    pub status: String,
    pub pr: Option<String>,
    pub hub: String,
}

/// Worker session status.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSessionRow {
    pub running: bool,
}

/// List every linked worktree of the repository, excluding the main checkout.
pub fn list(ctx: &Context) -> Result<ListOutcome, String> {
    let all = identity::worktrees(&ctx.repo.main)?;
    let linked: Vec<_> = all
        .into_iter()
        .filter(|w| !same_path(&w.path, &ctx.repo.main))
        .collect();

    let hubs = mail::all_repo_hubs(&ctx.state, &ctx.repo);
    let mut all_tasks: Vec<(String, task::Task)> = Vec::new();
    for hub in &hubs {
        for t in task::list(&ctx.state, &hub.slug) {
            all_tasks.push((hub.slug.clone(), t));
        }
    }

    let mut rows = Vec::new();
    for w in linked {
        let path = w.path;
        let branch = w.branch;
        let detached = branch.is_none();

        let mut tasks_for_row: Vec<WorktreeTaskRow> = Vec::new();
        let mut raw_tasks: Vec<task::Task> = Vec::new();
        for (hub_slug, t) in &all_tasks {
            if t.worktree.as_deref().is_some_and(|wt| same_path(wt, &path)) {
                tasks_for_row.push(WorktreeTaskRow {
                    id: t.id.clone(),
                    status: t.status.as_str().to_string(),
                    pr: t.pr.clone(),
                    hub: hub_slug.clone(),
                });
                raw_tasks.push(t.clone());
            }
        }

        let base = raw_tasks.iter().find_map(|t| t.base.as_deref());
        let deadline = Instant::now() + Duration::from_secs(crate::board::view::GIT_CHECK_SECS);
        let git_res =
            crate::kernel::worktree_state::worktree_git_state(Path::new(&path), base, deadline);

        let (git, error) = match git_res.as_ref() {
            Ok(Some(state)) => (Some(state.clone()), None),
            Ok(None) => (None, Some("worktree does not exist on disk".to_string())),
            Err(e) => (None, Some(e.clone())),
        };

        let running = registry::worker_status(Path::new(&path)).present;
        let session = WorktreeSessionRow { running };

        let is_starting = registry::is_starting(Path::new(&path), crate::infra::clock::now_secs());
        let blockers = worktree_blockers(&raw_tasks, is_starting, &git_res);
        let removable = blockers.is_empty();

        rows.push(WorktreeRow {
            path,
            branch,
            detached,
            git,
            error,
            tasks: tasks_for_row,
            session,
            removable,
            blockers,
        });
    }

    Ok(ListOutcome { worktrees: rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_git(args: &[&str], dir: &Path) {
        let out = crate::infra::git::git(args, Some(dir)).unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn setup_test_repo() -> (
        crate::testing::Sandbox,
        tempfile::TempDir,
        tempfile::TempDir,
        Context,
    ) {
        let sandbox = crate::testing::Sandbox::empty();
        let origin = tempfile::tempdir().unwrap();
        let main = tempfile::tempdir().unwrap();

        run_git(&["init", "-q", "--bare", "-b", "main"], origin.path());

        crate::testing::init_repo(main.path(), "main");
        run_git(&["config", "user.email", "test@example.com"], main.path());
        run_git(&["config", "user.name", "Tester"], main.path());
        run_git(
            &["commit", "--allow-empty", "-m", "initial commit"],
            main.path(),
        );
        run_git(
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
            main.path(),
        );
        run_git(&["push", "-u", "origin", "main"], main.path());
        run_git(&["remote", "set-head", "origin", "main"], main.path());

        let repo =
            crate::kernel::identity::resolve_in(Some(main.path()), Some("acme/widget"), None)
                .unwrap();
        let ctx = crate::registry::context_of(repo).unwrap();
        (sandbox, origin, main, ctx)
    }

    #[test]
    fn main_checkout_left_out_and_clean_worktree_is_removable() {
        let (_sandbox, _origin, main, ctx) = setup_test_repo();

        let wt_parent = tempfile::tempdir().unwrap();
        let wt_path = wt_parent.path().join("linked-wt");

        run_git(
            &[
                "worktree",
                "add",
                "-b",
                "feature-clean",
                wt_path.to_str().unwrap(),
                "origin/main",
            ],
            main.path(),
        );

        let outcome = list(&ctx).expect("list should succeed");
        // Main checkout is NOT in worktrees
        assert!(
            !outcome
                .worktrees
                .iter()
                .any(|w| same_path(&w.path, main.path().to_str().unwrap()))
        );

        assert_eq!(outcome.worktrees.len(), 1);
        let row = &outcome.worktrees[0];
        assert_eq!(row.branch.as_deref(), Some("feature-clean"));
        assert!(!row.detached);
        assert!(row.removable, "clean worktree should be removable");
        assert!(row.blockers.is_empty());
    }

    #[test]
    fn dirty_worktree_is_not_removable_with_blocker() {
        let (_sandbox, _origin, main, ctx) = setup_test_repo();

        let wt_parent = tempfile::tempdir().unwrap();
        let wt_path = wt_parent.path().join("dirty-wt");

        run_git(
            &[
                "worktree",
                "add",
                "-b",
                "feature-dirty",
                wt_path.to_str().unwrap(),
                "origin/main",
            ],
            main.path(),
        );

        // Make an untracked file
        std::fs::write(wt_path.join("untracked.txt"), "hello").unwrap();

        let outcome = list(&ctx).expect("list should succeed");
        assert_eq!(outcome.worktrees.len(), 1);
        let row = &outcome.worktrees[0];
        assert!(
            !row.removable,
            "dirty worktree with untracked file must not be removable"
        );
        assert!(!row.blockers.is_empty());
        assert!(row.blockers.iter().any(|b| b.contains("untracked")));
    }
}
