//! The parent issue each of some issues has on the tracker, asked of GitHub in as few queries as
//! it takes.
//!
//! A task's `parent` is the record's own word; GitHub's sub-issues are the tracker's. The board
//! reads the second and falls back on the first, so what is asked here is only the tracker's.

use std::time::Instant;

use serde_json::Value;

use crate::infra::gh::run;

use super::github::{first_error, issue_parts, pr_ref_of, unread_error};

/// How many issues one query asks about, as `read_prs` does for pull requests.
const PER_QUERY: usize = 50;

/// An issue as GitHub names it, with the owner and repository in lower case where GitHub
/// compares them so.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IssueRef {
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

/// The issue an issue URL `fetchable_issue` accepts names. A value that starts with `-` or holds
/// a character a name cannot is `None`, since it travels to `gh` as a value.
pub fn issue_ref_of(url: &str) -> Option<IssueRef> {
    let url = url.trim();
    let (owner, repo, number) = issue_parts(url)?;
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    let r = pr_ref_of(host, owner, repo, number)?;
    Some(IssueRef {
        host: r.host,
        owner: r.owner,
        repo: r.repo,
        number: r.number,
    })
}

/// The issue a sub-issue hangs under, as the tracker says. Also what an issue read on its own
/// (`read_issue_summary`) comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerParent {
    pub url: String,
    pub number: u64,
    pub title: String,
    /// Whether the parent is still open.
    pub open: bool,
    /// How many sub-issues the parent has, and how many of them are done.
    pub total: u32,
    pub completed: u32,
}

/// What reading the parents of some issues came to.
pub struct IssueParentsRead {
    /// One answer per issue, in the order they were given: the parent it has, none, or why that
    /// issue could not be read.
    pub answers: Vec<Result<Option<TrackerParent>, String>>,
    /// Why `gh` or GitHub could not be asked at all, when that is so for any of the queries: the
    /// round failing, as against one issue that is gone.
    pub failed: Option<String>,
}

/// What is asked of an issue, whether it is the parent of the one named or the one named.
const ISSUE_FIELDS: &str = "url number title state subIssuesSummary{total completed}";

/// The query for the parents of `n` issues: the values travel as variables (`o0`, `r0`, `p0`,
/// ...), so nothing from a record is ever spliced into the text.
pub(super) fn parent_query(n: usize) -> String {
    query_of(n, &format!("parent{{{ISSUE_FIELDS}}}"))
}

/// The query for `n` issues themselves, with the same variables as `parent_query`.
pub(super) fn issue_query(n: usize) -> String {
    query_of(n, ISSUE_FIELDS)
}

fn query_of(n: usize, selection: &str) -> String {
    let variables: Vec<String> = (0..n)
        .map(|i| format!("$o{i}:String!,$r{i}:String!,$p{i}:Int!"))
        .collect();
    let aliases: Vec<String> = (0..n)
        .map(|i| {
            format!("i{i}:repository(owner:$o{i},name:$r{i}){{issue(number:$p{i}){{{selection}}}}}")
        })
        .collect();
    format!("query({}){{{}}}", variables.join(","), aliases.join(" "))
}

/// Read the parent of each of `refs`, one query per host and `PER_QUERY` issues.
pub fn read_issue_parents(refs: &[IssueRef], deadline: Instant) -> IssueParentsRead {
    let mut answers: Vec<Option<Result<Option<TrackerParent>, String>>> = vec![None; refs.len()];
    let mut failed = None;
    let mut hosts: Vec<&str> = Vec::new();
    for r in refs {
        if !hosts.contains(&r.host.as_str()) {
            hosts.push(&r.host);
        }
    }
    for host in hosts {
        let at: Vec<usize> = (0..refs.len()).filter(|&i| refs[i].host == host).collect();
        for chunk in at.chunks(PER_QUERY) {
            let asked: Vec<&IssueRef> = chunk.iter().map(|&i| &refs[i]).collect();
            let (read, why) = read_chunk(host, &asked, &parent_query(asked.len()), deadline);
            failed = failed.or(why);
            for (&i, answer) in chunk.iter().zip(read) {
                answers[i] = Some(answer);
            }
        }
    }
    IssueParentsRead {
        answers: answers
            .into_iter()
            .map(|a| a.unwrap_or_else(|| Err("not read".to_string())))
            .collect(),
        failed,
    }
}

type Answers = (Vec<Result<Option<TrackerParent>, String>>, Option<String>);

fn read_chunk(host: &str, refs: &[&IssueRef], query: &str, deadline: Instant) -> Answers {
    match graphql(host, refs, query, deadline) {
        Ok(run) => answers_of(&run.stdout, &run.stderr, refs.len()),
        Err(why) => failed_all(refs.len(), why),
    }
}

/// `gh api graphql` for `query`, whose variables name each of `refs`.
fn graphql(
    host: &str,
    refs: &[&IssueRef],
    query: &str,
    deadline: Instant,
) -> Result<crate::infra::gh::GhRun, String> {
    let query = format!("query={query}");
    let mut owned: Vec<String> = Vec::new();
    for (i, r) in refs.iter().enumerate() {
        owned.push(format!("o{i}={}", r.owner));
        owned.push(format!("r{i}={}", r.repo));
        owned.push(format!("p{i}={}", r.number));
    }
    let mut args: Vec<&str> = vec!["api", "graphql", "--hostname", host, "-f", &query];
    for (i, _) in refs.iter().enumerate() {
        args.extend(["-f", &owned[i * 3], "-f", &owned[i * 3 + 1]]);
        args.extend(["-F", &owned[i * 3 + 2]]);
    }
    run(None, &args, deadline)
}

fn failed_all(n: usize, why: String) -> Answers {
    ((0..n).map(|_| Err(why.clone())).collect(), Some(why))
}

/// What one query printed, read as the answers of `n` issues and, when the query itself failed,
/// why. Whatever the exit status was is read: GraphQL answers a query that found some issues and
/// not others with data and errors together, and `gh` exits 1.
pub(super) fn answers_of(stdout: &str, stderr: &str, n: usize) -> Answers {
    match serde_json::from_str::<Value>(stdout) {
        // Without `data` nothing was looked up: a rate limit, a login, a server error.
        Ok(value) if value.get("data").is_some_and(Value::is_object) => {
            (parse_parents(&value, n), unread_error(&value))
        }
        Ok(_) => failed_all(
            n,
            first_error(stdout)
                .or_else(|| Some(stderr.to_string()).filter(|s| !s.is_empty()))
                .unwrap_or_else(|| "GitHub answered without data".to_string()),
        ),
        Err(_) if !stderr.is_empty() => failed_all(n, stderr.to_string()),
        Err(_) => failed_all(n, format!("gh answered {:?}", stdout.trim())),
    }
}

/// The answers of `n` issues out of what `gh api graphql` printed for `parent_query`. An issue
/// whose alias holds no issue is the one the error that names it says, and says nothing of the
/// others.
fn parse_parents(value: &Value, n: usize) -> Vec<Result<Option<TrackerParent>, String>> {
    let errors = value
        .get("errors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    (0..n)
        .map(|i| {
            let alias = format!("i{i}");
            // An error under this alias, whatever field it is on (the parent, its sub-issue
            // summary), says the read of this issue did not come whole: what came back beside it
            // is not an answer, and a parent that is `null` there is not "no parent".
            let own_error = errors
                .iter()
                .find(|e| e.pointer("/path/0").and_then(Value::as_str) == Some(alias.as_str()))
                .and_then(|e| e.get("message").and_then(Value::as_str));
            if let Some(said) = own_error
                && value
                    .pointer(&format!("/data/{alias}/issue"))
                    .is_some_and(Value::is_object)
            {
                return Err(said.to_string());
            }
            let Some(issue) = value
                .pointer(&format!("/data/{alias}/issue"))
                .filter(|issue| issue.is_object())
            else {
                // The error that names this alias; failing that, the first one.
                let said = errors
                    .iter()
                    .find(|e| e.pointer("/path/0").and_then(Value::as_str) == Some(alias.as_str()))
                    .or_else(|| errors.first())
                    .and_then(|e| e.get("message").and_then(Value::as_str));
                return Err(said.unwrap_or("no such issue").to_string());
            };
            let Some(parent) = issue.get("parent").filter(|p| p.is_object()) else {
                return Ok(None);
            };
            tracker_issue_of(parent).map(Some)
        })
        .collect()
}

/// An issue as GitHub's answer holds it, whether it came as a parent or on its own.
fn tracker_issue_of(node: &Value) -> Result<TrackerParent, String> {
    let text = |key: &str| node.get(key).and_then(Value::as_str);
    let count = |key: &str| {
        node.pointer(&format!("/subIssuesSummary/{key}"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32
    };
    let (Some(url), Some(number)) = (text("url"), node.get("number").and_then(Value::as_u64))
    else {
        return Err("GitHub's answer names an issue without its URL".to_string());
    };
    Ok(TrackerParent {
        url: url.to_string(),
        number,
        title: text("title")
            .unwrap_or_default()
            .chars()
            .take(super::ISSUE_TITLE_CAP)
            .collect(),
        open: text("state").is_none_or(|s| s.eq_ignore_ascii_case("open")),
        total: count("total"),
        completed: count("completed"),
    })
}

/// What reading one issue on its own came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueSummaryRead {
    Found(TrackerParent),
    /// GitHub says there is no such issue (or none this login can see). Asking again will not
    /// change that, as against `Failed`.
    NotFound,
    /// `gh` or GitHub could not be asked, or its answer was not an issue.
    Failed(String),
}

/// Read one issue: its title and its sub-issue counts.
pub fn read_issue_summary(issue: &IssueRef, deadline: Instant) -> IssueSummaryRead {
    match graphql(&issue.host, &[issue], &issue_query(1), deadline) {
        Ok(run) => summary_of(&run.stdout, &run.stderr),
        Err(why) => IssueSummaryRead::Failed(why),
    }
}

/// What `gh api graphql` printed for `issue_query(1)`, read as one issue. Whatever the exit
/// status was is read, as in `answers_of`; an issue is told gone by the type of the error that
/// names its alias, never by its message.
pub(super) fn summary_of(stdout: &str, stderr: &str) -> IssueSummaryRead {
    let value = match serde_json::from_str::<Value>(stdout) {
        Ok(value) if value.get("data").is_some_and(Value::is_object) => value,
        // Nothing was looked up: said as `answers_of` says it.
        _ => {
            let (mut answers, _) = answers_of(stdout, stderr, 1);
            return IssueSummaryRead::Failed(
                answers
                    .pop()
                    .and_then(Result::err)
                    .unwrap_or_else(|| "GitHub answered without data".to_string()),
            );
        }
    };
    if let Some(why) = unread_error(&value) {
        return IssueSummaryRead::Failed(why);
    }
    if let Some(issue) = value.pointer("/data/i0/issue").filter(|i| i.is_object()) {
        return match tracker_issue_of(issue) {
            Ok(found) => IssueSummaryRead::Found(found),
            Err(why) => IssueSummaryRead::Failed(why),
        };
    }
    let own = value
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|errors| {
            errors
                .iter()
                .find(|e| e.pointer("/path/0").and_then(Value::as_str) == Some("i0"))
        });
    match own {
        Some(e) if e.get("type").and_then(Value::as_str) == Some("NOT_FOUND") => {
            IssueSummaryRead::NotFound
        }
        Some(e) => IssueSummaryRead::Failed(
            e.get("message")
                .and_then(Value::as_str)
                .unwrap_or("GitHub reported an error")
                .to_string(),
        ),
        None => IssueSummaryRead::Failed("GitHub's answer holds no issue".to_string()),
    }
}

#[cfg(test)]
mod tests;
