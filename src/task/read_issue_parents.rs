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

/// The issue a sub-issue hangs under, as the tracker says.
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

/// The query for `n` issues: the values travel as variables (`o0`, `r0`, `p0`, ...), so nothing
/// from a record is ever spliced into the text.
pub(super) fn parent_query(n: usize) -> String {
    let variables: Vec<String> = (0..n)
        .map(|i| format!("$o{i}:String!,$r{i}:String!,$p{i}:Int!"))
        .collect();
    let aliases: Vec<String> = (0..n)
        .map(|i| {
            format!(
                "i{i}:repository(owner:$o{i},name:$r{i}){{issue(number:$p{i}){{\
                 parent{{url number title state subIssuesSummary{{total completed}}}}}}}}"
            )
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
            let (read, why) = read_chunk(host, &asked, deadline);
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

fn read_chunk(host: &str, refs: &[&IssueRef], deadline: Instant) -> Answers {
    let query = format!("query={}", parent_query(refs.len()));
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
    match run(None, &args, deadline) {
        Ok(run) => answers_of(&run.stdout, &run.stderr, refs.len()),
        Err(why) => failed_all(refs.len(), why),
    }
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
            let text = |key: &str| parent.get(key).and_then(Value::as_str);
            let count = |key: &str| {
                parent
                    .pointer(&format!("/subIssuesSummary/{key}"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32
            };
            let (Some(url), Some(number)) =
                (text("url"), parent.get("number").and_then(Value::as_u64))
            else {
                return Err("GitHub's answer names a parent without its URL".to_string());
            };
            Ok(Some(TrackerParent {
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
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests;
