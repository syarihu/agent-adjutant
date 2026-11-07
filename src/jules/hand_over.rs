//! Handing an approved Jules plan over: starting the session and removing the planning worktree.
//!
//! The planning worktree was cut only for the planning sub-agent to read the repository and
//! write its design document into `.claude/`. Once the plan is approved on the board, this
//! operation verifies that the worktree was kept clean and detached, starts the session with
//! the approved plan, removes the worktree, and runs the repository's `onWorktreeRemove` hooks.
//!
//! With `discard: true`, the worktree is removed without starting a session (for a rejected plan).

use std::path::Path;

use serde::Serialize;

use crate::infra::git::git;
use crate::kernel::worktree_hooks::{HookRun, run_remove_hooks};
use crate::registry::Context;
use crate::task::{self, Status};

use super::Session;

/// Result of handing a task over to Jules or discarding its planning worktree.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HandedOver {
    pub task: String,
    pub session: Option<Session>,
    pub worktree: String,
    pub worktree_removed: bool,
    pub remove_error: Option<String>,
    pub hooks: Vec<HookRun>,
}

/// Extract dirty paths from `git status --porcelain` output.
///
/// When `ignore_plan_files` is true, `.claude/jules-plan.md` and `.claude/jules-gate.json` are
/// ignored. Any other path (including `.claude/other`) is considered an unexpected change.
pub fn dirty_worktree_paths(porcelain: &str, ignore_plan_files: bool) -> Vec<String> {
    let mut paths = Vec::new();
    for line in porcelain.lines() {
        let line = line.trim_end();
        if line.len() < 4 {
            continue;
        }
        let raw_path = &line[3..];
        let path = if let Some((_, to)) = raw_path.split_once(" -> ") {
            to.trim()
        } else {
            raw_path.trim()
        };
        let path = path.trim_matches('"');
        if ignore_plan_files
            && (path == ".claude/jules-plan.md" || path == ".claude/jules-gate.json")
        {
            continue;
        }
        paths.push(path.to_string());
    }
    paths
}

/// Check that `worktree` is detached and clean.
fn check_worktree(worktree: &str, ignore_plan_files: bool) -> Result<(), String> {
    // 1. Detached check: symbolic-ref must fail
    let sym = git(&["symbolic-ref", "-q", "HEAD"], Some(Path::new(worktree)))?;
    if sym.status.success() {
        let branch = String::from_utf8_lossy(&sym.stdout)
            .trim()
            .trim_start_matches("refs/heads/")
            .to_string();
        return Err(format!(
            "{worktree} is on branch {branch}: it is not a planning worktree; nothing was started"
        ));
    }

    // 2. Clean check: status --porcelain --untracked-files=all
    let status = git(
        &["status", "--porcelain", "--untracked-files=all"],
        Some(Path::new(worktree)),
    )?;
    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr).trim().to_string();
        return Err(format!("git status in {worktree} failed: {stderr}"));
    }
    let stdout = String::from_utf8_lossy(&status.stdout);
    let dirty = dirty_worktree_paths(&stdout, ignore_plan_files);
    if !dirty.is_empty() {
        return Err(format!(
            "{worktree} has changes: {}; nothing was started",
            dirty.join(", ")
        ));
    }

    Ok(())
}

/// Hand over a task with an approved plan to Jules, or discard its planning worktree.
pub fn hand_over(ctx: &Context, id: &str, discard: bool) -> Result<HandedOver, String> {
    // 1. Read the record and validate
    let t = task::get(&ctx.state, &ctx.repo.slug, id)?;
    if t.executor != task::Executor::Jules {
        return Err(format!(
            "{} is to be implemented by its worker; `adj task update --id {} --executor jules` first",
            t.id, t.id
        ));
    }
    if t.status != Status::Dispatched {
        return Err(format!(
            "{} is {}, not in progress: a task goes to Jules once the hub has dispatched it and its plan is approved",
            t.id,
            t.status.as_str()
        ));
    }
    if let Some(session) = &t.jules_session {
        return Err(format!(
            "{} is already with Jules (session {session}); clear it with `adj task update --id {} --jules-session ''` to start another",
            t.id, t.id
        ));
    }
    let Some(worktree) = t.worktree.clone().filter(|s| !s.is_empty()) else {
        return Err(format!(
            "{id} has no worktree recorded; write one with `adj task update --id {id} --worktree <path>`"
        ));
    };
    if !Path::new(&worktree).is_dir() {
        return Err(format!(
            "worktree {worktree} does not exist or is not a directory"
        ));
    }

    // 2. Check the worktree before anything is started
    check_worktree(&worktree, true)?;

    // 3. Unless discard: read plan and start session
    let session = if !discard {
        let plan_path = Path::new(&worktree).join(".claude/jules-plan.md");
        let prompt = std::fs::read_to_string(&plan_path)
            .map_err(|_| format!("no plan at {worktree}/.claude/jules-plan.md"))?;
        if prompt.trim().is_empty() {
            return Err(format!("no plan at {worktree}/.claude/jules-plan.md"));
        }
        let s = super::start(ctx, id, &prompt, None)?;
        Some(s)
    } else {
        None
    };

    // 4. Delete the two plan files
    let _ = std::fs::remove_file(Path::new(&worktree).join(".claude/jules-plan.md"));
    let _ = std::fs::remove_file(Path::new(&worktree).join(".claude/jules-gate.json"));

    // Run the two checks of step 2 again with nothing ignored
    let remove_check = check_worktree(&worktree, false);
    if let Err(why) = remove_check {
        return Ok(HandedOver {
            task: id.to_string(),
            session,
            worktree,
            worktree_removed: false,
            remove_error: Some(why),
            hooks: Vec::new(),
        });
    }

    // Remove the worktree without force
    let rm = git(
        &["worktree", "remove", &worktree],
        Some(Path::new(&ctx.repo.main)),
    );
    match rm {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Ok(HandedOver {
                task: id.to_string(),
                session,
                worktree,
                worktree_removed: false,
                remove_error: Some(format!("git worktree remove failed: {stderr}")),
                hooks: Vec::new(),
            });
        }
        Err(e) => {
            return Ok(HandedOver {
                task: id.to_string(),
                session,
                worktree,
                worktree_removed: false,
                remove_error: Some(format!("git worktree remove failed: {e}")),
                hooks: Vec::new(),
            });
        }
    }

    // Clear the record's worktree
    let patch = task::TaskPatch {
        worktree: Some(None),
        ..Default::default()
    };
    let update_res = task::update(ctx, id, &patch, false);
    let remove_error = match update_res {
        Ok(_) => None,
        Err(e) => Some(format!(
            "worktree removed but the record could not be updated: {e}"
        )),
    };

    // Run onWorktreeRemove hooks
    let name = Path::new(&worktree)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let hooks = run_remove_hooks(&ctx.repo.nwo, &ctx.repo.main, &worktree, &name);

    Ok(HandedOver {
        task: id.to_string(),
        session,
        worktree,
        worktree_removed: true,
        remove_error,
        hooks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_filter_ignores_exactly_the_two_plan_files() {
        let status = "\
?? .claude/jules-plan.md
?? .claude/jules-gate.json
?? .claude/other
 M src/main.rs
";
        let ignored = dirty_worktree_paths(status, true);
        assert_eq!(ignored, vec![".claude/other", "src/main.rs"]);

        let all = dirty_worktree_paths(status, false);
        assert_eq!(
            all,
            vec![
                ".claude/jules-plan.md",
                ".claude/jules-gate.json",
                ".claude/other",
                "src/main.rs",
            ]
        );
    }
}
