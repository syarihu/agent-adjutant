//! `adj pr` — pull request operations.

use std::path::Path;

use super::args::PrCheckBaseArgs;
use crate::task::check_pr_base;

pub fn check_base(args: &PrCheckBaseArgs) -> Result<(), String> {
    let worktree = args.worktree.as_deref().map(Path::new);
    let outcome = check_pr_base(worktree, args.pr, args.repo.as_deref())?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string(&outcome).map_err(|e| e.to_string())?
        );
    } else if outcome.changed {
        println!(
            "PR #{} base moved from {} to {}",
            outcome.pr, outcome.was, outcome.base
        );
    } else {
        println!("PR #{} base is {}", outcome.pr, outcome.base);
    }
    Ok(())
}
