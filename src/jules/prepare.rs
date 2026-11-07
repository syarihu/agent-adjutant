//! Preparing a task to be handed to Jules: assigning the issue, cutting a detached worktree,
//! and writing the task record before planning begins.
//!
//! A task handed to Jules starts no worker: the hub cuts a detached worktree to read from,
//! writes the record, and has a sub-agent write the plan into it. This module carries out the
//! deterministic steps so the hub's procedure keeps only the judgement.

use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::infra::git::git;
use crate::infra::paths::same_path;
use crate::kernel::brief::tracker_and_key;
use crate::kernel::config::{DEFAULT_WORKTREE_NAME, resolve_config};
use crate::kernel::identity::{self, DEFAULT_BRANCH_PATTERN, DEFAULT_WORKTREE_PATTERN};
use crate::registry::Context;
use crate::task::{self, Status};

/// One target to prepare for Jules planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareTarget {
    Issue(String),
    Task(String),
}

/// Request to prepare one or more targets for Jules planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareRequest {
    pub targets: Vec<PrepareTarget>,
    pub base: Option<String>,
}

/// Result of preparing targets for Jules planning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub prepared: Vec<PreparedTask>,
    pub failed: Vec<PrepareFailure>,
}

/// A task prepared for Jules planning, holding what the planning sub-agent brief needs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreparedTask {
    pub record: String,
    pub key: String,
    pub title: String,
    pub tracker: String,
    pub url: Option<String>,
    pub parent: String,
    pub instruction: String,
    pub base: String,
    pub worktree: String,
    pub main: String,
    pub plan_file: String,
    pub gate_file: String,
    pub verify: Vec<String>,
    pub language: Option<String>,
    pub created: bool,
    pub reused_worktree: bool,
    pub assign: AssignOutcome,
}

/// A target that could not be prepared and the reason why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrepareFailure {
    pub target: String,
    pub error: String,
}

/// Whether the task was assigned to the person running the hub.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssignOutcome {
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Render the worktree name template with the task key's parts.
///
/// `{issuekey-lowercase}` is the key's prefix in lowercase (e.g. `wid` of `WID-12`).
/// `{issuekey}` is the full key (e.g. `WID-12`).
/// `{issue}` is the number part of the key (e.g. `12`).
pub fn render_worktree_name(template: &str, key: &str) -> String {
    let (prefix, number) = match key.rsplit_once('-') {
        Some((p, n)) => (p, n),
        None => (key, ""),
    };
    template
        .replace("{issuekey-lowercase}", &prefix.to_lowercase())
        .replace("{issuekey}", key)
        .replace("{issue}", number)
}

/// Normalise a branch name by prefixing `origin/` unless it already starts with it.
pub fn normalize_base(name: &str) -> String {
    if name.starts_with("origin/") {
        name.to_string()
    } else {
        format!("origin/{name}")
    }
}

/// Find the matching task source for a tracker type and optional GitHub repository.
pub fn find_matching_source<'a>(
    sources: &'a [Value],
    tracker: &str,
    issue_repo: Option<&str>,
) -> Option<&'a Value> {
    let type_of = |s: &'a Value| s.get("type").and_then(Value::as_str);
    if tracker == "github"
        && let Some(repo) = issue_repo
        && let Some(found) = sources.iter().find(|s| {
            type_of(s) == Some("github")
                && s.get("issueRepo")
                    .and_then(Value::as_str)
                    .is_some_and(|r| r.eq_ignore_ascii_case(repo))
        })
    {
        return Some(found);
    }
    sources.iter().find(|s| type_of(s) == Some(tracker))
}

/// Check if git worktree list lists `path`.
fn is_worktree_listed(main: &str, path: &str) -> Result<bool, String> {
    let list = identity::worktrees(main)?;
    Ok(list.iter().any(|w| same_path(&w.path, path)))
}

/// Look up an issue's parent on GitHub via `gh issue view`.
fn lookup_github_parent(main: &str, url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let _host = parts.next()?;
    let owner = parts.next()?;
    let repo = parts.next()?;
    let section = parts.next()?;
    let number = parts.next()?;
    if section != "issues" || number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let nwo = format!("{owner}/{repo}");
    let run = crate::infra::gh::run(
        Some(main),
        &[
            "issue",
            "view",
            number,
            "-R",
            &nwo,
            "--json",
            "parent",
            "--jq",
            ".parent.url // \"\"",
        ],
        Instant::now() + Duration::from_secs(20),
    )
    .ok()?;
    if !run.ok {
        return None;
    }
    let out = run.stdout.trim().to_string();
    if out.is_empty() { None } else { Some(out) }
}

/// Extract owner, repo, and issue number if the URL is a GitHub issue.
fn parse_github_issue_url(url: &str) -> Option<(String, String)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let mut parts = rest.split(['?', '#']).next().unwrap_or("").split('/');
    let _host = parts.next()?;
    let owner = parts.next()?;
    let repo = parts.next()?;
    let section = parts.next()?;
    let number = parts.next()?;
    if section == "issues" && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) {
        Some((format!("{owner}/{repo}"), number.to_string()))
    } else {
        None
    }
}

/// Decide the base commit-ish on origin for a target.
fn decide_base(main: &str, nwo: &str, candidate: &str) -> Result<String, String> {
    if candidate == "auto" {
        // 1. Try newest origin/release/*
        let out = git(
            &[
                "branch",
                "-r",
                "--list",
                "origin/release/*",
                "--format=%(refname:short)",
                "--sort=-version:refname",
            ],
            Some(Path::new(main)),
        )?;
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Some(first) = stdout.lines().map(str::trim).find(|l| !l.is_empty()) {
                return Ok(first.to_string());
            }
        }

        // 2. Try remote's default branch
        let head_out = git(
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
            Some(Path::new(main)),
        )?;
        if head_out.status.success() {
            let refname = String::from_utf8_lossy(&head_out.stdout).trim().to_string();
            if !refname.is_empty() {
                return Ok(refname);
            }
        }

        // 3. Fallback to gh repo view
        let gh_run = crate::infra::gh::run(
            Some(main),
            &[
                "repo",
                "view",
                nwo,
                "--json",
                "defaultBranchRef",
                "--jq",
                ".defaultBranchRef.name",
            ],
            Instant::now() + Duration::from_secs(20),
        );
        if let Ok(run) = gh_run
            && run.ok
        {
            let name = run.stdout.trim();
            if !name.is_empty() {
                return Ok(format!("origin/{name}"));
            }
        }

        return Err("cannot decide default branch for remote origin".to_string());
    }

    let norm = normalize_base(candidate);
    let verify = git(
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{norm}^{{commit}}"),
        ],
        Some(Path::new(main)),
    )?;
    if !verify.status.success() {
        return Err(format!(
            "base {norm} does not exist on origin; ask which branch to use"
        ));
    }
    Ok(norm)
}

/// Check if a target needs git fetch (new worktree or base to be decided).
fn target_needs_fetch(
    ctx: &Context,
    target: &PrepareTarget,
    request_base: Option<&str>,
    listed: &[String],
) -> bool {
    if request_base.is_some() {
        return true;
    }
    let existing_task = match target {
        PrepareTarget::Task(id) => task::get(&ctx.state, &ctx.repo.slug, id).ok(),
        PrepareTarget::Issue(n_or_url) => {
            let url = if n_or_url.chars().all(|c| c.is_ascii_digit()) {
                format!("https://github.com/{}/issues/{n_or_url}", ctx.repo.nwo)
            } else {
                n_or_url.clone()
            };
            task::list(&ctx.state, &ctx.repo.slug)
                .into_iter()
                .find(|t| {
                    !matches!(t.status, Status::Done | Status::Cancelled)
                        && t.issue_url.as_deref() == Some(&url)
                })
        }
    };
    let Some(t) = existing_task else {
        return true;
    };
    if t.base.is_none() {
        return true;
    }
    let Some(wt) = &t.worktree else {
        return true;
    };
    if !Path::new(wt).exists() || !listed.iter().any(|w| same_path(w, wt)) {
        return true;
    }
    let sym = git(&["symbolic-ref", "-q", "HEAD"], Some(Path::new(wt)));
    !matches!(sym, Ok(out) if !out.status.success())
}

/// Prepare one or more tasks to be planned by Jules.
pub fn prepare(ctx: &Context, request: &PrepareRequest) -> Prepared {
    let mut prepared = Vec::new();
    let mut failed = Vec::new();

    // Determine config's baseBranch once
    let config_base: Result<String, String> = resolve_config(&ctx.repo.nwo).map(|r| {
        r.config
            .as_ref()
            .and_then(|c| c.get("baseBranch"))
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .to_string()
    });

    let listed = identity::linked_worktrees(&ctx.repo.main).unwrap_or_default();
    let needs_fetch = request
        .targets
        .iter()
        .any(|t| target_needs_fetch(ctx, t, request.base.as_deref(), &listed));

    // Run git fetch --prune origin once per call if needed
    let mut fetch_err: Option<String> = None;
    if needs_fetch {
        let fetch_out = git(
            &["fetch", "--prune", "origin"],
            Some(Path::new(&ctx.repo.main)),
        );
        match fetch_out {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                fetch_err = Some(format!("git fetch --prune origin failed: {stderr}"));
            }
            Err(e) => {
                fetch_err = Some(format!("git fetch --prune origin failed: {e}"));
            }
        }
    }

    for target in &request.targets {
        let target_str = match target {
            PrepareTarget::Issue(s) | PrepareTarget::Task(s) => s.clone(),
        };

        match prepare_one(
            ctx,
            target,
            request.base.as_deref(),
            config_base.as_deref().map_err(|e| e.as_str()),
            fetch_err.as_deref(),
        ) {
            Ok(p) => prepared.push(p),
            Err(e) => failed.push(PrepareFailure {
                target: target_str,
                error: e,
            }),
        }
    }

    Prepared { prepared, failed }
}

fn prepare_one(
    ctx: &Context,
    target: &PrepareTarget,
    request_base: Option<&str>,
    config_base: Result<&str, &str>,
    fetch_err: Option<&str>,
) -> Result<PreparedTask, String> {
    // 3.a Find or create the record
    let (mut task, created) = match target {
        PrepareTarget::Task(id) => {
            let t = task::get(&ctx.state, &ctx.repo.slug, id)?;
            if t.executor != task::Executor::Jules {
                return Err(format!(
                    "{} is to be implemented by its worker; `adj task update --id {} --executor jules` first",
                    t.id, t.id
                ));
            }
            if let Some(session) = &t.jules_session {
                return Err(format!("already with Jules (session {session})"));
            }
            if !matches!(
                t.status,
                Status::Backlog | Status::Queued | Status::Dispatched
            ) {
                return Err(format!(
                    "{} is in status {}: must be backlog, queued or dispatched",
                    t.id,
                    t.status.as_str()
                ));
            }
            (t, false)
        }
        PrepareTarget::Issue(n_or_url) => {
            let url = if n_or_url.chars().all(|c| c.is_ascii_digit()) {
                format!("https://github.com/{}/issues/{n_or_url}", ctx.repo.nwo)
            } else {
                n_or_url.clone()
            };
            task::check_url(&url, "an issue")?;

            // Check tracker and key before creating
            let gh_info = parse_github_issue_url(&url);
            if let Some((repo, _)) = &gh_info {
                let has_key =
                    ctx.settings.issue_keys.iter().any(|(name, key)| {
                        name.eq_ignore_ascii_case(repo) && key.as_str().is_some()
                    });
                if !has_key {
                    return Err(format!(
                        "no issueKeys entry for {repo}: no worktree name can be made"
                    ));
                }
            }

            // Check if record with this issue_url already exists in a non-final status
            let existing = task::list(&ctx.state, &ctx.repo.slug)
                .into_iter()
                .find(|t| {
                    !matches!(t.status, Status::Done | Status::Cancelled)
                        && t.issue_url.as_deref() == Some(&url)
                });

            if let Some(t) = existing {
                if t.executor != task::Executor::Jules {
                    return Err(format!(
                        "{} is to be implemented by its worker; `adj task update --id {} --executor jules` first",
                        t.id, t.id
                    ));
                }
                if let Some(session) = &t.jules_session {
                    return Err(format!("already with Jules (session {session})"));
                }
                if !matches!(
                    t.status,
                    Status::Backlog | Status::Queued | Status::Dispatched
                ) {
                    return Err(format!(
                        "{} is in status {}: must be backlog, queued or dispatched",
                        t.id,
                        t.status.as_str()
                    ));
                }
                (t, false)
            } else {
                let parent = lookup_github_parent(&ctx.repo.main, &url);
                let (new_task, _) = task::create(
                    ctx,
                    task::NewTask {
                        kind: task::Kind::Start,
                        body: String::new(),
                        issue_url: Some(url.clone()),
                        executor: task::Executor::Jules,
                        done_when: task::DoneWhen::Pr,
                        stop_at: task::StopAt::Plan,
                        status: Status::Queued,
                        parent,
                        ..Default::default()
                    },
                    false,
                )?;
                (new_task, true)
            }
        }
    };

    let res = (|| -> Result<PreparedTask, String> {
        // 3.b Tracker and key
        let (tracker, key) = match &task.issue_url {
            Some(url) => {
                match tracker_and_key(url, &ctx.settings.task_sources, &ctx.settings.issue_keys) {
                    Some((t, k)) => (t.as_str().to_string(), k),
                    None => ("-".to_string(), "-".to_string()),
                }
            }
            None => ("-".to_string(), "-".to_string()),
        };

        // 3.c & 3.d Worktree reuse check
        let listed = identity::linked_worktrees(&ctx.repo.main)?;
        let can_reuse = task
            .worktree
            .as_deref()
            .is_some_and(|wt| Path::new(wt).exists() && listed.iter().any(|w| same_path(w, wt)));

        let (worktree, reused_worktree, base) = if can_reuse {
            let wt = task.worktree.clone().unwrap();
            // Check if detached
            let sym = git(&["symbolic-ref", "-q", "HEAD"], Some(Path::new(&wt)))?;
            if sym.status.success() {
                let branch = String::from_utf8_lossy(&sym.stdout)
                    .trim()
                    .trim_start_matches("refs/heads/")
                    .to_string();
                return Err(format!(
                    "{wt} is on branch {branch}, not a Jules planning worktree"
                ));
            }
            // Base: request.base wins, otherwise keep record's base
            let base = match request_base {
                Some(req_b) => {
                    if let Some(err) = fetch_err {
                        return Err(err.to_string());
                    }
                    decide_base(&ctx.repo.main, &ctx.repo.nwo, req_b)?
                }
                None => match task.base.clone() {
                    Some(b) => decide_base(&ctx.repo.main, &ctx.repo.nwo, &b)?,
                    None => {
                        if let Some(err) = fetch_err {
                            return Err(err.to_string());
                        }
                        let cfg_b = match config_base {
                            Ok(b) => b,
                            Err(e) => {
                                return Err(format!(
                                    "cannot read the config to decide the base: {e}"
                                ));
                            }
                        };
                        decide_base(&ctx.repo.main, &ctx.repo.nwo, cfg_b)?
                    }
                },
            };

            let wt_head_out = git(&["rev-parse", "HEAD"], Some(Path::new(&wt)))?;
            if !wt_head_out.status.success() {
                let stderr = String::from_utf8_lossy(&wt_head_out.stderr)
                    .trim()
                    .to_string();
                return Err(format!("git rev-parse HEAD in {wt} failed: {stderr}"));
            }
            let wt_head = String::from_utf8_lossy(&wt_head_out.stdout)
                .trim()
                .to_string();

            let base_commit_out = git(
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("{base}^{{commit}}"),
                ],
                Some(Path::new(&ctx.repo.main)),
            )?;
            if !base_commit_out.status.success() {
                return Err(format!(
                    "base {base} does not exist on origin; ask which branch to use"
                ));
            }
            let base_commit = String::from_utf8_lossy(&base_commit_out.stdout)
                .trim()
                .to_string();

            if wt_head != base_commit {
                let short_head = git(&["rev-parse", "--short", "HEAD"], Some(Path::new(&wt)))
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| wt_head.chars().take(7).collect());
                return Err(format!(
                    "{wt} is at {short_head}, not {base}: remove it, or prepare without --base"
                ));
            }

            (wt, true, base)
        } else {
            if let Some(err) = fetch_err {
                return Err(err.to_string());
            }

            let base_candidate = match request_base.or(task.base.as_deref()) {
                Some(b) => b,
                None => match config_base {
                    Ok(b) => b,
                    Err(e) => {
                        return Err(format!("cannot read the config to decide the base: {e}"));
                    }
                },
            };
            let base = decide_base(&ctx.repo.main, &ctx.repo.nwo, base_candidate)?;

            // Compute worktree name
            let issue_repo = task
                .issue_url
                .as_deref()
                .and_then(parse_github_issue_url)
                .map(|(r, _)| r);
            let matching_source =
                find_matching_source(&ctx.settings.task_sources, &tracker, issue_repo.as_deref());

            let name = if let Some(custom) = &task.worktree_name {
                custom.clone()
            } else if key == "-" {
                return Err("pass a worktree name on the record".to_string());
            } else {
                let template = matching_source
                    .and_then(|s| s.get("worktreeName"))
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_WORKTREE_NAME);
                render_worktree_name(template, &key)
            };
            task::check_worktree_name(&name)?;

            let branch_pattern = matching_source
                .and_then(|s| s.get("branchPattern"))
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_BRANCH_PATTERN);

            let user = super::github_login(&ctx.repo.main)
                .or_else(|| std::env::var("USER").ok())
                .unwrap_or_else(|| "worker".to_string());

            let branch = identity::branch_fallback(branch_pattern, &user, &name);
            let layout = ctx
                .settings
                .worktree_pattern
                .as_deref()
                .unwrap_or(DEFAULT_WORKTREE_PATTERN);
            let path = identity::worktree_fallback(layout, &ctx.repo.main, &branch)?;

            if Path::new(&path).exists() || is_worktree_listed(&ctx.repo.main, &path)? {
                return Err(format!("{path} already exists"));
            }

            // Add detached worktree
            let add = git(
                &["worktree", "add", "--detach", &path, &base],
                Some(Path::new(&ctx.repo.main)),
            )?;
            if !add.status.success() {
                let stderr = String::from_utf8_lossy(&add.stderr).trim().to_string();
                return Err(format!("git worktree add failed: {stderr}"));
            }

            // Create .claude directory
            let claude_dir = Path::new(&path).join(".claude");
            if let Err(e) = std::fs::create_dir_all(&claude_dir) {
                let _ = git(
                    &["worktree", "remove", "--force", &path],
                    Some(Path::new(&ctx.repo.main)),
                );
                return Err(format!("cannot create {}: {e}", claude_dir.display()));
            }

            (path, false, base)
        };

        // 3.e Assign
        let assign = match (&task.issue_url, tracker.as_str()) {
            (Some(url), "github" | "github-project") => {
                if let Some((repo, number)) = parse_github_issue_url(url) {
                    let ran = crate::infra::gh::run(
                        Some(&ctx.repo.main),
                        &[
                            "issue",
                            "edit",
                            &number,
                            "-R",
                            &repo,
                            "--add-assignee",
                            "@me",
                        ],
                        Instant::now() + Duration::from_secs(20),
                    );
                    match ran {
                        Ok(out) if out.ok => AssignOutcome {
                            done: true,
                            reason: None,
                        },
                        Ok(out) => {
                            let stderr = out.stderr.trim().to_string();
                            AssignOutcome {
                                done: false,
                                reason: Some(stderr),
                            }
                        }
                        Err(e) => AssignOutcome {
                            done: false,
                            reason: Some(e),
                        },
                    }
                } else {
                    AssignOutcome {
                        done: false,
                        reason: Some("cannot parse issue number or repo".to_string()),
                    }
                }
            }
            (Some(_), "jira" | "linear") => AssignOutcome {
                done: false,
                reason: Some(
                    "not a GitHub issue: assign it with the tracker's own tool".to_string(),
                ),
            },
            _ => AssignOutcome {
                done: false,
                reason: Some("no issue".to_string()),
            },
        };

        // 3.f Mark it in progress
        let patch = task::TaskPatch {
            status: Some(Status::Dispatched),
            worktree: Some(Some(worktree.clone())),
            base: Some(Some(base.clone())),
            note: Some(None),
            ..Default::default()
        };
        if let Err(e) = task::update(ctx, &task.id, &patch, false) {
            if !reused_worktree {
                let _ = git(
                    &["worktree", "remove", "--force", &worktree],
                    Some(Path::new(&ctx.repo.main)),
                );
            }
            return Err(e);
        }
        task.worktree = Some(worktree.clone());
        task.base = Some(base.clone());
        task.status = Status::Dispatched;

        let plan_file = format!("{worktree}/.claude/jules-plan.md");
        let gate_file = format!("{worktree}/.claude/jules-gate.json");

        Ok(PreparedTask {
            record: task.id.clone(),
            key,
            title: task.title.clone(),
            tracker,
            url: task.issue_url.clone(),
            parent: task.parent.clone().unwrap_or_else(|| "-".to_string()),
            instruction: task.instruction.clone().unwrap_or_else(|| "-".to_string()),
            base,
            worktree,
            main: ctx.repo.main.clone(),
            plan_file,
            gate_file,
            verify: ctx.settings.verify.clone(),
            language: ctx.settings.language.clone(),
            created,
            reused_worktree,
            assign,
        })
    })();

    match res {
        Ok(p) => Ok(p),
        Err(e) => {
            if created {
                match task::remove(ctx, &task.id) {
                    Ok(()) => Err(format!("{e}; nothing was kept")),
                    Err(rm_err) => Err(format!(
                        "{e}; the record {} it created could not be removed: {rm_err}",
                        task.id
                    )),
                }
            } else {
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_normalization_prefixes_origin_only_when_needed() {
        assert_eq!(normalize_base("main"), "origin/main");
        assert_eq!(normalize_base("origin/main"), "origin/main");
        assert_eq!(normalize_base("feature/login"), "origin/feature/login");
        assert_eq!(
            normalize_base("origin/feature/login"),
            "origin/feature/login"
        );
    }

    #[test]
    fn worktree_name_template_renders_placeholders() {
        assert_eq!(
            render_worktree_name("{issuekey-lowercase}-{issue}", "WID-12"),
            "wid-12"
        );
        assert_eq!(render_worktree_name("{issuekey}", "WID-12"), "WID-12");
        assert_eq!(
            render_worktree_name("{issuekey-lowercase}", "WID-12"),
            "wid"
        );
        assert_eq!(render_worktree_name("{issue}", "WID-12"), "12");
    }

    #[test]
    fn source_matching_finds_github_with_issue_repo_and_github_project_by_type() {
        let sources = serde_json::json!([
            {
                "type": "github",
                "issueRepo": "acme/widget",
                "worktreeName": "widget-{issue}"
            },
            {
                "type": "github",
                "issueRepo": "acme/other",
                "worktreeName": "other-{issue}"
            },
            {
                "type": "github-project",
                "worktreeName": "proj-{issue}"
            },
            {
                "type": "jira",
                "worktreeName": "jira-{issue}"
            }
        ]);
        let array = sources.as_array().unwrap();

        let s1 = find_matching_source(array, "github", Some("acme/widget")).unwrap();
        assert_eq!(s1["worktreeName"], "widget-{issue}");

        let s2 = find_matching_source(array, "github", Some("ACME/OTHER")).unwrap();
        assert_eq!(s2["worktreeName"], "other-{issue}");

        let s3 = find_matching_source(array, "github-project", None).unwrap();
        assert_eq!(s3["worktreeName"], "proj-{issue}");

        let s4 = find_matching_source(array, "jira", None).unwrap();
        assert_eq!(s4["worktreeName"], "jira-{issue}");
    }
}
