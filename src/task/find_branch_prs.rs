//! The pull request each of some branches has, asked of GitHub in as few queries as it takes.
//!
//! A task's pull request is the task's own record; a session with no task has none, so it is
//! found by the branch it works on instead.

use std::time::Instant;

use serde_json::Value;

use crate::infra::gh::run;

use super::github::{first_error, unread_error};

/// How many branches one query asks about, as `read_prs` does for pull requests.
const PER_QUERY: usize = 50;

/// How many pull requests of a branch are looked at in each of the two lists (the open ones, and
/// the newest of any state): a few forks or closed ones newer than an open one must not hide it.
const PER_BRANCH: usize = 10;

/// A branch of a repository, with the names in lower case where GitHub compares them so.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BranchRef {
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub branch: String,
}

/// The pull request of a branch, as a session row shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchPr {
    pub number: u64,
    pub url: String,
    /// `open`, `draft`, `merged` or `closed`.
    pub state: String,
}

/// What reading the pull requests of some branches came to.
pub struct BranchPrsRead {
    /// One answer per branch, in the order they were given: the pull request it has, none, or
    /// why that branch could not be read.
    pub answers: Vec<Result<Option<BranchPr>, String>>,
    /// Why `gh` or GitHub could not be asked at all, when that is so for any of the queries:
    /// the round failing, as against one repository that is gone.
    pub failed: Option<String>,
}

/// The query for `n` branches: the values travel as variables (`o0`, `r0`, `h0`, ...), so
/// nothing from a branch name is ever spliced into the text.
pub(super) fn branch_query(n: usize) -> String {
    let variables: Vec<String> = (0..n)
        .map(|i| format!("$o{i}:String!,$r{i}:String!,$h{i}:String!"))
        .collect();
    let aliases: Vec<String> = (0..n)
        .map(|i| {
            format!(
                "b{i}:repository(owner:$o{i},name:$r{i}){{\
                 open:pullRequests(headRefName:$h{i},first:{PER_BRANCH},states:[OPEN],\
                 orderBy:{{field:CREATED_AT,direction:DESC}}){{nodes{{...F}}}} \
                 recent:pullRequests(headRefName:$h{i},first:{PER_BRANCH},\
                 states:[OPEN,MERGED,CLOSED],\
                 orderBy:{{field:CREATED_AT,direction:DESC}}){{nodes{{...F}}}}}}"
            )
        })
        .collect();
    format!(
        "query({}){{{}}} fragment F on PullRequest {{ number url state isDraft \
         headRepositoryOwner{{login}} }}",
        variables.join(","),
        aliases.join(" ")
    )
}

/// Read the pull request of each of `refs`, one query per host and `PER_QUERY` branches.
pub fn find_branch_prs(refs: &[BranchRef], deadline: Instant) -> BranchPrsRead {
    let mut answers: Vec<Option<Result<Option<BranchPr>, String>>> = vec![None; refs.len()];
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
            let asked: Vec<&BranchRef> = chunk.iter().map(|&i| &refs[i]).collect();
            let (read, why) = read_chunk(host, &asked, deadline);
            failed = failed.or(why);
            for (&i, answer) in chunk.iter().zip(read) {
                answers[i] = Some(answer);
            }
        }
    }
    BranchPrsRead {
        answers: answers
            .into_iter()
            .map(|a| a.unwrap_or_else(|| Err("not read".to_string())))
            .collect(),
        failed,
    }
}

type Answers = (Vec<Result<Option<BranchPr>, String>>, Option<String>);

fn read_chunk(host: &str, refs: &[&BranchRef], deadline: Instant) -> Answers {
    let query = format!("query={}", branch_query(refs.len()));
    let mut owned: Vec<String> = Vec::new();
    for (i, r) in refs.iter().enumerate() {
        owned.push(format!("o{i}={}", r.owner));
        owned.push(format!("r{i}={}", r.repo));
        owned.push(format!("h{i}={}", r.branch));
    }
    let mut args: Vec<&str> = vec!["api", "graphql", "--hostname", host, "-f", &query];
    for value in &owned {
        args.extend(["-f", value]);
    }
    match run(None, &args, deadline) {
        Ok(run) => answers_of(&run.stdout, &run.stderr, refs),
        Err(why) => failed_all(refs.len(), why),
    }
}

fn failed_all(n: usize, why: String) -> Answers {
    ((0..n).map(|_| Err(why.clone())).collect(), Some(why))
}

/// What one query printed, read as the answers of `refs` and, when the query itself failed, why.
/// Whatever the exit status was is read: GraphQL answers a query that found some repositories
/// and not others with data and errors together.
fn answers_of(stdout: &str, stderr: &str, refs: &[&BranchRef]) -> Answers {
    match serde_json::from_str::<Value>(stdout) {
        // Without `data` nothing was looked up: a rate limit, a login, a server error.
        Ok(value) if value.get("data").is_some_and(Value::is_object) => {
            (parse_branch_prs(stdout, refs), unread_error(&value))
        }
        Ok(_) => failed_all(
            refs.len(),
            first_error(stdout)
                .or_else(|| Some(stderr.to_string()).filter(|s| !s.is_empty()))
                .unwrap_or_else(|| "GitHub answered without data".to_string()),
        ),
        Err(_) if !stderr.is_empty() => failed_all(refs.len(), stderr.to_string()),
        Err(_) => failed_all(refs.len(), format!("gh answered {:?}", stdout.trim())),
    }
}

/// The answers of `refs` out of what `gh api graphql` printed for `branch_query`. A pull
/// request from a fork is not the branch's own, whatever its head is called; of the rest, the
/// newest that is open (or a draft) wins, else the newest.
pub(super) fn parse_branch_prs(
    stdout: &str,
    refs: &[&BranchRef],
) -> Vec<Result<Option<BranchPr>, String>> {
    let value: Value = match serde_json::from_str(stdout) {
        Ok(value) => value,
        Err(e) => {
            return refs
                .iter()
                .map(|_| Err(format!("cannot read gh's answer: {e}")))
                .collect();
        }
    };
    let errors = value
        .get("errors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    refs.iter()
        .enumerate()
        .map(|(i, r)| {
            let alias = format!("b{i}");
            let nodes_of = |list: &str| {
                value
                    .pointer(&format!("/data/{alias}/{list}/nodes"))
                    .and_then(Value::as_array)
            };
            let (Some(open), Some(recent)) = (nodes_of("open"), nodes_of("recent")) else {
                // The error that names this alias; failing that, the first one.
                let said = errors
                    .iter()
                    .find(|e| e.pointer("/path/0").and_then(Value::as_str) == Some(alias.as_str()))
                    .or_else(|| errors.first())
                    .and_then(|e| e.get("message").and_then(Value::as_str));
                return Err(said.unwrap_or("no such repository").to_string());
            };
            let own = |nodes: &Vec<Value>| -> Vec<BranchPr> {
                nodes
                    .iter()
                    .filter(|node| {
                        node.pointer("/headRepositoryOwner/login")
                            .and_then(Value::as_str)
                            .is_some_and(|login| login.eq_ignore_ascii_case(&r.owner))
                    })
                    .filter_map(branch_pr_of)
                    .collect()
            };
            // The newest open (or draft) one of the branch's own, else the newest of any state.
            Ok(own(open)
                .into_iter()
                .next()
                .or_else(|| own(recent).into_iter().next()))
        })
        .collect()
}

fn branch_pr_of(node: &Value) -> Option<BranchPr> {
    let draft = node
        .get("isDraft")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let state = match node.get("state").and_then(Value::as_str)? {
        "OPEN" if draft => "draft",
        "OPEN" => "open",
        "MERGED" => "merged",
        "CLOSED" => "closed",
        _ => return None,
    };
    Some(BranchPr {
        number: node.get("number").and_then(Value::as_u64)?,
        url: node.get("url").and_then(Value::as_str)?.to_string(),
        state: state.to_string(),
    })
}

#[cfg(test)]
mod tests;
