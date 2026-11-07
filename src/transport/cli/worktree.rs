//! `adj worktree` — add, list, and remove linked worktrees.

use super::args::{WorktreeAddArgs, WorktreeLsArgs, WorktreeRmArgs};
use crate::board::worktree::{self, AddRequest, RemoveRequest};
use crate::registry::context_without_hub;

pub fn add(args: &WorktreeAddArgs) -> Result<(), String> {
    let ctx = context_without_hub(args.repo.as_deref())?;
    let outcome = worktree::add(
        &ctx,
        AddRequest {
            name: args.name.clone(),
            user: args.user.clone(),
            pattern: args.pattern.clone(),
            branch: args.branch.clone(),
            path: args.path.clone(),
            base: args.base.clone(),
        },
    )?;
    let rendered = serde_json::to_string_pretty(&outcome)
        .map_err(|e| format!("cannot serialize outcome: {e}"))?;
    println!("{rendered}");
    Ok(())
}

pub fn ls(args: &WorktreeLsArgs) -> Result<(), String> {
    let ctx = context_without_hub(args.repo.as_deref())?;
    let outcome = worktree::list(&ctx)?;
    let rendered = serde_json::to_string_pretty(&outcome)
        .map_err(|e| format!("cannot serialize outcome: {e}"))?;
    println!("{rendered}");
    Ok(())
}

pub fn rm(args: &WorktreeRmArgs) -> Result<i32, String> {
    let ctx = context_without_hub(args.repo.as_deref())?;
    let outcome = worktree::remove(
        &ctx,
        RemoveRequest {
            path: args.path.clone(),
            branch: args.branch.clone(),
            cwd: None,
        },
    )?;
    let rendered = serde_json::to_string_pretty(&outcome)
        .map_err(|e| format!("cannot serialize outcome: {e}"))?;
    println!("{rendered}");
    if outcome.is_removed() { Ok(0) } else { Ok(1) }
}
