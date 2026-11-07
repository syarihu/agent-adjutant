//! `adj collect` — collect task rows from github and github-project sources.

use super::args::CollectArgs;
use crate::task::collect;

pub(crate) fn run(args: &CollectArgs) -> Result<(), String> {
    let parent = args
        .parent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let repo = args
        .repo
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let collection = collect(None, repo, parent)?;
    if args.json {
        let json_str = serde_json::to_string(&collection)
            .map_err(|e| format!("cannot serialize result: {e}"))?;
        println!("{json_str}");
    } else {
        println!("Taken at: {}", collection.taken_at);
        for row in &collection.rows {
            let line =
                serde_json::to_string(row).map_err(|e| format!("cannot serialize row: {e}"))?;
            println!("{line}");
        }
        if !collection.failed.is_empty() {
            let failed_items: Vec<String> = collection
                .failed
                .iter()
                .map(|f| format!("{} ({}: {})", f.what, f.lookup, f.error))
                .collect();
            println!("Lookups that failed: {}", failed_items.join(", "));
        }
        if !collection.left_out.is_empty() {
            let left_out_items: Vec<String> = collection
                .left_out
                .iter()
                .map(|l| format!("{} ×{}", l.repo, l.count))
                .collect();
            println!("no key configured, left out: {}", left_out_items.join(", "));
        }
        if let Some(agent_sources) = &collection.agent_sources
            && !agent_sources.is_empty()
        {
            let agent_items: Vec<String> = agent_sources
                .iter()
                .map(|source| {
                    let kind = source.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    let key = source
                        .get("jira")
                        .and_then(|j| j.get("project"))
                        .or_else(|| source.get("linear").and_then(|l| l.get("team")))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if key.is_empty() {
                        kind.to_string()
                    } else {
                        format!("{kind} {key}")
                    }
                })
                .collect();
            println!("For the collection agent: {}", agent_items.join(", "));
        }
        if let Some(unmatched) = &collection.unmatched_worktrees
            && !unmatched.is_empty()
        {
            let unmatched_items: Vec<String> = unmatched
                .iter()
                .map(|w| format!("{} ({})", w.path, w.branch))
                .collect();
            println!(
                "Worktrees that matched no subtask: {}",
                unmatched_items.join(", ")
            );
        }
    }
    Ok(())
}
