use std::collections::HashSet;
use std::time::Instant;

use serde_json::Value;

use crate::infra::env::deadline;
use crate::kernel::config::DEFAULT_WORKTREE_NAME;
use crate::kernel::identity::{DEFAULT_BRANCH_PATTERN, branch_fallback};

use super::{
    CollectGh, CollectInput, Collection, Failed, LIST_LIMIT, LeftOut, NODES_PER_CALL, NODES_QUERY,
    ParentRow, Row, SUB_ISSUES_JQ, SubtaskRow, TIMEOUT, WorktreeListing, WorktreeRow, error_text,
    extract_project_item_and_status, first_error, key_for_repo, record_left_out, single_line,
};

/// GitHub secondary rate limits trigger on bursts of concurrent requests; 8 parallel
/// lookups stays well below that threshold while cutting the total wait time.
const MAX_CONCURRENT_PR_LOOKUPS: usize = 8;

pub(super) fn collect_parent(
    input: &CollectInput<'_>,
    parent_key: &str,
    gh: &CollectGh<'_>,
    list_worktrees: &dyn Fn() -> WorktreeListing,
) -> Result<Collection, String> {
    let (chosen_source, parent_r, parent_key_spelled, parent_n) = choose_source(input, parent_key)?;

    let chosen_type = chosen_source
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("");
    if chosen_type == "github-project"
        && chosen_source
            .get("projectNumber")
            .filter(|v| !v.is_null())
            .is_none()
    {
        return Err(format!(
            "{parent_key}'s task source (github-project) has no projectNumber in the config"
        ));
    }

    let parent_repo = parent_r;
    let parent_full_key = format!("{parent_key_spelled}-{parent_n}");

    // P1: parent
    let p1_args = [
        "api",
        &format!("repos/{parent_repo}/issues/{parent_n}"),
        "--jq",
        "{title, html_url, node_id}",
    ];
    let p1_res = gh(&p1_args, None, Instant::now() + deadline(TIMEOUT));
    let (p1_title, p1_url) = match &p1_res {
        Ok(run) if run.ok => {
            let val: Value = serde_json::from_str(&run.stdout).map_err(|_| {
                format!(
                    "cannot read the parent {parent_key} (gh api repos/{parent_repo}/issues/{parent_n}): {}",
                    error_text(&p1_res, Some("invalid JSON"))
                )
            })?;
            let title = val
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "cannot read the parent {parent_key} (gh api repos/{parent_repo}/issues/{parent_n}): missing title"
                    )
                })?
                .to_string();
            let url = val
                .get("html_url")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "cannot read the parent {parent_key} (gh api repos/{parent_repo}/issues/{parent_n}): missing html_url"
                    )
                })?
                .to_string();
            (title, url)
        }
        _ => {
            return Err(format!(
                "cannot read the parent {parent_key} (gh api repos/{parent_repo}/issues/{parent_n}): {}",
                error_text(&p1_res, None)
            ));
        }
    };

    // P2: who-am-I
    let p2_args = ["api", "user", "--jq", ".login"];
    let p2_res = gh(&p2_args, None, Instant::now() + deadline(TIMEOUT));
    let me_login = match &p2_res {
        Ok(run) if run.ok => {
            let login = run.stdout.trim().to_string();
            if login.is_empty() {
                return Err(format!(
                    "cannot tell who you are (gh api user): {}",
                    error_text(&p2_res, Some("empty login"))
                ));
            }
            login
        }
        _ => {
            return Err(format!(
                "cannot tell who you are (gh api user): {}",
                error_text(&p2_res, None)
            ));
        }
    };

    // P3: sub-issues
    let p3_args = [
        "api",
        "--paginate",
        &format!("repos/{parent_repo}/issues/{parent_n}/sub_issues"),
        "--jq",
        SUB_ISSUES_JQ,
    ];
    let p3_res = gh(&p3_args, None, Instant::now() + deadline(TIMEOUT));
    let raw_sub_issues = match &p3_res {
        Ok(run) if run.ok => {
            let mut list = Vec::new();
            for line in run.stdout.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let val: Value = serde_json::from_str(trimmed).map_err(|e| {
                    format!(
                        "cannot read the sub-issues of {parent_key}: {}",
                        single_line(&e.to_string())
                    )
                })?;
                let number = val.get("number").and_then(Value::as_u64);
                let title = val.get("title").and_then(Value::as_str);
                let state = val.get("state").and_then(Value::as_str);
                let html_url = val.get("html_url").and_then(Value::as_str);
                let node_id = val.get("node_id").and_then(Value::as_str);
                let repo = val.get("repo").and_then(Value::as_str);
                let labels = val.get("labels").and_then(Value::as_array);
                let assignees = val.get("assignees").and_then(Value::as_array);

                if let (
                    Some(number),
                    Some(title),
                    Some(state),
                    Some(html_url),
                    Some(node_id),
                    Some(repo),
                ) = (number, title, state, html_url, node_id, repo)
                {
                    let label_names: Vec<String> = labels
                        .into_iter()
                        .flatten()
                        .filter_map(|l| l.as_str().map(str::to_string))
                        .collect();
                    let assignee_logins: Vec<String> = assignees
                        .into_iter()
                        .flatten()
                        .filter_map(|a| a.as_str().map(str::to_string))
                        .collect();
                    list.push(RawChildIssue {
                        number,
                        title: title.to_string(),
                        state: state.to_string(),
                        html_url: html_url.to_string(),
                        node_id: node_id.to_string(),
                        repo: repo.to_string(),
                        labels: label_names,
                        assignees: assignee_logins,
                    });
                } else {
                    return Err(format!(
                        "cannot read the sub-issues of {parent_key}: missing required field"
                    ));
                }
            }
            list
        }
        _ => {
            return Err(format!(
                "cannot read the sub-issues of {parent_key}: {}",
                error_text(&p3_res, None)
            ));
        }
    };

    let mut left_out: Vec<LeftOut> = Vec::new();
    let mut seen_left_out: HashSet<String> = HashSet::new();
    let mut keyed_children: Vec<KeyedChild> = Vec::new();

    for child in raw_sub_issues {
        let identifier = format!("{}#{}", child.repo, child.number);
        match key_for_repo(input.issue_keys, &child.repo) {
            Some(key_spelled) => {
                let full_key = format!("{key_spelled}-{}", child.number);
                keyed_children.push(KeyedChild {
                    raw: child,
                    identifier,
                    key: full_key,
                    key_spelled: key_spelled.to_string(),
                });
            }
            None => {
                record_left_out(&mut left_out, &mut seen_left_out, &identifier, &child.repo);
            }
        }
    }

    let mut failed: Vec<Failed> = Vec::new();

    // P4: nodes(ids:) if github-project
    let chosen_type = chosen_source
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("");
    let is_project = chosen_type == "github-project";

    let mut child_node_status_item: std::collections::HashMap<
        String,
        Result<(String, String), Option<String>>,
    > = std::collections::HashMap::new();

    if is_project && !keyed_children.is_empty() {
        let project_num_val = chosen_source.get("projectNumber").unwrap();
        let source_label = source_label(chosen_source);

        let mut unique_node_ids = Vec::new();
        for child in &keyed_children {
            if !unique_node_ids.contains(&child.raw.node_id.as_str()) {
                unique_node_ids.push(child.raw.node_id.as_str());
            }
        }

        for chunk in unique_node_ids.chunks(NODES_PER_CALL) {
            let chunk_ids: Vec<Value> = chunk
                .iter()
                .map(|&id| Value::String(id.to_string()))
                .collect();
            let payload = serde_json::json!({
                "query": NODES_QUERY,
                "variables": { "ids": chunk_ids },
            });
            let input_bytes = serde_json::to_vec(&payload).unwrap_or_default();
            let dl = Instant::now() + deadline(TIMEOUT);
            let res = gh(
                &[
                    "api",
                    "graphql",
                    "-H",
                    "Content-Type: application/json",
                    "--input",
                    "-",
                ],
                Some(&input_bytes),
                dl,
            );

            let parsed_nodes = match &res {
                Ok(run) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&run.stdout) {
                        if let Some(nodes_arr) =
                            val.pointer("/data/nodes").and_then(Value::as_array)
                        {
                            Ok((nodes_arr.clone(), run.stdout.clone()))
                        } else {
                            let err_msg = first_error(&run.stdout)
                                .or_else(|| {
                                    let err = run.stderr.trim();
                                    if !err.is_empty() {
                                        Some(single_line(err))
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_else(|| "GitHub answered without data".to_string());
                            Err(err_msg)
                        }
                    } else {
                        Err(error_text(&res, Some("invalid JSON in nodes answer")))
                    }
                }
                Err(_) => Err(error_text(&res, None)),
            };

            match parsed_nodes {
                Ok((nodes_arr, stdout)) => {
                    for (idx, &node_id) in chunk.iter().enumerate() {
                        let node_opt = nodes_arr.get(idx);
                        if let Some(node) = node_opt.filter(|n| !n.is_null()) {
                            let (item, status) =
                                extract_project_item_and_status(node, project_num_val);
                            child_node_status_item.insert(node_id.to_string(), Ok((item, status)));
                        } else {
                            let err =
                                first_error(&stdout).unwrap_or_else(|| "not found".to_string());
                            child_node_status_item.insert(node_id.to_string(), Err(Some(err)));
                        }
                    }
                }
                Err(err_msg) => {
                    failed.push(Failed {
                        what: source_label.clone(),
                        lookup: "nodes(ids:)".to_string(),
                        error: err_msg,
                    });
                    for &node_id in chunk {
                        child_node_status_item.insert(node_id.to_string(), Err(None));
                    }
                }
            }
        }
    }

    // Worktree listing
    let listing = list_worktrees();
    let wt_failed = listing.worktrees.is_err();
    if let Err(e) = &listing.worktrees {
        failed.push(Failed {
            what: "worktrees".to_string(),
            lookup: "worktree listing".to_string(),
            error: single_line(e),
        });
    }

    let pattern = chosen_source
        .get("branchPattern")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BRANCH_PATTERN);
    let worktree_name_tmpl = chosen_source
        .get("worktreeName")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_WORKTREE_NAME);

    let mut matched_worktree_paths: HashSet<String> = HashSet::new();
    struct ChildPrep {
        child: KeyedChild,
        item: String,
        status: String,
        nodes_err: Option<String>,
        assignee: String,
        row_branch: String,
        row_worktree: String,
    }

    let mut prepped_children: Vec<ChildPrep> = Vec::with_capacity(keyed_children.len());

    for child in keyed_children {
        // Status & item
        let is_closed = child.raw.state.eq_ignore_ascii_case("closed");
        let (item, status, nodes_err) = if is_project {
            match child_node_status_item.get(&child.raw.node_id) {
                Some(Ok((it, st))) => {
                    let final_st = if is_closed {
                        "closed".to_string()
                    } else {
                        st.clone()
                    };
                    (it.clone(), final_st, None)
                }
                Some(Err(Some(err))) => {
                    let final_st = if is_closed {
                        "closed".to_string()
                    } else {
                        "?".to_string()
                    };
                    ("?".to_string(), final_st, Some(err.clone()))
                }
                Some(Err(None)) | None => {
                    let final_st = if is_closed {
                        "closed".to_string()
                    } else {
                        "?".to_string()
                    };
                    ("?".to_string(), final_st, None)
                }
            }
        } else {
            let final_st = if is_closed {
                "closed".to_string()
            } else if child.raw.labels.is_empty() {
                "-".to_string()
            } else {
                child.raw.labels.join(", ")
            };
            ("-".to_string(), final_st, None)
        };

        let assignee = if child.raw.assignees.is_empty() {
            "-".to_string()
        } else {
            child.raw.assignees.join(",")
        };

        // Branch & worktree matching
        let by_key = branch_fallback(
            pattern,
            &me_login,
            &format!("{}-{}", child.key_spelled, child.raw.number),
        );
        let expanded_name =
            expand_worktree_name(worktree_name_tmpl, &child.key_spelled, child.raw.number);
        let by_name = branch_fallback(pattern, &me_login, &expanded_name);

        let (primary, other) = if listing.proctor {
            (by_key, by_name)
        } else {
            (by_name, by_key)
        };

        let (row_branch, row_worktree) = if wt_failed {
            (primary, "?".to_string())
        } else {
            let wts = listing.worktrees.as_ref().unwrap();
            let matched = wts
                .iter()
                .find(|wt| wt.branch.as_deref() == Some(&primary))
                .or_else(|| wts.iter().find(|wt| wt.branch.as_deref() == Some(&other)));
            match matched {
                Some(wt) => {
                    matched_worktree_paths.insert(wt.path.clone());
                    (wt.branch.clone().unwrap_or(primary), wt.path.clone())
                }
                None => (primary, "-".to_string()),
            }
        };

        prepped_children.push(ChildPrep {
            child,
            item,
            status,
            nodes_err,
            assignee,
            row_branch,
            row_worktree,
        });
    }

    let pr_results: Vec<(String, String, Option<Failed>)> = if prepped_children.is_empty() {
        Vec::new()
    } else {
        let results_mutex = std::sync::Mutex::new(vec![None; prepped_children.len()]);
        let next_idx = std::sync::atomic::AtomicUsize::new(0);
        let worker_count = prepped_children.len().min(MAX_CONCURRENT_PR_LOOKUPS);

        std::thread::scope(|s| {
            for _ in 0..worker_count {
                s.spawn(|| {
                    loop {
                        let idx = next_idx.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if idx >= prepped_children.len() {
                            break;
                        }
                        let prep = &prepped_children[idx];
                        let res = resolve_child_pr(
                            gh,
                            input.code_repo,
                            &prep.row_branch,
                            &prep.child.key_spelled,
                            prep.child.raw.number,
                            &prep.child.key,
                        );
                        results_mutex.lock().unwrap()[idx] = Some(res);
                    }
                });
            }
        });

        results_mutex
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|opt| opt.expect("every child lookup must produce a result"))
            .collect()
    };

    let mut subtask_rows: Vec<SubtaskRow> = Vec::with_capacity(prepped_children.len());
    for (prep, (pr_val, pr_state_val, pr_failure)) in prepped_children.into_iter().zip(pr_results) {
        if let Some(err) = prep.nodes_err {
            failed.push(Failed {
                what: prep.child.key.clone(),
                lookup: "nodes(ids:)".to_string(),
                error: err,
            });
        }
        if let Some(failure) = pr_failure {
            failed.push(failure);
        }
        subtask_rows.push(SubtaskRow {
            kind: "subtask".to_string(),
            identifier: prep.child.identifier,
            key: prep.child.key,
            item: prep.item,
            status: prep.status,
            assignee: prep.assignee,
            branch: prep.row_branch,
            title: prep.child.raw.title,
            url: prep.child.raw.html_url,
            pr: pr_val,
            pr_state: pr_state_val,
            worktree: prep.row_worktree,
        });
    }

    // Worktree rows and unmatched worktrees
    let (worktree_rows, unmatched_worktrees) = if wt_failed {
        (Vec::new(), Some(Vec::new()))
    } else {
        let wts = listing.worktrees.unwrap();
        let mut rows = Vec::new();
        let mut unmatched = Vec::new();
        for wt in wts {
            let row = WorktreeRow {
                kind: "worktree".to_string(),
                path: wt.path.clone(),
                branch: wt.branch.clone().unwrap_or_else(|| "-".to_string()),
            };
            rows.push(row.clone());
            if !matched_worktree_paths.contains(&wt.path) {
                unmatched.push(row);
            }
        }
        (rows, Some(unmatched))
    };

    let mut rows = Vec::with_capacity(1 + subtask_rows.len() + worktree_rows.len());
    rows.push(Row::Parent(ParentRow {
        kind: "parent".to_string(),
        key: parent_full_key,
        title: p1_title,
        url: p1_url,
    }));
    for st in subtask_rows {
        rows.push(Row::Subtask(st));
    }
    for wt in worktree_rows {
        rows.push(Row::Worktree(wt));
    }

    Ok(Collection {
        mode: "parent".to_string(),
        repo: input.code_repo.to_string(),
        taken_at: input.taken_at.clone(),
        me: Some(me_login),
        rows,
        failed,
        left_out,
        agent_sources: None,
        unmatched_worktrees,
    })
}

struct RawChildIssue {
    number: u64,
    title: String,
    state: String,
    html_url: String,
    node_id: String,
    repo: String,
    labels: Vec<String>,
    assignees: Vec<String>,
}

struct KeyedChild {
    raw: RawChildIssue,
    identifier: String,
    key: String,
    key_spelled: String,
}

pub(crate) fn choose_source<'a>(
    input: &'a CollectInput<'_>,
    parent_key: &str,
) -> Result<(&'a Value, &'a str, &'a str, u64), String> {
    let (k, num_str) = parent_key
        .rsplit_once('-')
        .ok_or_else(|| format!("{parent_key} is not a KEY-number task key"))?;
    if k.is_empty() {
        return Err(format!("{parent_key} is not a KEY-number task key"));
    }
    let n = num_str
        .parse::<u64>()
        .map_err(|_| format!("{parent_key} is not a KEY-number task key"))?;

    let mut matched_repos: Vec<(&str, &str)> = Vec::new();
    for (repo, val) in input.issue_keys {
        if let Some(prefix) = val.as_str().filter(|p| p.eq_ignore_ascii_case(k)) {
            matched_repos.push((repo.as_str(), prefix));
        }
    }

    if matched_repos.len() > 1 {
        let names: Vec<&str> = matched_repos.iter().map(|(r, _)| *r).collect();
        return Err(format!(
            "{parent_key} matches several repositories in issueKeys: {}",
            names.join(", ")
        ));
    }

    let mut candidates: Vec<&Value> = Vec::new();

    if let Some((repo, _)) = matched_repos.first() {
        for source in input.sources {
            let kind = source.get("type").and_then(Value::as_str).unwrap_or("");
            let is_match = kind == "github-project"
                || (kind == "github"
                    && source
                        .get("issueRepo")
                        .and_then(Value::as_str)
                        .is_some_and(|ir| ir.eq_ignore_ascii_case(repo)));
            if is_match {
                candidates.push(source);
            }
        }
    }

    // Matching jira and linear sources
    for source in input.sources {
        let kind = source.get("type").and_then(Value::as_str).unwrap_or("");
        let is_jira_or_linear = (kind == "jira"
            && source
                .pointer("/jira/project")
                .and_then(Value::as_str)
                .is_some_and(|proj| proj.eq_ignore_ascii_case(k)))
            || (kind == "linear"
                && source
                    .pointer("/linear/team")
                    .and_then(Value::as_str)
                    .is_some_and(|team| team.eq_ignore_ascii_case(k)));
        if is_jira_or_linear {
            candidates.push(source);
        }
    }

    if candidates.is_empty() {
        return Err(format!(
            "{parent_key} matches no task source of {}: it is not a parent this repository's hub can collect",
            input.code_repo
        ));
    }

    let chosen = if candidates.len() == 1 {
        candidates[0]
    } else {
        let all_projects = candidates
            .iter()
            .all(|c| c.get("type").and_then(Value::as_str) == Some("github-project"));
        if all_projects {
            let with_pf: Vec<&Value> = candidates
                .iter()
                .copied()
                .filter(|c| c.get("projectFields").is_some_and(Value::is_object))
                .collect();
            if with_pf.len() == 1 {
                with_pf[0]
            } else {
                let labels: Vec<String> = candidates.iter().map(|&c| source_label(c)).collect();
                return Err(format!(
                    "{parent_key} matches several task sources: {}",
                    labels.join(", ")
                ));
            }
        } else {
            let labels: Vec<String> = candidates.iter().map(|&c| source_label(c)).collect();
            return Err(format!(
                "{parent_key} matches several task sources: {}",
                labels.join(", ")
            ));
        }
    };

    let chosen_kind = chosen.get("type").and_then(Value::as_str).unwrap_or("");
    if chosen_kind == "jira" || chosen_kind == "linear" {
        return Err(format!(
            "{parent_key} is a {chosen_kind} task: collect it with the collection agent"
        ));
    }

    let (parent_r, parent_key_spelled) = matched_repos[0];
    Ok((chosen, parent_r, parent_key_spelled, n))
}

fn source_label(source: &Value) -> String {
    let kind = source.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "github" => format!(
            "github {}",
            source
                .get("issueRepo")
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        "github-project" => {
            let owner = source
                .get("projectOwner")
                .and_then(Value::as_str)
                .unwrap_or("");
            let num = source
                .get("projectNumber")
                .map(|v| match v {
                    Value::Number(n) => n.to_string(),
                    Value::String(s) => s.clone(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            format!("github-project {owner}/{num}")
        }
        "jira" => format!(
            "jira {}",
            source
                .pointer("/jira/project")
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        "linear" => format!(
            "linear {}",
            source
                .pointer("/linear/team")
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        _ => kind.to_string(),
    }
}

fn expand_worktree_name(template: &str, key: &str, number: u64) -> String {
    template
        .replace("{issuekey-lowercase}", &key.to_ascii_lowercase())
        .replace("{issueKey}", key)
        .replace("{issue}", &number.to_string())
}

#[derive(Debug, Clone)]
struct ParsedPr {
    number: u64,
    state: String,
}

fn rank_pr_state(state: &str) -> u8 {
    match state {
        "OPEN" => 0,
        "MERGED" => 1,
        "CLOSED" => 2,
        _ => 3,
    }
}

fn pick_best_pr(prs: &[ParsedPr]) -> Option<ParsedPr> {
    prs.iter()
        .min_by_key(|pr| (rank_pr_state(&pr.state), std::cmp::Reverse(pr.number)))
        .cloned()
}

fn parse_pr_list(stdout: &str) -> Option<Vec<ParsedPr>> {
    let val: Value = serde_json::from_str(stdout).ok()?;
    let arr = val.as_array()?;
    let mut result = Vec::new();
    for item in arr {
        let number = item.get("number").and_then(Value::as_u64)?;
        let state = item.get("state").and_then(Value::as_str)?.to_string();
        result.push(ParsedPr { number, state });
    }
    Some(result)
}

struct SearchPr {
    number: u64,
    state: String,
    title: String,
    body: String,
    head_ref_name: String,
}

/// Returns whether `text` contains `key` as a whole token, compared ASCII case-insensitively.
///
/// GitHub search tokenizes the query string and may return PRs that do not actually name
/// the key (for example, matching an unrelated PR where separate tokens or numbers coincidentally hit).
/// To prevent misattributing an unrelated PR to a subtask, we check that `key` appears in the
/// title, body, or headRefName as a whole token: the match must not be preceded by an ASCII
/// alphanumeric character and must not be followed by an ASCII digit (so e.g. `ALPHA-11` never matches
/// `ALPHA-119`, and `ALPHA-119` preceded by a letter does not match).
fn contains_key_token(text: &str, key: &str) -> bool {
    let text_bytes = text.as_bytes();
    let key_bytes = key.as_bytes();
    if key_bytes.is_empty() || text_bytes.len() < key_bytes.len() {
        return false;
    }
    for i in 0..=(text_bytes.len() - key_bytes.len()) {
        let candidate = &text_bytes[i..i + key_bytes.len()];
        if candidate.eq_ignore_ascii_case(key_bytes) {
            let preceded_by_alphanumeric = i > 0 && text_bytes[i - 1].is_ascii_alphanumeric();
            let followed_by_digit = (i + key_bytes.len() < text_bytes.len())
                && text_bytes[i + key_bytes.len()].is_ascii_digit();
            if !preceded_by_alphanumeric && !followed_by_digit {
                return true;
            }
        }
    }
    false
}

fn parse_search_prs(stdout: &str) -> Option<Vec<SearchPr>> {
    let val: Value = serde_json::from_str(stdout).ok()?;
    let arr = val.as_array()?;
    let mut result = Vec::new();
    for item in arr {
        let number = item.get("number").and_then(Value::as_u64)?;
        let state = item.get("state").and_then(Value::as_str)?.to_string();
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let body = item
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let head_ref_name = item
            .get("headRefName")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        result.push(SearchPr {
            number,
            state,
            title,
            body,
            head_ref_name,
        });
    }
    Some(result)
}

fn resolve_child_pr(
    gh: &CollectGh<'_>,
    code_repo: &str,
    branch: &str,
    key_spelled: &str,
    number: u64,
    child_key: &str,
) -> (String, String, Option<Failed>) {
    let p5_args = [
        "pr",
        "list",
        "-R",
        code_repo,
        "--head",
        branch,
        "--state",
        "all",
        "--limit",
        LIST_LIMIT,
        "--json",
        "number,title,url,state,isDraft",
    ];
    let p5_res = gh(&p5_args, None, Instant::now() + deadline(TIMEOUT));

    let p5_prs = match &p5_res {
        Ok(run) if run.ok => parse_pr_list(&run.stdout),
        _ => None,
    };

    match p5_prs {
        Some(prs) if !prs.is_empty() => {
            let best = pick_best_pr(&prs).unwrap();
            (best.number.to_string(), best.state, None)
        }
        Some(_) => {
            // P5 succeeded and returned [] -> try P6
            let search_term = format!("{key_spelled}-{number}");
            let p6_args = [
                "pr",
                "list",
                "-R",
                code_repo,
                "--search",
                &search_term,
                "--state",
                "all",
                "--json",
                "number,title,url,state,isDraft,body,headRefName",
            ];
            let p6_res = gh(&p6_args, None, Instant::now() + deadline(TIMEOUT));
            let p6_prs = match &p6_res {
                Ok(run) if run.ok => parse_search_prs(&run.stdout),
                _ => None,
            };

            match p6_prs {
                Some(prs) => {
                    let matching_prs: Vec<ParsedPr> = prs
                        .into_iter()
                        .filter(|pr| {
                            contains_key_token(&pr.title, &search_term)
                                || contains_key_token(&pr.body, &search_term)
                                || contains_key_token(&pr.head_ref_name, &search_term)
                        })
                        .map(|pr| ParsedPr {
                            number: pr.number,
                            state: pr.state,
                        })
                        .collect();

                    if let Some(best) = pick_best_pr(&matching_prs) {
                        (best.number.to_string(), best.state, None)
                    } else {
                        ("-".to_string(), "-".to_string(), None)
                    }
                }
                None => {
                    let failure = Failed {
                        what: child_key.to_string(),
                        lookup: "PR by --search".to_string(),
                        error: error_text(&p6_res, None),
                    };
                    ("?".to_string(), "?".to_string(), Some(failure))
                }
            }
        }
        None => {
            let failure = Failed {
                what: child_key.to_string(),
                lookup: "PR by --head".to_string(),
                error: error_text(&p5_res, None),
            };
            ("?".to_string(), "?".to_string(), Some(failure))
        }
    }
}
