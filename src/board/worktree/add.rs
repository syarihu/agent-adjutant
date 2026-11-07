//! Adding a worktree to the repository.

use std::path::Path;

use serde::Serialize;

use crate::kernel::identity;
use crate::kernel::worktree_hooks::HookRun;
use crate::registry::Context;
use crate::task;

/// Arguments to create a new worktree.
#[derive(Debug, Clone, Default)]
pub struct AddRequest {
    pub name: Option<String>,
    pub user: Option<String>,
    pub pattern: Option<String>,
    pub branch: Option<String>,
    pub path: Option<String>,
    pub base: Option<String>,
}

/// The result of creating a worktree.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddOutcome {
    pub name: String,
    pub branch: String,
    pub path: String,
    pub base: String,
    pub main: String,
    pub post_create: Vec<HookRun>,
}

/// Add a new worktree and branch, run postCreate hooks, and return the outcome.
pub fn add(ctx: &Context, request: AddRequest) -> Result<AddOutcome, String> {
    let name = request
        .name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let user = request
        .user
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let pattern = request
        .pattern
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let branch = request
        .branch
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let path = request
        .path
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let base = request
        .base
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if name.is_none() && path.is_none() {
        return Err("pass --name, or --path with --branch".to_string());
    }
    if path.is_some() && branch.is_none() && name.is_none() {
        return Err("pass --name, or --path with --branch".to_string());
    }
    if let Some(ref n) = name {
        task::check_worktree_name(n)?;
    }
    if let Some(ref b) = branch {
        if b.starts_with('-') {
            return Err(format!("git cannot name a branch: {b}"));
        }
        let out = crate::infra::git::git(&["check-ref-format", "--branch", b], None)?;
        if !out.status.success() {
            return Err(format!("git cannot name a branch: {b}"));
        }
    }

    let branch = match branch {
        Some(b) => b,
        None => {
            let user = user
                .unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "worker".to_string()));
            let pattern_str = pattern
                .as_deref()
                .unwrap_or(identity::DEFAULT_BRANCH_PATTERN);
            identity::branch_fallback(pattern_str, &user, name.as_deref().unwrap())
        }
    };
    let path = match path {
        Some(p) => crate::infra::paths::absolute(&p)
            .to_string_lossy()
            .to_string(),
        None => {
            let layout = ctx
                .settings
                .worktree_pattern
                .as_deref()
                .unwrap_or(identity::DEFAULT_WORKTREE_PATTERN);
            identity::worktree_fallback(layout, &ctx.repo.main, &branch)?
        }
    };
    let name = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| format!("cannot get worktree name from path: {path}"))?;

    if Path::new(&path).exists() {
        return Err(format!("path already exists: {path}"));
    }
    let verify = crate::infra::git::git(
        &[
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/heads/{branch}"),
        ],
        Some(Path::new(&ctx.repo.main)),
    )?;
    if verify.status.success() {
        return Err(format!("branch already exists: {branch}"));
    }

    let fetch = crate::infra::git::git(
        &["fetch", "--prune", "origin"],
        Some(Path::new(&ctx.repo.main)),
    )?;
    if !fetch.status.success() {
        let stderr = String::from_utf8_lossy(&fetch.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "git fetch --prune origin failed".to_string()
        } else {
            stderr
        });
    }

    let candidate_base = match base {
        Some(b) => b,
        None => {
            let resolved = crate::kernel::config::resolve_config(&ctx.repo.nwo)?;
            resolved
                .config
                .as_ref()
                .and_then(|c| c.get("baseBranch"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("auto")
                .to_string()
        }
    };
    let resolved_base = crate::jules::decide_base(&ctx.repo.main, &ctx.repo.nwo, &candidate_base)?;

    let add_out = crate::infra::git::git(
        &[
            "-C",
            &ctx.repo.main,
            "worktree",
            "add",
            "-b",
            &branch,
            &path,
            &resolved_base,
        ],
        None,
    )?;
    if !add_out.status.success() {
        let stderr = String::from_utf8_lossy(&add_out.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "git worktree add failed".to_string()
        } else {
            stderr
        });
    }

    let post_create = crate::kernel::worktree_hooks::run_create_hooks(
        &ctx.repo.nwo,
        &ctx.repo.main,
        &path,
        &name,
    );

    Ok(AddOutcome {
        name,
        branch,
        path,
        base: resolved_base,
        main: ctx.repo.main.clone(),
        post_create,
    })
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
        let sandbox = crate::testing::Sandbox::new(
            r#"{
                "repos": {
                    "acme/widget": {
                        "postCreate": ["touch {worktree}/hook.txt"]
                    }
                }
            }"#,
        );
        let origin = tempfile::tempdir().unwrap();
        let main = tempfile::tempdir().unwrap();

        // Bare origin
        run_git(&["init", "-q", "--bare", "-b", "main"], origin.path());

        // Main checkout
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
    fn creates_worktree_on_resolved_base_and_runs_post_create() {
        let (_sandbox, _origin, main, ctx) = setup_test_repo();

        let wt_parent = tempfile::tempdir().unwrap();
        let wt_path = wt_parent.path().join("my-worktree");

        let req = AddRequest {
            name: Some("my-worktree".to_string()),
            path: Some(wt_path.to_string_lossy().to_string()),
            branch: Some("feature-1".to_string()),
            ..Default::default()
        };

        let outcome = add(&ctx, req).expect("add should succeed");
        assert_eq!(outcome.name, "my-worktree");
        assert_eq!(outcome.branch, "feature-1");
        assert_eq!(outcome.base, "origin/main");
        assert!(Path::new(&outcome.path).is_dir());
        assert!(Path::new(&outcome.path).join("hook.txt").exists());
        assert_eq!(outcome.post_create.len(), 1);
        assert!(outcome.post_create[0].result.is_ok());

        // Clean up git worktree
        let _ = crate::infra::git::git(
            &["worktree", "remove", "--force", &outcome.path],
            Some(Path::new(&main.path())),
        );
    }

    #[test]
    fn refuses_existing_path_or_branch_before_fetching() {
        let (_sandbox, _origin, main, ctx) = setup_test_repo();

        // 1. Existing branch
        run_git(&["branch", "existing-branch"], main.path());
        let req = AddRequest {
            name: Some("wt-1".to_string()),
            branch: Some("existing-branch".to_string()),
            ..Default::default()
        };
        let err = add(&ctx, req).unwrap_err();
        assert!(
            err.contains("branch already exists: existing-branch"),
            "{err}"
        );

        // 2. Existing path
        let existing_dir = tempfile::tempdir().unwrap();
        let req = AddRequest {
            name: Some("wt-2".to_string()),
            path: Some(existing_dir.path().to_string_lossy().to_string()),
            branch: Some("branch-2".to_string()),
            ..Default::default()
        };
        let err = add(&ctx, req).unwrap_err();
        assert!(err.contains("path already exists"), "{err}");
    }

    #[test]
    fn base_that_does_not_exist_is_an_error_not_default() {
        let (_sandbox, _origin, _main, ctx) = setup_test_repo();

        let req = AddRequest {
            name: Some("wt-bad-base".to_string()),
            base: Some("nonexistent-base-branch".to_string()),
            branch: Some("feature-bad".to_string()),
            ..Default::default()
        };
        let err = add(&ctx, req).unwrap_err();
        assert!(
            err.contains("does not exist on origin"),
            "expected error about base not existing, got: {err}"
        );
    }
}
