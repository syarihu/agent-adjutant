//! Check a PR's base branch against the brief's Base branch, and move it if it differs.

use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::infra::gh::GhRun;
use crate::kernel::brief;

/// How long one `gh` operation may take.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Runs `gh` with these arguments by this deadline: `crate::infra::gh::run`, or a stand-in in tests.
pub type GhRunner<'a> = dyn Fn(&[&str], Instant) -> Result<GhRun, String> + 'a;

/// The outcome of checking and possibly updating a PR's base branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckPrBaseResult {
    pub pr: u64,
    pub url: String,
    pub repo: String,
    pub base: String,
    pub was: String,
    pub changed: bool,
}

/// Strip a leading `origin/` from a branch name.
fn strip_origin(base: &str) -> &str {
    base.strip_prefix("origin/").unwrap_or(base)
}

/// What `gh pr view --json baseRefName,headRefName,url` says of a PR.
struct PrView {
    base: String,
    head: String,
    url: String,
}

fn parse_pr_view(stdout: &str) -> Result<PrView, String> {
    let val: serde_json::Value =
        serde_json::from_str(stdout).map_err(|e| format!("cannot read gh's answer: {e}"))?;
    let base = val
        .get("baseRefName")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "gh pr view answer has no baseRefName".to_string())?
        .to_string();
    let head = val
        .get("headRefName")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "gh pr view answer has no headRefName".to_string())?
        .to_string();
    let url = val
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(PrView { base, head, url })
}

/// Check a PR's base against the brief in `worktree`, moving it with `gh pr edit` if it differs.
///
/// Only the worktree's own PR is moved: one whose head is another branch was named by a wrong
/// number, and moving it would change the diff of somebody else's review.
pub fn check_pr_base(
    worktree: Option<&Path>,
    pr: u64,
    repo: Option<&str>,
) -> Result<CheckPrBaseResult, String> {
    let dir = worktree.unwrap_or_else(|| Path::new("."));
    check_pr_base_with(
        worktree,
        pr,
        repo,
        &|| current_branch(dir),
        &|args, deadline| crate::infra::gh::run(None, args, deadline),
    )
}

/// The branch the worktree at `dir` is on; an error for a detached head.
fn current_branch(dir: &Path) -> Result<String, String> {
    let out = crate::infra::git::git(&["rev-parse", "--abbrev-ref", "HEAD"], Some(dir))?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || branch.is_empty() || branch == "HEAD" {
        return Err(format!(
            "cannot tell which branch the worktree at {} is on: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(branch)
}

/// Same as `check_pr_base`, but taking the worktree's branch and the `gh` runner so tests can run
/// without git or `gh`. The branch is asked for only once the brief has named a base.
pub fn check_pr_base_with(
    worktree: Option<&Path>,
    pr: u64,
    repo: Option<&str>,
    head: &dyn Fn() -> Result<String, String>,
    gh: &GhRunner<'_>,
) -> Result<CheckPrBaseResult, String> {
    let worktree_dir = worktree.unwrap_or_else(|| Path::new("."));

    let brief_path = worktree_dir.join(".claude").join("task-brief.md");
    let brief_content = std::fs::read_to_string(&brief_path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            format!("task brief was not found at {}", brief_path.display())
        }
        _ => format!(
            "cannot read the task brief at {}: {e}",
            brief_path.display()
        ),
    })?;

    let base_raw = brief::parse_base_branch(&brief_content);
    let base_val = match base_raw {
        Some(val) if !val.trim().is_empty() && val.trim() != "-" => val.trim(),
        _ => {
            return Err(
                "the brief names no base branch: ask the user which base the PR targets (never guess the default branch)".to_string(),
            );
        }
    };

    let head = head()?;

    let repo = match repo.map(str::trim).filter(|s| !s.is_empty()) {
        Some(explicit) => explicit.to_string(),
        None => {
            let info = crate::kernel::identity::resolve_in(Some(worktree_dir), None, None)?;
            info.nwo
        }
    };

    let wanted_base = strip_origin(base_val);
    if wanted_base.is_empty() {
        return Err(format!(
            "the brief's base `{base_val}` names no branch: ask the user which base the PR targets (never guess the default branch)"
        ));
    }
    let pr_str = pr.to_string();
    let view_args = [
        "pr",
        "view",
        &pr_str,
        "-R",
        &repo,
        "--json",
        "baseRefName,headRefName,url",
    ];

    let deadline = Instant::now() + crate::infra::env::deadline(TIMEOUT);
    let view_run = gh(&view_args, deadline)?;
    if !view_run.ok {
        let err = if view_run.stderr.is_empty() {
            "gh pr view failed".to_string()
        } else {
            view_run.stderr
        };
        return Err(format!("gh pr view failed: {err}"));
    }

    let PrView {
        base: current_base,
        head: pr_head,
        url,
    } = parse_pr_view(&view_run.stdout)?;
    if pr_head != head {
        return Err(format!(
            "PR #{pr} is on {pr_head}, not this worktree's branch {head}: check the PR number"
        ));
    }
    if current_base == wanted_base {
        return Ok(CheckPrBaseResult {
            pr,
            url,
            repo,
            base: wanted_base.to_string(),
            was: current_base,
            changed: false,
        });
    }

    let edit_args = ["pr", "edit", &pr_str, "-R", &repo, "--base", wanted_base];
    let edit_deadline = Instant::now() + crate::infra::env::deadline(TIMEOUT);
    // `gh` that could not be run or did not finish in time may still have reached GitHub.
    let edit_run = gh(&edit_args, edit_deadline).map_err(|e| {
        format!("moving PR #{pr} to {wanted_base} may or may not have gone through: {e}")
    })?;
    if !edit_run.ok {
        let err = if edit_run.stderr.is_empty() {
            "gh pr edit failed".to_string()
        } else {
            edit_run.stderr
        };
        return Err(format!("gh pr edit failed: {err}"));
    }

    let confirm_deadline = Instant::now() + crate::infra::env::deadline(TIMEOUT);
    // The edit has been sent by now, so a failed look must not read as "nothing changed".
    let unconfirmed =
        |why: String| format!("moved PR #{pr} to {wanted_base} but could not confirm it: {why}");
    let confirm_run = gh(&view_args, confirm_deadline).map_err(unconfirmed)?;
    if !confirm_run.ok {
        let err = if confirm_run.stderr.is_empty() {
            "gh pr view failed".to_string()
        } else {
            confirm_run.stderr
        };
        return Err(unconfirmed(err));
    }

    let PrView {
        base: confirmed_base,
        url: confirmed_url,
        ..
    } = parse_pr_view(&confirm_run.stdout).map_err(unconfirmed)?;
    if confirmed_base != wanted_base {
        let err_detail = if !edit_run.stderr.is_empty() {
            format!(": {}", edit_run.stderr)
        } else if !confirm_run.stderr.is_empty() {
            format!(": {}", confirm_run.stderr)
        } else {
            String::new()
        };
        return Err(format!(
            "PR base is still {confirmed_base} after editing to {wanted_base}{err_detail}"
        ));
    }

    Ok(CheckPrBaseResult {
        pr,
        url: if confirmed_url.is_empty() {
            url
        } else {
            confirmed_url
        },
        repo,
        base: wanted_base.to_string(),
        was: current_base,
        changed: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn head() -> Result<String, String> {
        Ok("syarihu/x".to_string())
    }

    fn make_brief(dir: &Path, base: &str) {
        let claude = dir.join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        let content = format!(
            "- Task: WID-1 \"Title\" (github)\n- Workspace: ...\n- Base branch: {base}\n- Parent task: -\n"
        );
        std::fs::write(claude.join("task-brief.md"), content).unwrap();
    }

    #[test]
    fn already_right_no_edit_called() {
        let temp = tempfile::tempdir().unwrap();
        make_brief(temp.path(), "main");

        let edit_called = AtomicUsize::new(0);
        let gh = |args: &[&str], _deadline: Instant| -> Result<GhRun, String> {
            if args[0] == "pr" && args[1] == "view" {
                assert_eq!(args[2], "12");
                assert_eq!(args[3], "-R");
                assert_eq!(args[4], "acme/widget");
                return Ok(GhRun {
                    ok: true,
                    stdout: r#"{"headRefName": "syarihu/x", "baseRefName": "main", "url": "https://github.com/acme/widget/pull/12"}"#.to_string(),
                    stderr: String::new(),
                });
            }
            if args[0] == "pr" && args[1] == "edit" {
                edit_called.fetch_add(1, Ordering::SeqCst);
                return Ok(GhRun {
                    ok: true,
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            panic!("unexpected gh call: {args:?}");
        };

        let result =
            check_pr_base_with(Some(temp.path()), 12, Some("acme/widget"), &head, &gh).unwrap();
        assert_eq!(edit_called.load(Ordering::SeqCst), 0);
        assert_eq!(result.pr, 12);
        assert_eq!(result.repo, "acme/widget");
        assert_eq!(result.base, "main");
        assert_eq!(result.was, "main");
        assert_eq!(result.url, "https://github.com/acme/widget/pull/12");
        assert!(!result.changed);
    }

    #[test]
    fn wrong_and_fixed_origin_stripped() {
        let temp = tempfile::tempdir().unwrap();
        make_brief(temp.path(), "origin/release/1.2");

        let call_count = AtomicUsize::new(0);
        let edit_args_seen: Mutex<Vec<String>> = Mutex::new(Vec::new());

        let gh = |args: &[&str], _deadline: Instant| -> Result<GhRun, String> {
            let count = call_count.fetch_add(1, Ordering::SeqCst);
            if count == 0 {
                // First view
                assert_eq!(args[1], "view");
                return Ok(GhRun {
                    ok: true,
                    stdout: r#"{"headRefName": "syarihu/x", "baseRefName": "main", "url": "https://github.com/acme/widget/pull/12"}"#.to_string(),
                    stderr: String::new(),
                });
            } else if count == 1 {
                // Edit
                assert_eq!(args[1], "edit");
                assert_eq!(args[2], "12");
                assert_eq!(args[3], "-R");
                assert_eq!(args[4], "acme/widget");
                assert_eq!(args[5], "--base");
                assert_eq!(args[6], "release/1.2"); // stripped origin/!
                edit_args_seen
                    .lock()
                    .unwrap()
                    .extend(args.iter().map(|s| s.to_string()));
                return Ok(GhRun {
                    ok: true,
                    stdout: String::new(),
                    stderr: String::new(),
                });
            } else if count == 2 {
                // Confirm view
                assert_eq!(args[1], "view");
                return Ok(GhRun {
                    ok: true,
                    stdout: r#"{"headRefName": "syarihu/x", "baseRefName": "release/1.2", "url": "https://github.com/acme/widget/pull/12"}"#.to_string(),
                    stderr: String::new(),
                });
            }
            panic!("too many gh calls");
        };

        let result =
            check_pr_base_with(Some(temp.path()), 12, Some("acme/widget"), &head, &gh).unwrap();
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
        assert_eq!(result.pr, 12);
        assert_eq!(result.repo, "acme/widget");
        assert_eq!(result.base, "release/1.2");
        assert_eq!(result.was, "main");
        assert_eq!(result.url, "https://github.com/acme/widget/pull/12");
        assert!(result.changed);
    }

    #[test]
    fn edit_fails_returns_err() {
        let temp = tempfile::tempdir().unwrap();
        make_brief(temp.path(), "origin/release/1.2");

        let gh = |args: &[&str], _deadline: Instant| -> Result<GhRun, String> {
            if args[1] == "view" {
                return Ok(GhRun {
                    ok: true,
                    stdout: r#"{"headRefName": "syarihu/x", "baseRefName": "main", "url": "https://github.com/acme/widget/pull/12"}"#.to_string(),
                    stderr: String::new(),
                });
            }
            if args[1] == "edit" {
                return Ok(GhRun {
                    ok: false,
                    stdout: String::new(),
                    stderr: "base branch does not exist on remote".to_string(),
                });
            }
            panic!("unexpected gh call: {args:?}");
        };

        let err =
            check_pr_base_with(Some(temp.path()), 12, Some("acme/widget"), &head, &gh).unwrap_err();
        assert!(
            err.contains("base branch does not exist on remote"),
            "{err}"
        );
    }

    #[test]
    fn edit_does_not_change_base_returns_err() {
        let temp = tempfile::tempdir().unwrap();
        make_brief(temp.path(), "release/1.2");

        let call_count = AtomicUsize::new(0);
        let gh = |_args: &[&str], _deadline: Instant| -> Result<GhRun, String> {
            let count = call_count.fetch_add(1, Ordering::SeqCst);
            if count == 0 {
                return Ok(GhRun {
                    ok: true,
                    stdout: r#"{"headRefName": "syarihu/x", "baseRefName": "main", "url": "https://github.com/acme/widget/pull/12"}"#.to_string(),
                    stderr: String::new(),
                });
            } else if count == 1 {
                return Ok(GhRun {
                    ok: true,
                    stdout: String::new(),
                    stderr: String::new(),
                });
            } else if count == 2 {
                // Still main!
                return Ok(GhRun {
                    ok: true,
                    stdout: r#"{"headRefName": "syarihu/x", "baseRefName": "main", "url": "https://github.com/acme/widget/pull/12"}"#.to_string(),
                    stderr: String::new(),
                });
            }
            panic!("too many gh calls");
        };

        let err =
            check_pr_base_with(Some(temp.path()), 12, Some("acme/widget"), &head, &gh).unwrap_err();
        assert!(err.contains("still main"), "{err}");
    }

    #[test]
    fn view_fails_returns_err() {
        let temp = tempfile::tempdir().unwrap();
        make_brief(temp.path(), "main");

        let gh = |args: &[&str], _deadline: Instant| -> Result<GhRun, String> {
            if args[1] == "view" {
                return Ok(GhRun {
                    ok: false,
                    stdout: String::new(),
                    stderr: "Could not resolve to a PullRequest with the number 12.".to_string(),
                });
            }
            panic!("unexpected gh call: {args:?}");
        };

        let err =
            check_pr_base_with(Some(temp.path()), 12, Some("acme/widget"), &head, &gh).unwrap_err();
        assert!(err.contains("Could not resolve to a PullRequest"), "{err}");
    }

    #[test]
    fn brief_missing_returns_err() {
        let temp = tempfile::tempdir().unwrap();
        let gh = |_args: &[&str], _deadline: Instant| -> Result<GhRun, String> {
            panic!("gh should not be called when brief is missing");
        };
        let err =
            check_pr_base_with(Some(temp.path()), 12, Some("acme/widget"), &head, &gh).unwrap_err();
        assert!(err.contains("brief") && err.contains("not found"), "{err}");
    }

    #[test]
    fn base_line_dash_or_empty_returns_err() {
        let temp = tempfile::tempdir().unwrap();
        make_brief(temp.path(), "-");
        let gh = |_args: &[&str], _deadline: Instant| -> Result<GhRun, String> {
            panic!("gh should not be called when base is -");
        };
        let err =
            check_pr_base_with(Some(temp.path()), 12, Some("acme/widget"), &head, &gh).unwrap_err();
        assert!(
            err.contains("names no base") && err.contains("never guess the default branch"),
            "{err}"
        );

        // Empty base
        let temp2 = tempfile::tempdir().unwrap();
        make_brief(temp2.path(), "   ");
        let err2 = check_pr_base_with(Some(temp2.path()), 12, Some("acme/widget"), &head, &gh)
            .unwrap_err();
        assert!(
            err2.contains("names no base") && err2.contains("never guess the default branch"),
            "{err2}"
        );

        // Missing base branch line entirely
        let temp3 = tempfile::tempdir().unwrap();
        let claude = temp3.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(claude.join("task-brief.md"), "- Task: 1\n").unwrap();
        let err3 = check_pr_base_with(Some(temp3.path()), 12, Some("acme/widget"), &head, &gh)
            .unwrap_err();
        assert!(
            err3.contains("names no base") && err3.contains("never guess the default branch"),
            "{err3}"
        );
    }

    #[test]
    fn a_pr_on_another_branch_is_refused_before_any_edit() {
        let temp = tempfile::tempdir().unwrap();
        make_brief(temp.path(), "origin/release/1.2");
        let gh = |args: &[&str], _deadline: Instant| -> Result<GhRun, String> {
            assert_eq!(args[1], "view", "only a look is allowed: {args:?}");
            Ok(GhRun {
                ok: true,
                stdout: r#"{"headRefName": "someone/else", "baseRefName": "main", "url": "u"}"#
                    .to_string(),
                stderr: String::new(),
            })
        };
        let err =
            check_pr_base_with(Some(temp.path()), 12, Some("acme/widget"), &head, &gh).unwrap_err();
        assert!(err.contains("not this worktree's branch"), "{err}");
    }

    #[test]
    fn a_failed_look_after_the_edit_says_the_edit_was_sent() {
        let temp = tempfile::tempdir().unwrap();
        make_brief(temp.path(), "release/1.2");
        let calls = AtomicUsize::new(0);
        let gh = |_args: &[&str], _deadline: Instant| -> Result<GhRun, String> {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            Ok(match n {
                0 => GhRun {
                    ok: true,
                    stdout: r#"{"headRefName": "syarihu/x", "baseRefName": "main", "url": "u"}"#
                        .to_string(),
                    stderr: String::new(),
                },
                1 => GhRun {
                    ok: true,
                    stdout: String::new(),
                    stderr: String::new(),
                },
                _ => GhRun {
                    ok: false,
                    stdout: String::new(),
                    stderr: "HTTP 502".to_string(),
                },
            })
        };
        let err =
            check_pr_base_with(Some(temp.path()), 12, Some("acme/widget"), &head, &gh).unwrap_err();
        assert!(
            err.contains("moved PR #12 to release/1.2 but could not confirm it")
                && err.contains("HTTP 502"),
            "{err}"
        );
    }

    #[test]
    fn a_bare_remote_names_no_branch() {
        let temp = tempfile::tempdir().unwrap();
        make_brief(temp.path(), "origin/");
        let gh = |_args: &[&str], _deadline: Instant| -> Result<GhRun, String> {
            panic!("gh should not be called when the base names no branch");
        };
        let err =
            check_pr_base_with(Some(temp.path()), 12, Some("acme/widget"), &head, &gh).unwrap_err();
        assert!(err.contains("names no branch"), "{err}");
    }

    #[test]
    fn origin_stripping_logic() {
        assert_eq!(strip_origin("origin/main"), "main");
        assert_eq!(strip_origin("origin/release/1.2"), "release/1.2");
        assert_eq!(strip_origin("main"), "main");
        assert_eq!(strip_origin("release/1.2"), "release/1.2");
    }
}
