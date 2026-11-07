//! Safely removing a worktree.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::board::session::{
    CleanUpBranch, CleanUpExecution, CleanUpPolicy, CleanUpRemoved, CleanUpRequest, HookRun, Loss,
    clean_up_worktree,
};
use crate::infra::paths::same_path;
use crate::kernel::identity;
use crate::kernel::worktree_state::GitState;
use crate::mail;
use crate::registry::Context;
use crate::task;

/// Request to remove a worktree.
#[derive(Debug, Clone)]
pub struct RemoveRequest {
    pub path: String,
    pub branch: Option<String>,
    pub cwd: Option<PathBuf>,
}

/// Outcome of removing a worktree.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum RemoveOutcome {
    Removed(RemoveSuccess),
    Kept(RemoveKept),
}

impl RemoveOutcome {
    pub fn is_removed(&self) -> bool {
        matches!(self, Self::Removed(_))
    }
}

/// What removing a worktree did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveSuccess {
    pub removed: bool,
    pub path: String,
    pub closed: bool,
    pub branch: Option<RemoveBranchOutcome>,
    pub tasks: Vec<String>,
    pub kept_tasks: Vec<String>,
    pub task_errors: Vec<TaskError>,
    pub hooks: Vec<HookRun>,
}

/// Branch removal or retention result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveBranchOutcome {
    pub name: String,
    pub deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kept: Option<String>,
}

/// A task that could not be updated to done.
#[derive(Debug, Clone, Serialize)]
pub struct TaskError {
    pub id: String,
    pub error: String,
}

/// Why removing the worktree was refused to prevent losing work.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveKept {
    pub removed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
    pub reasons: Vec<Loss>,
    pub git: Option<GitState>,
}

/// Safely remove a worktree, closing any running session first.
pub fn remove(ctx: &Context, request: RemoveRequest) -> Result<RemoveOutcome, String> {
    let abs_path = crate::infra::paths::absolute(&request.path)
        .to_string_lossy()
        .to_string();

    let caller_cwd = request
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let cwd = std::fs::canonicalize(&caller_cwd).unwrap_or(caller_cwd);
    let target =
        std::fs::canonicalize(Path::new(&abs_path)).unwrap_or_else(|_| PathBuf::from(&abs_path));
    if cwd == target || cwd.starts_with(&target) {
        return Err("cannot remove the worktree you are standing in".to_string());
    }

    if same_path(&abs_path, &ctx.repo.main) {
        return Err("the main checkout is not a worktree to remove".to_string());
    }

    let all = identity::worktrees(&ctx.repo.main)?;
    let entry = all.iter().find(|w| same_path(&w.path, &abs_path));
    let Some(entry) = entry else {
        return Err(format!("not a worktree of this repository: {abs_path}"));
    };

    if let Some(ref expected) = request.branch {
        let actual = entry.branch.as_deref().unwrap_or("detached HEAD");
        if actual != expected {
            return Err(format!(
                "{abs_path} has {actual} checked out, not {expected}"
            ));
        }
    }

    let hubs = mail::all_repo_hubs(&ctx.state, &ctx.repo);
    let base = hubs.iter().find_map(|hub| {
        task::list(&ctx.state, &hub.slug)
            .into_iter()
            .find(|t| {
                t.worktree
                    .as_deref()
                    .is_some_and(|w| same_path(w, &abs_path))
            })
            .and_then(|t| t.base)
    });

    let execution = clean_up_worktree(
        ctx,
        &ctx.settings,
        &abs_path,
        request.branch.clone().or_else(|| entry.branch.clone()),
        base,
        CleanUpRequest {
            force: false,
            confirm: None,
        },
        CleanUpPolicy::WorktreeRm,
    )?;

    match execution {
        CleanUpExecution::Kept {
            reasons,
            git,
            closed,
        } => Ok(RemoveOutcome::Kept(RemoveKept {
            removed: false,
            closed,
            reasons,
            git,
        })),
        CleanUpExecution::Removed(CleanUpRemoved {
            path,
            closed,
            branch,
            tasks,
            kept_tasks,
            task_errors,
            hooks,
            ..
        }) => {
            let branch = branch.map(
                |CleanUpBranch {
                     name,
                     deleted,
                     kept,
                 }| {
                    let (is_deleted, error) = match deleted {
                        Ok(()) => (kept.is_none(), None),
                        Err(e) => (false, Some(e)),
                    };
                    RemoveBranchOutcome {
                        name,
                        deleted: is_deleted,
                        error,
                        kept,
                    }
                },
            );
            Ok(RemoveOutcome::Removed(RemoveSuccess {
                removed: true,
                path,
                closed,
                branch,
                tasks,
                kept_tasks,
                task_errors: task_errors
                    .into_iter()
                    .map(|(id, error)| TaskError { id, error })
                    .collect(),
                hooks,
            }))
        }
    }
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
    fn refuses_uncommitted_untracked_and_unpushed_work() {
        let (_sandbox, _origin, main, ctx) = setup_test_repo();

        let wt_parent = tempfile::tempdir().unwrap();
        let wt_path = wt_parent.path().join("dirty-rm");

        run_git(
            &[
                "worktree",
                "add",
                "-b",
                "branch-dirty",
                wt_path.to_str().unwrap(),
                "origin/main",
            ],
            main.path(),
        );

        // Add untracked file
        std::fs::write(wt_path.join("untracked.txt"), "content").unwrap();

        let req = RemoveRequest {
            path: wt_path.to_string_lossy().to_string(),
            branch: None,
            cwd: Some(main.path().to_path_buf()),
        };

        let outcome = remove(&ctx, req).expect("remove should return Kept");
        assert!(!outcome.is_removed(), "dirty worktree must be Kept");
        assert!(wt_path.is_dir(), "worktree directory must still exist");
    }

    #[test]
    fn refuses_branch_mismatch() {
        let (_sandbox, _origin, main, ctx) = setup_test_repo();

        let wt_parent = tempfile::tempdir().unwrap();
        let wt_path = wt_parent.path().join("mismatch-rm");

        run_git(
            &[
                "worktree",
                "add",
                "-b",
                "actual-branch",
                wt_path.to_str().unwrap(),
                "origin/main",
            ],
            main.path(),
        );

        let req = RemoveRequest {
            path: wt_path.to_string_lossy().to_string(),
            branch: Some("wrong-branch".to_string()),
            cwd: Some(main.path().to_path_buf()),
        };

        let err = remove(&ctx, req).unwrap_err();
        assert!(
            err.contains("has actual-branch checked out, not wrong-branch"),
            "{err}"
        );
    }

    #[test]
    fn refuses_the_worktree_you_stand_in() {
        let (_sandbox, _origin, main, ctx) = setup_test_repo();

        let wt_parent = tempfile::tempdir().unwrap();
        let wt_path = wt_parent.path().join("inside-rm");

        run_git(
            &[
                "worktree",
                "add",
                "-b",
                "inside-branch",
                wt_path.to_str().unwrap(),
                "origin/main",
            ],
            main.path(),
        );

        let req = RemoveRequest {
            path: wt_path.to_string_lossy().to_string(),
            branch: None,
            cwd: Some(wt_path.clone()),
        };

        let err = remove(&ctx, req).unwrap_err();
        assert_eq!(err, "cannot remove the worktree you are standing in");
    }

    #[test]
    fn removes_clean_merged_one_and_deletes_its_branch() {
        let (_sandbox, _origin, main, ctx) = setup_test_repo();

        let wt_parent = tempfile::tempdir().unwrap();
        let wt_path = wt_parent.path().join("clean-rm");

        run_git(
            &[
                "worktree",
                "add",
                "-b",
                "clean-branch",
                wt_path.to_str().unwrap(),
                "origin/main",
            ],
            main.path(),
        );

        let req = RemoveRequest {
            path: wt_path.to_string_lossy().to_string(),
            branch: Some("clean-branch".to_string()),
            cwd: Some(main.path().to_path_buf()),
        };

        let outcome = remove(&ctx, req).expect("remove should succeed");
        assert!(outcome.is_removed());
        if let RemoveOutcome::Removed(res) = outcome {
            assert_eq!(res.path, wt_path.to_string_lossy().to_string());
            let branch = res.branch.expect("branch should be reported");
            assert_eq!(branch.name, "clean-branch");
            assert!(branch.deleted);
            assert!(branch.kept.is_none());
        }

        assert!(!wt_path.exists());
        // Verify local branch is deleted
        let branch_check = crate::infra::git::git(
            &["rev-parse", "-q", "--verify", "refs/heads/clean-branch"],
            Some(main.path()),
        )
        .unwrap();
        assert!(!branch_check.status.success());
    }

    #[test]
    fn keeps_unmerged_branch_and_leaves_pr_task_in_pr() {
        let (_sandbox, _origin, main, ctx) = setup_test_repo();

        let wt_parent = tempfile::tempdir().unwrap();
        let wt_path = wt_parent.path().join("unmerged-rm");

        run_git(
            &[
                "worktree",
                "add",
                "-b",
                "unmerged-branch",
                wt_path.to_str().unwrap(),
                "origin/main",
            ],
            main.path(),
        );

        // Commit and push on the unmerged branch (so it's unmerged into main, but pushed to remote)
        run_git(&["config", "user.email", "test@example.com"], &wt_path);
        run_git(&["config", "user.name", "Tester"], &wt_path);
        run_git(
            &["commit", "--allow-empty", "-m", "feature commit"],
            &wt_path,
        );
        run_git(&["push", "-u", "origin", "unmerged-branch"], &wt_path);

        // Create a task with status: Pr in this worktree
        let new_task = task::NewTask {
            title: Some("My PR task".to_string()),
            worktree: Some(wt_path.to_string_lossy().to_string()),
            base: Some("origin/main".to_string()),
            status: task::Status::Pr,
            ..Default::default()
        };
        let (created, _) =
            task::create(&ctx, new_task, false).expect("task creation should succeed");
        let task_id = created.id;

        let req = RemoveRequest {
            path: wt_path.to_string_lossy().to_string(),
            branch: Some("unmerged-branch".to_string()),
            cwd: Some(main.path().to_path_buf()),
        };

        let outcome = remove(&ctx, req).expect("remove should succeed");
        assert!(outcome.is_removed());
        if let RemoveOutcome::Removed(res) = outcome {
            let branch = res.branch.expect("branch should be reported");
            assert_eq!(branch.name, "unmerged-branch");
            assert!(!branch.deleted, "unmerged branch must not be deleted");
            assert!(
                branch.kept.as_ref().unwrap().contains("not merged into"),
                "kept reason must say not merged: {:?}",
                branch.kept
            );
            assert_eq!(res.tasks, Vec::<String>::new());
            assert_eq!(res.kept_tasks, vec![task_id.clone()]);
        }

        // Verify task is still in Pr status
        let task_after =
            task::get(&ctx.state, &ctx.repo.slug, &task_id).expect("task should exist");
        assert_eq!(task_after.status, task::Status::Pr);

        // Verify local branch still exists
        let branch_check = crate::infra::git::git(
            &["rev-parse", "-q", "--verify", "refs/heads/unmerged-branch"],
            Some(main.path()),
        )
        .unwrap();
        assert!(branch_check.status.success());
    }

    #[test]
    fn a_pr_task_is_done_when_its_branch_is_merged() {
        let (_sandbox, _origin, main, ctx) = setup_test_repo();

        let wt_parent = tempfile::tempdir().unwrap();
        let wt_path = wt_parent.path().join("merged-rm");
        run_git(
            &[
                "worktree",
                "add",
                "-b",
                "merged-branch",
                wt_path.to_str().unwrap(),
                "origin/main",
            ],
            main.path(),
        );
        let (created, _) = task::create(
            &ctx,
            task::NewTask {
                title: Some("Merged PR task".to_string()),
                worktree: Some(wt_path.to_string_lossy().to_string()),
                base: Some("origin/main".to_string()),
                status: task::Status::Pr,
                ..Default::default()
            },
            false,
        )
        .expect("task creation should succeed");

        let outcome = remove(
            &ctx,
            RemoveRequest {
                path: wt_path.to_string_lossy().to_string(),
                branch: None,
                cwd: Some(main.path().to_path_buf()),
            },
        )
        .expect("remove should succeed");
        let RemoveOutcome::Removed(res) = outcome else {
            panic!("a clean merged worktree must be removed");
        };
        assert_eq!(res.tasks, vec![created.id.clone()]);
        assert!(res.kept_tasks.is_empty());
        let after = task::get(&ctx.state, &ctx.repo.slug, &created.id).unwrap();
        assert_eq!(after.status, task::Status::Done);
    }
}
