//! `adj collect` / `adjutant_collect` — collect task rows from github and github-project sources.

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::infra::clock::{now_secs, utc_iso};
use crate::infra::env::deadline;
use crate::infra::gh::{GhRun, run_with_input};
use crate::infra::shell::on_path;
use crate::kernel::config;
use crate::kernel::identity::{self, Worktree};

pub(super) use super::github::first_error;

mod parent;
#[cfg(test)]
pub(crate) use parent::choose_source;
use parent::collect_parent;

#[cfg(test)]
mod tests;

pub(super) const SUB_ISSUES_JQ: &str = r#".[] | {number, title, state, html_url, node_id, repo: (.repository_url | sub(".*/repos/"; "")), labels: [.labels[].name], assignees: [.assignees[].login]}"#;

/// How long one `gh` operation may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Keep the recipe's `--limit 50`: a truncated list looks like a short one, `--limit` is the only lever.
const LIST_LIMIT: &str = "50";

/// GitHub's cap on `nodes(ids:)`.
const NODES_PER_CALL: usize = 100;

/// GraphQL query for batch status and item resolution.
const NODES_QUERY: &str = "query($ids: [ID!]!) { nodes(ids: $ids) { ... on Issue { id number title url state \
repository { nameWithOwner } \
projectItems(first: 50) { pageInfo { hasNextPage } nodes { id project { number owner { ... on User { login } ... on Organization { login } } } \
fieldValueByName(name: \"Status\") { ... on ProjectV2ItemFieldSingleSelectValue { name } } } } } } }";

/// Runs `gh` with these arguments, and this on stdin when given, by this deadline:
/// `crate::infra::gh::run_with_input`, or a stand-in in tests.
pub type CollectGh<'a> =
    dyn Fn(&[&str], Option<&[u8]>, Instant) -> Result<GhRun, String> + Sync + 'a;

/// What the worktree listing came to, and whether proctor answered it.
pub struct WorktreeListing {
    /// proctor is installed (`on_path("proctor")`), whether or not its answer could be read.
    pub proctor: bool,
    /// The linked worktrees, the main checkout already left out by its marker; `Err` with the
    /// reason when the listing failed.
    pub worktrees: Result<Vec<Worktree>, String>,
}

pub struct CollectInput<'a> {
    /// `owner/name` of the code repository (PRs are looked for here).
    pub code_repo: &'a str,
    pub issue_keys: &'a serde_json::Map<String, Value>,
    pub sources: &'a [Value],
    /// The parent task's key, trimmed; `None` for the Dashboard.
    pub parent: Option<&'a str>,
    /// `YYYY-MM-DDTHH:MM:SSZ`, taken before anything was fetched.
    pub taken_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    /// `dashboard` or `parent`.
    pub mode: String,
    pub repo: String,
    pub taken_at: String,
    /// The gh login (parent mode only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub me: Option<String>,
    pub rows: Vec<Row>,
    pub failed: Vec<Failed>,
    pub left_out: Vec<LeftOut>,
    /// The taskSources entries that are not github / github-project, as the config holds them
    /// (Dashboard only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_sources: Option<Vec<Value>>,
    /// Worktrees whose branch matched no subtask (parent mode only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmatched_worktrees: Option<Vec<WorktreeRow>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Row {
    Task(TaskRow),
    Worktree(WorktreeRow),
    Pr(PrRow),
    Parent(ParentRow),
    Subtask(SubtaskRow),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRow {
    pub kind: String,
    pub identifier: String,
    pub key: String,
    pub item: String,
    pub status: String,
    pub title: String,
    pub url: String,
    pub assignee: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRow {
    pub kind: String,
    pub path: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrRow {
    pub kind: String,
    pub number: String,
    pub title: String,
    pub url: String,
    pub draft: String,
    pub ci: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentRow {
    pub kind: String,
    pub key: String,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtaskRow {
    pub kind: String,
    pub identifier: String,
    pub key: String,
    pub item: String,
    pub status: String,
    pub assignee: String,
    pub branch: String,
    pub title: String,
    pub url: String,
    pub pr: String,
    pub pr_state: String,
    pub worktree: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failed {
    pub what: String,
    pub lookup: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeftOut {
    pub repo: String,
    pub count: u64,
}

/// Collect the Dashboard's rows, or with `parent` what sits under that parent task, from the
/// repository's github and github-project task sources.
pub fn collect(
    cwd: Option<&Path>,
    repo: Option<&str>,
    parent: Option<&str>,
) -> Result<Collection, String> {
    let taken_at = utc_iso(now_secs());
    let clean_repo = repo.map(str::trim).filter(|s| !s.is_empty());
    let info = identity::resolve_in(cwd, clean_repo, None)?;
    let resolved = config::resolve_config(&info.nwo)?;
    if !resolved.registered || resolved.settings.task_sources.is_empty() {
        return Err(format!(
            "{} has no task sources in the config: nothing to collect",
            info.nwo
        ));
    }
    let input = CollectInput {
        code_repo: &info.nwo,
        issue_keys: &resolved.settings.issue_keys,
        sources: &resolved.settings.task_sources,
        parent: parent.map(str::trim).filter(|s| !s.is_empty()),
        taken_at,
    };
    collect_with(
        &input,
        &|args, input_bytes, dl| run_with_input(None, args, input_bytes, dl),
        &|| list_worktrees(&info.main),
    )
}

/// Same, with `gh` and the worktree listing handed in so tests need neither.
pub fn collect_with(
    input: &CollectInput<'_>,
    gh: &CollectGh<'_>,
    list_worktrees: &dyn Fn() -> WorktreeListing,
) -> Result<Collection, String> {
    if let Some(parent_key) = input.parent {
        collect_parent(input, parent_key, gh, list_worktrees)
    } else {
        collect_dashboard(input, gh, list_worktrees)
    }
}

fn list_worktrees(main: &str) -> WorktreeListing {
    let proctor = on_path("proctor");
    if proctor {
        let worktrees = run_proctor_worktree_ls(main, Instant::now() + deadline(TIMEOUT))
            .and_then(|stdout| parse_proctor_worktrees(main, &stdout));
        WorktreeListing { proctor, worktrees }
    } else {
        let worktrees = identity::worktrees(main).map(|wts| wts.into_iter().skip(1).collect());
        WorktreeListing { proctor, worktrees }
    }
}

fn run_proctor_worktree_ls(main: &str, deadline: Instant) -> Result<String, String> {
    let mut command = std::process::Command::new("proctor");
    command
        .args(["worktree", "ls", "--json"])
        .current_dir(main)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for name in crate::infra::git::REPOSITORY_LOCATION_ENV {
        command.env_remove(name);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("proctor worktree ls --json failed: {e}"))?;

    fn drain(
        pipe: Option<impl std::io::Read + Send + 'static>,
    ) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = std::io::Read::read_to_end(&mut pipe, &mut bytes);
            }
            let _ = tx.send(bytes);
        });
        rx
    }

    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("proctor worktree ls --json did not finish in time".to_string());
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("cannot wait for proctor: {e}"));
            }
        }
    };

    let collect = |rx: std::sync::mpsc::Receiver<Vec<u8>>| match rx
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        Ok(bytes) => Ok(bytes),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err("proctor worktree ls --json did not finish in time".to_string())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err("cannot read proctor: the reader panicked".to_string())
        }
    };
    let out = collect(out)?;
    let err = collect(err)?;

    if status.success() {
        Ok(String::from_utf8_lossy(&out).into_owned())
    } else {
        Err(format!(
            "proctor worktree ls --json failed: {}",
            single_line(&String::from_utf8_lossy(&err))
        ))
    }
}

pub(crate) fn parse_proctor_worktrees(main: &str, stdout: &str) -> Result<Vec<Worktree>, String> {
    let val: Value =
        serde_json::from_str(stdout).map_err(|e| format!("cannot read proctor's answer: {e}"))?;
    let repos = val
        .as_array()
        .ok_or_else(|| "cannot read proctor's answer: expected array".to_string())?;

    let canon_main = std::fs::canonicalize(main).unwrap_or_else(|_| std::path::PathBuf::from(main));
    let matched_group = if repos.len() == 1 {
        Some(&repos[0])
    } else {
        repos.iter().find(|repo_val| {
            let repo_str = repo_val
                .get("repo")
                .or_else(|| repo_val.get("repository"))
                .and_then(Value::as_str);
            if let Some(r) = repo_str {
                let canon_r =
                    std::fs::canonicalize(r).unwrap_or_else(|_| std::path::PathBuf::from(r));
                canon_r == canon_main || r == main
            } else {
                false
            }
        })
    };
    let repo_val =
        matched_group.ok_or_else(|| format!("proctor's answer has no group for {main}"))?;

    let wts = repo_val
        .get("worktrees")
        .and_then(Value::as_array)
        .ok_or_else(|| "cannot read proctor's answer: missing worktrees".to_string())?;
    let mut result = Vec::new();
    for wt in wts {
        let is_main = wt.get("isMain").and_then(Value::as_bool).unwrap_or(false);
        if is_main {
            continue;
        }
        let path = wt
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "cannot read proctor's answer: missing path".to_string())?
            .to_string();
        let branch = wt
            .get("branch")
            .and_then(Value::as_str)
            .map(|b| b.strip_prefix("refs/heads/").unwrap_or(b).trim())
            .filter(|b| !b.is_empty())
            .map(str::to_string);
        result.push(Worktree { path, branch });
    }
    Ok(result)
}

pub(super) fn single_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn error_text(run_res: &Result<GhRun, String>, parse_err: Option<&str>) -> String {
    match run_res {
        Err(e) => single_line(e),
        Ok(run) => {
            let stderr = run.stderr.trim();
            if !stderr.is_empty() {
                single_line(stderr)
            } else if let Some(pe) = parse_err {
                single_line(pe)
            } else {
                "gh exited with an error".to_string()
            }
        }
    }
}

pub(super) fn key_for_repo<'a>(
    issue_keys: &'a serde_json::Map<String, Value>,
    repo: &str,
) -> Option<&'a str> {
    issue_keys
        .get(repo)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .or_else(|| {
            issue_keys.iter().find_map(|(k, v)| {
                if k.eq_ignore_ascii_case(repo) {
                    v.as_str().filter(|val| !val.is_empty())
                } else {
                    None
                }
            })
        })
}

pub(super) fn record_left_out(
    left_out: &mut Vec<LeftOut>,
    seen_identifiers: &mut HashSet<String>,
    identifier: &str,
    repo_as_spelled: &str,
) {
    if seen_identifiers.insert(identifier.to_ascii_lowercase()) {
        if let Some(entry) = left_out
            .iter_mut()
            .find(|e| e.repo.eq_ignore_ascii_case(repo_as_spelled))
        {
            entry.count += 1;
        } else {
            left_out.push(LeftOut {
                repo: repo_as_spelled.to_string(),
                count: 1,
            });
        }
    }
}

struct RawDashboardTask {
    identifier: String,
    key: String,
    title: String,
    url: String,
    item: String,
    status: String,
    assignee_me: bool,
    has_project_fields: bool,
}

fn collect_dashboard(
    input: &CollectInput<'_>,
    gh: &CollectGh<'_>,
    list_worktrees: &dyn Fn() -> WorktreeListing,
) -> Result<Collection, String> {
    let mut raw_tasks: Vec<RawDashboardTask> = Vec::new();
    let mut failed: Vec<Failed> = Vec::new();
    let mut left_out: Vec<LeftOut> = Vec::new();
    let mut seen_left_out: HashSet<String> = HashSet::new();
    let mut agent_sources: Vec<Value> = Vec::new();

    for source in input.sources {
        let kind = source.get("type").and_then(Value::as_str).unwrap_or("");
        if kind != "github" && kind != "github-project" {
            agent_sources.push(source.clone());
            continue;
        }

        if kind == "github" {
            let issue_repo = match source.get("issueRepo").and_then(Value::as_str) {
                Some(r) => r,
                None => {
                    failed.push(Failed {
                        what: "github".to_string(),
                        lookup: "config".to_string(),
                        error: "missing issueRepo".to_string(),
                    });
                    continue;
                }
            };
            let label = format!("github {issue_repo}");

            // D1: assigned
            read_github_issues(
                gh,
                issue_repo,
                &[
                    "issue",
                    "list",
                    "-R",
                    issue_repo,
                    "--assignee",
                    "@me",
                    "--state",
                    "open",
                    "--json",
                    "number,title,labels,url",
                    "--limit",
                    LIST_LIMIT,
                ],
                true,
                &label,
                "assigned list",
                input.issue_keys,
                &mut raw_tasks,
                &mut failed,
                &mut left_out,
                &mut seen_left_out,
            );

            // D2: unassigned
            read_github_issues(
                gh,
                issue_repo,
                &[
                    "issue",
                    "list",
                    "-R",
                    issue_repo,
                    "--search",
                    "no:assignee",
                    "--state",
                    "open",
                    "--json",
                    "number,title,labels,url",
                    "--limit",
                    LIST_LIMIT,
                ],
                false,
                &label,
                "unassigned list",
                input.issue_keys,
                &mut raw_tasks,
                &mut failed,
                &mut left_out,
                &mut seen_left_out,
            );
        } else if kind == "github-project" {
            let project_owner = source.get("projectOwner").and_then(Value::as_str);
            let project_number_val = source.get("projectNumber");
            let project_number_str = project_number_val.and_then(|v| match v {
                Value::Number(n) => Some(n.to_string()),
                Value::String(s) => Some(s.clone()),
                _ => None,
            });

            let (owner, num_str) = match (project_owner, &project_number_str) {
                (Some(o), Some(n)) => (o, n),
                _ => {
                    let label = match project_owner {
                        Some(o) => format!("github-project {o}/?"),
                        None => "github-project".to_string(),
                    };
                    failed.push(Failed {
                        what: label,
                        lookup: "config".to_string(),
                        error: "missing projectOwner or projectNumber".to_string(),
                    });
                    continue;
                }
            };
            let owner_and_num = format!("{owner}/{num_str}");
            let label = format!("github-project {owner_and_num}");
            let has_pf = source.get("projectFields").is_some_and(Value::is_object);

            let mut project_items = Vec::new();

            // D3: assigned
            let mut d3_succeeded = false;
            read_project_issues(
                gh,
                &[
                    "search",
                    "issues",
                    "--assignee",
                    "@me",
                    "--state",
                    "open",
                    "--project",
                    &owner_and_num,
                    "--json",
                    "id,number,title,url,repository",
                    "--limit",
                    LIST_LIMIT,
                ],
                true,
                &label,
                "assigned list",
                input.issue_keys,
                &mut project_items,
                &mut failed,
                &mut left_out,
                &mut seen_left_out,
                &mut d3_succeeded,
            );

            // D4: unassigned
            let mut d4_succeeded = false;
            read_project_issues(
                gh,
                &[
                    "search",
                    "issues",
                    "no:assignee",
                    "--state",
                    "open",
                    "--project",
                    &owner_and_num,
                    "--json",
                    "id,number,title,url,repository",
                    "--limit",
                    LIST_LIMIT,
                ],
                false,
                &label,
                "unassigned list",
                input.issue_keys,
                &mut project_items,
                &mut failed,
                &mut left_out,
                &mut seen_left_out,
                &mut d4_succeeded,
            );

            if d3_succeeded || d4_succeeded {
                let project_num_val = project_number_val.unwrap();
                resolve_project_nodes(
                    gh,
                    &label,
                    project_num_val,
                    owner,
                    has_pf,
                    project_items,
                    &mut raw_tasks,
                    &mut failed,
                );
            }
        }
    }

    // Dedupe
    let mut deduped_tasks: Vec<TaskRow> = Vec::new();
    let mut seen_indices: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut chosen_has_pf: Vec<bool> = Vec::new();

    for task in raw_tasks {
        let lower_id = task.identifier.to_ascii_lowercase();
        if let Some(&idx) = seen_indices.get(&lower_id) {
            if task.assignee_me {
                deduped_tasks[idx].assignee = "@me".to_string();
            }
            if !chosen_has_pf[idx] && task.has_project_fields {
                deduped_tasks[idx].item = task.item;
                deduped_tasks[idx].status = task.status;
                chosen_has_pf[idx] = true;
            }
        } else {
            let idx = deduped_tasks.len();
            seen_indices.insert(lower_id, idx);
            chosen_has_pf.push(task.has_project_fields);
            deduped_tasks.push(TaskRow {
                kind: "task".to_string(),
                identifier: task.identifier,
                key: task.key,
                item: task.item,
                status: task.status,
                title: task.title,
                url: task.url,
                assignee: if task.assignee_me {
                    "@me".to_string()
                } else {
                    "-".to_string()
                },
            });
        }
    }

    // D6: Own PRs
    let mut pr_rows = Vec::new();
    let pr_deadline = Instant::now() + deadline(TIMEOUT);
    let pr_args = [
        "pr",
        "list",
        "-R",
        input.code_repo,
        "--author",
        "@me",
        "--state",
        "open",
        "--json",
        "number,title,url,isDraft,statusCheckRollup",
        "--limit",
        LIST_LIMIT,
    ];
    let pr_res = gh(&pr_args, None, pr_deadline);
    match pr_res {
        Ok(run) if run.ok => match serde_json::from_str::<Value>(&run.stdout) {
            Ok(Value::Array(prs)) => {
                let mut valid = true;
                for pr in prs {
                    let number = pr.get("number").and_then(Value::as_u64);
                    let title = pr.get("title").and_then(Value::as_str);
                    let url = pr.get("url").and_then(Value::as_str);
                    let is_draft = pr.get("isDraft").and_then(Value::as_bool).unwrap_or(false);
                    let rollup = pr.get("statusCheckRollup");
                    if let (Some(number), Some(title), Some(url)) = (number, title, url) {
                        pr_rows.push(PrRow {
                            kind: "pr".to_string(),
                            number: number.to_string(),
                            title: title.to_string(),
                            url: url.to_string(),
                            draft: if is_draft {
                                "yes".to_string()
                            } else {
                                "no".to_string()
                            },
                            ci: ci_of(rollup),
                        });
                    } else {
                        valid = false;
                        break;
                    }
                }
                if !valid {
                    pr_rows.clear();
                    failed.push(Failed {
                        what: "own PRs".to_string(),
                        lookup: "gh pr list".to_string(),
                        error: "missing required field".to_string(),
                    });
                }
            }
            Ok(_) => {
                failed.push(Failed {
                    what: "own PRs".to_string(),
                    lookup: "gh pr list".to_string(),
                    error: "expected array".to_string(),
                });
            }
            Err(e) => {
                failed.push(Failed {
                    what: "own PRs".to_string(),
                    lookup: "gh pr list".to_string(),
                    error: error_text(&Ok(run), Some(&e.to_string())),
                });
            }
        },
        _ => {
            failed.push(Failed {
                what: "own PRs".to_string(),
                lookup: "gh pr list".to_string(),
                error: error_text(&pr_res, None),
            });
        }
    }

    // Worktrees
    let mut worktree_rows = Vec::new();
    match (list_worktrees)().worktrees {
        Ok(wts) => {
            for wt in wts {
                worktree_rows.push(WorktreeRow {
                    kind: "worktree".to_string(),
                    path: wt.path,
                    branch: wt.branch.unwrap_or_else(|| "-".to_string()),
                });
            }
        }
        Err(e) => {
            failed.push(Failed {
                what: "worktrees".to_string(),
                lookup: "worktree listing".to_string(),
                error: single_line(&e),
            });
        }
    }

    let mut rows = Vec::with_capacity(deduped_tasks.len() + pr_rows.len() + worktree_rows.len());
    for task in deduped_tasks {
        rows.push(Row::Task(task));
    }
    for pr in pr_rows {
        rows.push(Row::Pr(pr));
    }
    for wt in worktree_rows {
        rows.push(Row::Worktree(wt));
    }

    Ok(Collection {
        mode: "dashboard".to_string(),
        repo: input.code_repo.to_string(),
        taken_at: input.taken_at.clone(),
        me: None,
        rows,
        failed,
        left_out,
        agent_sources: Some(agent_sources),
        unmatched_worktrees: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn read_github_issues(
    gh: &CollectGh<'_>,
    issue_repo: &str,
    args: &[&str],
    assignee_me: bool,
    label: &str,
    lookup_name: &str,
    issue_keys: &serde_json::Map<String, Value>,
    raw_tasks: &mut Vec<RawDashboardTask>,
    failed: &mut Vec<Failed>,
    left_out: &mut Vec<LeftOut>,
    seen_left_out: &mut HashSet<String>,
) {
    let dl = Instant::now() + deadline(TIMEOUT);
    let res = gh(args, None, dl);
    match res {
        Ok(run) if run.ok => match serde_json::from_str::<Value>(&run.stdout) {
            Ok(Value::Array(issues)) => {
                let mut valid = true;
                let mut parsed_items = Vec::new();
                for issue in issues {
                    let number = issue.get("number").and_then(Value::as_u64);
                    let title = issue.get("title").and_then(Value::as_str);
                    let url = issue.get("url").and_then(Value::as_str);
                    let labels_arr = issue.get("labels").and_then(Value::as_array);
                    if let (Some(number), Some(title), Some(url), Some(labels)) =
                        (number, title, url, labels_arr)
                    {
                        let label_names: Option<Vec<String>> = labels
                            .iter()
                            .map(|l| l.get("name").and_then(Value::as_str).map(str::to_string))
                            .collect();
                        if let Some(names) = label_names {
                            parsed_items.push((number, title.to_string(), url.to_string(), names));
                        } else {
                            valid = false;
                            break;
                        }
                    } else {
                        valid = false;
                        break;
                    }
                }
                if valid {
                    for (number, title, url, names) in parsed_items {
                        let identifier = format!("{issue_repo}#{number}");
                        match key_for_repo(issue_keys, issue_repo) {
                            Some(k) => {
                                let key = format!("{k}-{number}");
                                let status = if names.is_empty() {
                                    "-".to_string()
                                } else {
                                    names.join(", ")
                                };
                                raw_tasks.push(RawDashboardTask {
                                    identifier,
                                    key,
                                    title,
                                    url,
                                    item: "-".to_string(),
                                    status,
                                    assignee_me,
                                    has_project_fields: false,
                                });
                            }
                            None => {
                                record_left_out(left_out, seen_left_out, &identifier, issue_repo);
                            }
                        }
                    }
                } else {
                    failed.push(Failed {
                        what: label.to_string(),
                        lookup: lookup_name.to_string(),
                        error: "missing required field".to_string(),
                    });
                }
            }
            Ok(_) => {
                failed.push(Failed {
                    what: label.to_string(),
                    lookup: lookup_name.to_string(),
                    error: "expected array".to_string(),
                });
            }
            Err(e) => {
                failed.push(Failed {
                    what: label.to_string(),
                    lookup: lookup_name.to_string(),
                    error: error_text(&Ok(run), Some(&e.to_string())),
                });
            }
        },
        _ => {
            failed.push(Failed {
                what: label.to_string(),
                lookup: lookup_name.to_string(),
                error: error_text(&res, None),
            });
        }
    }
}

struct ProjectIssue {
    id: String,
    identifier: String,
    key: String,
    title: String,
    url: String,
    assignee_me: bool,
}

#[allow(clippy::too_many_arguments)]
fn read_project_issues(
    gh: &CollectGh<'_>,
    args: &[&str],
    assignee_me: bool,
    label: &str,
    lookup_name: &str,
    issue_keys: &serde_json::Map<String, Value>,
    project_items: &mut Vec<ProjectIssue>,
    failed: &mut Vec<Failed>,
    left_out: &mut Vec<LeftOut>,
    seen_left_out: &mut HashSet<String>,
    succeeded: &mut bool,
) {
    let dl = Instant::now() + deadline(TIMEOUT);
    let res = gh(args, None, dl);
    match res {
        Ok(run) if run.ok => match serde_json::from_str::<Value>(&run.stdout) {
            Ok(Value::Array(issues)) => {
                let mut valid = true;
                let mut parsed_items = Vec::new();
                for issue in issues {
                    let id = issue.get("id").and_then(Value::as_str);
                    let number = issue.get("number").and_then(Value::as_u64);
                    let title = issue.get("title").and_then(Value::as_str);
                    let url = issue.get("url").and_then(Value::as_str);
                    let repo = issue
                        .pointer("/repository/nameWithOwner")
                        .and_then(Value::as_str);
                    if let (Some(id), Some(number), Some(title), Some(url), Some(repo)) =
                        (id, number, title, url, repo)
                    {
                        parsed_items.push((
                            id.to_string(),
                            number,
                            title.to_string(),
                            url.to_string(),
                            repo.to_string(),
                        ));
                    } else {
                        valid = false;
                        break;
                    }
                }
                if valid {
                    *succeeded = true;
                    for (id, number, title, url, repo) in parsed_items {
                        let identifier = format!("{repo}#{number}");
                        match key_for_repo(issue_keys, &repo) {
                            Some(k) => {
                                let key = format!("{k}-{number}");
                                project_items.push(ProjectIssue {
                                    id,
                                    identifier,
                                    key,
                                    title,
                                    url,
                                    assignee_me,
                                });
                            }
                            None => {
                                record_left_out(left_out, seen_left_out, &identifier, &repo);
                            }
                        }
                    }
                } else {
                    failed.push(Failed {
                        what: label.to_string(),
                        lookup: lookup_name.to_string(),
                        error: "missing required field".to_string(),
                    });
                }
            }
            Ok(_) => {
                failed.push(Failed {
                    what: label.to_string(),
                    lookup: lookup_name.to_string(),
                    error: "expected array".to_string(),
                });
            }
            Err(e) => {
                failed.push(Failed {
                    what: label.to_string(),
                    lookup: lookup_name.to_string(),
                    error: error_text(&Ok(run), Some(&e.to_string())),
                });
            }
        },
        _ => {
            failed.push(Failed {
                what: label.to_string(),
                lookup: lookup_name.to_string(),
                error: error_text(&res, None),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_project_nodes(
    gh: &CollectGh<'_>,
    label: &str,
    project_num_val: &Value,
    project_owner: &str,
    has_pf: bool,
    project_items: Vec<ProjectIssue>,
    raw_tasks: &mut Vec<RawDashboardTask>,
    failed: &mut Vec<Failed>,
) {
    if project_items.is_empty() {
        return;
    }

    // Deduplicate ids in first-seen order
    let mut unique_ids = Vec::new();
    for item in &project_items {
        if !unique_ids.contains(&item.id.as_str()) {
            unique_ids.push(item.id.as_str());
        }
    }

    let mut id_status_item: std::collections::HashMap<
        String,
        Result<(String, String), Option<String>>,
    > = std::collections::HashMap::new();

    for chunk in unique_ids.chunks(NODES_PER_CALL) {
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
                    if let Some(nodes_arr) = val.pointer("/data/nodes").and_then(Value::as_array) {
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
                let first_err = first_error(&stdout);
                for (idx, &id) in chunk.iter().enumerate() {
                    let node_opt = nodes_arr.get(idx);
                    if let Some(node) = node_opt.filter(|n| !n.is_null()) {
                        let (item, status, err_opt) = extract_project_item_and_status(
                            node,
                            project_num_val,
                            project_owner,
                            first_err.as_deref(),
                        );
                        if let Some(err) = err_opt {
                            id_status_item.insert(id.to_string(), Err(Some(err)));
                        } else {
                            id_status_item.insert(id.to_string(), Ok((item, status)));
                        }
                    } else {
                        let err = first_error(&stdout).unwrap_or_else(|| "not found".to_string());
                        id_status_item.insert(id.to_string(), Err(Some(err)));
                    }
                }
            }
            Err(err_msg) => {
                failed.push(Failed {
                    what: label.to_string(),
                    lookup: "nodes(ids:)".to_string(),
                    error: err_msg,
                });
                for &id in chunk {
                    id_status_item.insert(id.to_string(), Err(None));
                }
            }
        }
    }

    for item in project_items {
        let (item_id, status) = match id_status_item.get(&item.id) {
            Some(Ok((it, st))) => (it.clone(), st.clone()),
            Some(Err(Some(err))) => {
                failed.push(Failed {
                    what: item.key.clone(),
                    lookup: "nodes(ids:)".to_string(),
                    error: err.clone(),
                });
                ("?".to_string(), "?".to_string())
            }
            Some(Err(None)) | None => ("?".to_string(), "?".to_string()),
        };
        raw_tasks.push(RawDashboardTask {
            identifier: item.identifier,
            key: item.key,
            title: item.title,
            url: item.url,
            item: item_id,
            status,
            assignee_me: item.assignee_me,
            has_project_fields: has_pf,
        });
    }
}

pub(super) fn matches_project_number(project_num: Option<&Value>, target: &Value) -> bool {
    match (project_num, target) {
        (Some(Value::Number(n1)), Value::Number(n2)) => n1 == n2,
        (Some(Value::Number(n1)), Value::String(s2)) => n1.to_string() == *s2,
        (Some(Value::String(s1)), Value::String(s2)) => s1 == s2,
        (Some(Value::String(s1)), Value::Number(n2)) => s1 == &n2.to_string(),
        _ => false,
    }
}

pub(super) fn matches_project_owner(owner_val: Option<&Value>, target_owner: &str) -> bool {
    let login = owner_val.and_then(|o| match o {
        Value::Object(_) => o.get("login").and_then(Value::as_str),
        Value::String(s) => Some(s.as_str()),
        _ => None,
    });
    login.is_some_and(|l| l.eq_ignore_ascii_case(target_owner))
}

pub(super) fn extract_project_item_and_status(
    node: &Value,
    project_num_val: &Value,
    project_owner: &str,
    stdout_err: Option<&str>,
) -> (String, String, Option<String>) {
    let project_items = node.get("projectItems");
    if project_items.is_none() || project_items.is_some_and(Value::is_null) {
        if let Some(err) = stdout_err {
            return ("?".to_string(), "?".to_string(), Some(err.to_string()));
        } else {
            return ("no-item".to_string(), "N/A".to_string(), None);
        }
    }

    let items = node
        .pointer("/projectItems/nodes")
        .and_then(Value::as_array);
    let matched_item = items.into_iter().flatten().find(|it| {
        matches_project_number(it.pointer("/project/number"), project_num_val)
            && matches_project_owner(it.pointer("/project/owner"), project_owner)
    });
    match matched_item {
        Some(item) => {
            let item_id = item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("no-item")
                .to_string();
            let status = item
                .pointer("/fieldValueByName/name")
                .and_then(Value::as_str)
                .unwrap_or("N/A")
                .to_string();
            (item_id, status, None)
        }
        None => {
            let has_next_page = node
                .pointer("/projectItems/pageInfo/hasNextPage")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if has_next_page {
                (
                    "?".to_string(),
                    "?".to_string(),
                    Some("projectItems has more than 50 items".to_string()),
                )
            } else {
                ("no-item".to_string(), "N/A".to_string(), None)
            }
        }
    }
}

pub(super) fn ci_of(rollup: Option<&Value>) -> String {
    let Some(val) = rollup else {
        return "-".to_string();
    };
    let items: Vec<&Value> = if let Some(arr) = val.as_array() {
        arr.iter().collect()
    } else if let Some(arr) = val.pointer("/contexts/nodes").and_then(Value::as_array) {
        arr.iter().collect()
    } else if let Some(arr) = val.pointer("/nodes").and_then(Value::as_array) {
        arr.iter().collect()
    } else {
        return "-".to_string();
    };

    if items.is_empty() {
        return "-".to_string();
    }

    let mut any_failing = false;
    let mut any_unknown = false;
    let mut any_pending = false;

    for item in items {
        let type_name = item.get("__typename").and_then(Value::as_str);
        match type_name {
            Some("CheckRun") => {
                let status = item.get("status").and_then(Value::as_str);
                let conclusion = item.get("conclusion").and_then(Value::as_str);
                if matches!(
                    conclusion,
                    Some(
                        "FAILURE"
                            | "TIMED_OUT"
                            | "CANCELLED"
                            | "ACTION_REQUIRED"
                            | "STARTUP_FAILURE"
                    )
                ) {
                    any_failing = true;
                } else if status.is_some() && status != Some("COMPLETED") {
                    any_pending = true;
                } else if matches!(conclusion, Some("SUCCESS" | "NEUTRAL" | "SKIPPED"))
                    || (status == Some("COMPLETED") && conclusion.is_none())
                {
                    // recognized passing
                } else {
                    any_unknown = true;
                }
            }
            Some("StatusContext") => {
                let state = item.get("state").and_then(Value::as_str);
                if matches!(state, Some("FAILURE" | "ERROR")) {
                    any_failing = true;
                } else if matches!(state, Some("PENDING" | "EXPECTED")) {
                    any_pending = true;
                } else if matches!(state, Some("SUCCESS")) {
                    // recognized passing
                } else {
                    any_unknown = true;
                }
            }
            None => {
                let status = item.get("status").and_then(Value::as_str);
                let conclusion = item.get("conclusion").and_then(Value::as_str);
                let state = item.get("state").and_then(Value::as_str);
                if matches!(
                    conclusion,
                    Some(
                        "FAILURE"
                            | "TIMED_OUT"
                            | "CANCELLED"
                            | "ACTION_REQUIRED"
                            | "STARTUP_FAILURE"
                    )
                ) || matches!(state, Some("FAILURE" | "ERROR"))
                {
                    any_failing = true;
                } else if (status.is_some() && status != Some("COMPLETED"))
                    || matches!(state, Some("PENDING" | "EXPECTED"))
                {
                    any_pending = true;
                } else if matches!(conclusion, Some("SUCCESS" | "NEUTRAL" | "SKIPPED"))
                    || matches!(state, Some("SUCCESS"))
                    || (status == Some("COMPLETED") && conclusion.is_none())
                {
                    // recognized passing
                } else {
                    any_unknown = true;
                }
            }
            Some(_) => {
                // Any other __typename is not recognised
                any_unknown = true;
            }
        }
    }

    if any_failing {
        "failing".to_string()
    } else if any_unknown {
        "?".to_string()
    } else if any_pending {
        "pending".to_string()
    } else {
        "passing".to_string()
    }
}
