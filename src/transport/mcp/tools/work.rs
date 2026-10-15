use serde_json::{Value, json};

use super::text_param;

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_work",
        "description": "What 「いまの仕事」 shows, for every repository the resident server serves: per repository the hubs, parents, session rows with their task and state, and `turns` (open gates waiting on the person, each with the `slug` of its board). Read only. This is the machine-wide view, not the repository of any directory, and it needs the resident server running (`adj server start`). Pass a gate's `slug` and `id` to adjutant_gate_read or adjutant_show.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "repo": { "type": "string", "description": "owner/name. Leave it out for every repository; with it, only that repository's part is returned." },
            },
        },
    })
}

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let repo = text_param(args, "repo");
    let root = crate::registry::state_root(None);
    let mut work = crate::board::ask_resident(&root, "/api/work")?;
    if let Some(repo) = repo {
        keep_repo(&mut work, repo)?;
    }
    Ok(work)
}

/// Only the repository `wanted` names, ignoring case as GitHub does; one the resident does not
/// serve is an error that names the ones it does.
fn keep_repo(work: &mut Value, wanted: &str) -> Result<(), String> {
    let repos = work["repos"].as_array().cloned().unwrap_or_default();
    let (kept, others): (Vec<Value>, Vec<Value>) = repos.into_iter().partition(|r| {
        r["nwo"]
            .as_str()
            .is_some_and(|nwo| nwo.eq_ignore_ascii_case(wanted))
    });
    if kept.is_empty() {
        let served: Vec<&str> = others.iter().filter_map(|r| r["nwo"].as_str()).collect();
        return Err(format!(
            "no such repository on the board: {wanted} (served: {})",
            served.join(", ")
        ));
    }
    work["repos"] = Value::Array(kept);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_repository_is_kept_by_name_ignoring_case_and_an_unknown_one_names_the_rest() {
        let doc = json!({"now": 1, "repos": [{"nwo": "acme/widget"}, {"nwo": "acme/other"}]});
        let mut only = doc.clone();
        keep_repo(&mut only, "Acme/Widget").unwrap();
        assert_eq!(only, json!({"now": 1, "repos": [{"nwo": "acme/widget"}]}));
        let err = keep_repo(&mut doc.clone(), "acme/nope").unwrap_err();
        assert!(err.contains("acme/widget, acme/other"), "{err}");
    }
}
