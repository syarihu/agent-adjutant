//! What GitHub says about a task: issue and pull request URLs, and the issue and PR reads through `gh`.

use super::*;

use std::time::Instant;

use serde_json::Value;

use crate::infra::gh::run;

/// Whether `url` is an issue this tool knows how to read: an http(s) URL whose path is
/// `/<owner>/<repo>/issues/<number>`, on github.com or an enterprise host alike. Other
/// trackers, and pull request URLs, are left alone rather than guessed at.
pub fn fetchable_issue(url: &str) -> bool {
    issue_parts(url).is_some()
}

/// `owner/repo#N` for an issue `fetchable_issue` accepts: the name a task made from the issue
/// carries until the issue's own title has been read.
pub fn issue_ref(url: &str) -> Option<String> {
    let (owner, repo, number) = issue_parts(url)?;
    Some(format!("{owner}/{repo}#{number}"))
}

/// The owner, repository and number of an issue URL `fetchable_issue` accepts.
fn issue_parts(url: &str) -> Option<(&str, &str, &str)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    // Cut at the first `?` or `#` before looking at the path, so a '/' inside a query or
    // fragment can never pass for one in the path.
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (host, path) = rest.split_once('/')?;
    // One trailing slash after the number is the same issue.
    let path = path.strip_suffix('/').unwrap_or(path);
    let parts: Vec<&str> = path.split('/').collect();
    let ok = !host.is_empty()
        && parts.len() == 4
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && parts[2] == "issues"
        && !parts[3].is_empty()
        && parts[3].chars().all(|c| c.is_ascii_digit());
    ok.then(|| (parts[0], parts[1], parts[3]))
}

/// `text` cut to at most `cap` bytes, at a character boundary. `true` when something was cut.
fn cut_at(text: &str, cap: usize) -> (&str, bool) {
    if text.len() <= cap {
        return (text, false);
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

/// Build a snapshot from what `gh issue view --json title,body` printed.
pub fn snapshot_from_gh(json: &str, url: &str, stamp: &str) -> Result<IssueSnapshot, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("cannot read gh's answer: {e}"))?;
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or("gh's answer has no title")?;
    // An issue with no description comes back as null, not as an empty string.
    let body = value.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let title: String = title.chars().take(ISSUE_TITLE_CAP).collect();
    let (body, truncated) = cut_at(body, ISSUE_BODY_CAP);
    Ok(IssueSnapshot {
        url: url.to_string(),
        title,
        body: body.to_string(),
        truncated,
        fetched_at: stamp.to_string(),
    })
}

/// A pull request as GitHub names it. Owner and repository are kept in lower case, which is
/// how GitHub itself compares them, so two spellings of one PR are one key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrRef {
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

impl PrRef {
    /// `owner/repo`, the name a notification gives a repository by.
    pub fn nwo(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

/// Owner and repository names as GitHub allows them. Checked because they travel to `gh` as
/// values, and a name that starts with `-` would be read as a flag.
fn is_name(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('-')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn is_number(text: &str) -> Option<u64> {
    (!text.is_empty() && text.chars().all(|c| c.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn pr_ref_of(host: &str, owner: &str, repo: &str, number: &str) -> Option<PrRef> {
    let host_ok = !host.is_empty()
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    (host_ok && is_name(owner) && is_name(repo)).then_some(())?;
    Some(PrRef {
        host: host.to_ascii_lowercase(),
        owner: owner.to_ascii_lowercase(),
        repo: repo.to_ascii_lowercase(),
        number: is_number(number)?,
    })
}

/// The pull request a record's `pr` names: `https://HOST/OWNER/REPO/pull/N`, with whatever
/// follows the number (`/files`, a query, a fragment) ignored, or a bare `N` / `#N` when the
/// repository the board belongs to is known (`default`: the host its origin is on, and its
/// `owner/repo`). Anything else is not guessed at, and a value that starts with `-` is
/// refused before it can reach `gh`.
pub fn pr_ref(pr: &str, default: Option<(&str, &str)>) -> Option<PrRef> {
    let pr = pr.trim();
    if pr.starts_with('-') {
        return None;
    }
    if let Some(rest) = pr
        .strip_prefix("https://")
        .or_else(|| pr.strip_prefix("http://"))
    {
        let rest = rest.split(['?', '#']).next().unwrap_or("");
        let (host, path) = rest.split_once('/')?;
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() < 4 || parts[2] != "pull" {
            return None;
        }
        return pr_ref_of(host, parts[0], parts[1], parts[3]);
    }
    let number = pr.strip_prefix('#').unwrap_or(pr);
    let (host, nwo) = default?;
    let (owner, repo) = nwo.split_once('/')?;
    pr_ref_of(host, owner, repo, number)
}

/// The pull request an API URL of a notification's subject names:
/// `https://api.github.com/repos/O/R/pulls/N`, or `https://HOST/api/v3/repos/O/R/pulls/N` on
/// an enterprise host.
pub fn pr_ref_from_api(url: &str) -> Option<PrRef> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    let (host, path) = if host == "api.github.com" {
        ("github.com", path)
    } else {
        (host, path.strip_prefix("api/v3/")?)
    };
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 5 || parts[0] != "repos" || parts[3] != "pulls" {
        return None;
    }
    pr_ref_of(host, parts[1], parts[2], parts[4])
}

/// How long reading one issue may take. A person is waiting on the command or the click, and
/// an issue is one request, so this is shorter than a whole refresh's allowance.
const ISSUE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Run `gh` from `main` and return what it printed, or the reason it failed.
fn gh_output(main: &str, args: &[&str], deadline: std::time::Instant) -> Result<String, String> {
    let run = crate::infra::gh::run(Some(main), args, deadline)?;
    if run.ok {
        return Ok(run.stdout);
    }
    Err(if run.stderr.is_empty() {
        "gh exited without succeeding".to_string()
    } else {
        run.stderr
    })
}

/// Read one issue through `gh`, from the main checkout so a URL on another host is still
/// resolved with this machine's `gh` login.
pub fn read_issue(main: &str, url: &str) -> Result<IssueSnapshot, String> {
    // A value that starts with '-' would reach `gh` as a flag.
    if url.starts_with('-') {
        return Err(format!("not an issue: {url}"));
    }
    let deadline = std::time::Instant::now() + ISSUE_TIMEOUT;
    let json = gh_output(
        main,
        // `--` so the URL is only ever a positional, whatever it starts with.
        &["issue", "view", "--json", "title,body", "--", url],
        deadline,
    )?;
    snapshot_from_gh(&json, url, &store::stamp())
}

/// How many pull requests one query asks about. GraphQL prices a query by what it names, and
/// fifty aliases of one small fragment stay far inside GitHub's limits.
const PER_QUERY: usize = 50;

/// The query for `n` pull requests: the values travel as variables (`o0`, `r0`, `p0`, ...), so
/// nothing from a record is ever spliced into the text.
pub(super) fn graphql_query(n: usize) -> String {
    let variables: Vec<String> = (0..n)
        .map(|i| format!("$o{i}:String!,$r{i}:String!,$p{i}:Int!"))
        .collect();
    let aliases: Vec<String> = (0..n)
        .map(|i| {
            format!("p{i}:repository(owner:$o{i},name:$r{i}){{pullRequest(number:$p{i}){{...F}}}}")
        })
        .collect();
    format!(
        "query({}){{{}}} fragment F on PullRequest {{ state isDraft title reviewDecision \
         reviewRequests{{totalCount}} latestOpinionatedReviews(first:20){{nodes{{state}}}} \
         commits(last:1){{nodes{{commit{{statusCheckRollup{{state \
         contexts(first:1){{checkRunCountsByState{{state count}} \
         statusContextCountsByState{{state count}}}}}}}}}}}} }}",
        variables.join(","),
        aliases.join(" ")
    )
}

type Answer = (PrState, Option<PrStatus>);

/// What reading some pull requests came to.
pub struct PrsRead {
    /// One answer per reference, in the order they were given. A PR that could not be found is
    /// `Unreadable` here, which says something about that PR only.
    pub answers: Vec<Answer>,
    /// Why `gh` or GitHub could not be asked at all (no `gh`, no network, no login, an answer
    /// that is not the query's), when that is so for any of the queries. This is the poll
    /// failing, as opposed to a PR that is gone.
    pub failed: Option<String>,
}

/// Read each of `refs` in as few queries as the host split and the size cap allow.
pub fn read_prs(refs: &[PrRef], deadline: Instant) -> PrsRead {
    let mut answers: Vec<Option<Answer>> = vec![None; refs.len()];
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
            let asked: Vec<&PrRef> = chunk.iter().map(|&i| &refs[i]).collect();
            let (read, why) = read_chunk(host, &asked, deadline);
            failed = failed.or(why);
            for (&i, answer) in chunk.iter().zip(read) {
                answers[i] = Some(answer);
            }
        }
    }
    PrsRead {
        answers: answers
            .into_iter()
            .map(|a| a.unwrap_or_else(|| (PrState::Unreadable("not read".to_string()), None)))
            .collect(),
        failed,
    }
}

/// One query's answers, and why the query itself failed if it did.
fn read_chunk(host: &str, refs: &[&PrRef], deadline: Instant) -> (Vec<Answer>, Option<String>) {
    let all = |why: String| -> (Vec<Answer>, Option<String>) {
        (
            refs.iter()
                .map(|_| (PrState::Unreadable(why.clone()), None))
                .collect(),
            Some(why),
        )
    };
    let query = format!("query={}", graphql_query(refs.len()));
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
        // Read whatever the exit status was: GraphQL answers a query that found some of its
        // pull requests and not others with data and errors together, and `gh` exits 1.
        Ok(run) => match serde_json::from_str::<Value>(&run.stdout) {
            // Without `data` nothing was looked up: a rate limit, a login, a server error.
            Ok(value) if value.get("data").is_some_and(Value::is_object) => {
                (parse_graphql(&run.stdout, refs.len()), unread_error(&value))
            }
            Ok(_) => all(first_error(&run.stdout)
                .or_else(|| Some(run.stderr.clone()).filter(|s| !s.is_empty()))
                .unwrap_or_else(|| "GitHub answered without data".to_string())),
            Err(_) if !run.stderr.is_empty() => all(run.stderr),
            Err(_) => all(format!("gh answered {:?}", run.stdout.trim())),
        },
        Err(why) => all(why),
    }
}

/// The first error that says the read itself failed, as opposed to one about a single pull
/// request: a rate limit, an unavailable service, a timeout, an error with no type, or one that
/// names no alias. Those pass, and the round counts as a failed read. A typed error that names
/// an alias (`NOT_FOUND`, `FORBIDDEN`, `INSUFFICIENT_SCOPES`, ...) is that card's own business:
/// it will not go away by asking again, so it must not hold up the others.
pub(super) fn unread_error(answer: &Value) -> Option<String> {
    answer
        .get("errors")?
        .as_array()?
        .iter()
        .find(|e| {
            let kind = e.get("type").and_then(Value::as_str);
            let names_an_alias = e.pointer("/path/0").is_some_and(Value::is_string);
            let transient = matches!(
                kind,
                None | Some("RATE_LIMITED" | "SERVICE_UNAVAILABLE" | "TIMEOUT" | "INTERNAL")
            );
            transient || !names_an_alias
        })
        .map(|e| {
            e.get("message")
                .and_then(Value::as_str)
                .unwrap_or("GitHub reported an error")
                .to_string()
        })
}

/// The message of the first error in a GraphQL answer.
pub(super) fn first_error(stdout: &str) -> Option<String> {
    serde_json::from_str::<Value>(stdout)
        .ok()?
        .pointer("/errors/0/message")?
        .as_str()
        .map(str::to_string)
}

/// `n` answers out of what `gh api graphql` printed for `graphql_query(n)`. Anything that is
/// not a pull request this knows how to read is not guessed at.
pub(super) fn parse_graphql(stdout: &str, n: usize) -> Vec<Answer> {
    let value: Value = match serde_json::from_str(stdout) {
        Ok(value) => value,
        Err(e) => {
            return (0..n)
                .map(|_| {
                    (
                        PrState::Unreadable(format!("cannot read gh's answer: {e}")),
                        None,
                    )
                })
                .collect();
        }
    };
    let errors = value
        .get("errors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    (0..n)
        .map(|i| {
            let alias = format!("p{i}");
            let pr = value
                .pointer(&format!("/data/{alias}/pullRequest"))
                .filter(|pr| pr.is_object());
            match pr {
                Some(pr) => parse_pr(pr),
                None => {
                    // The error that names this alias; failing that, the first one: a rate
                    // limit or a login problem is not tied to any of them.
                    let said = errors
                        .iter()
                        .find(|e| {
                            e.pointer("/path/0").and_then(Value::as_str) == Some(alias.as_str())
                        })
                        .or_else(|| errors.first())
                        .and_then(|e| e.get("message").and_then(Value::as_str));
                    (
                        PrState::Unreadable(said.unwrap_or("no such pull request").to_string()),
                        None,
                    )
                }
            }
        })
        .collect()
}

fn parse_pr(pr: &Value) -> Answer {
    let word =
        |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let is_draft = pr.get("isDraft").and_then(Value::as_bool).unwrap_or(false);
    let state = word(pr, "state").unwrap_or_default();
    let (state, shown) = match state.as_str() {
        "OPEN" => (PrState::Open, if is_draft { "draft" } else { "open" }),
        "CLOSED" => (PrState::Closed, "closed"),
        "MERGED" => (PrState::Merged, "merged"),
        other => {
            return (PrState::Unreadable(format!("gh answered {other:?}")), None);
        }
    };
    let requested = pr
        .pointer("/reviewRequests/totalCount")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        > 0;
    // A review is owed only when somebody was asked: a repository that does not require
    // reviews says REVIEW_REQUIRED for nobody in particular, and that is not another person's
    // turn.
    let owed = if requested { "required" } else { "none" };
    let review = match word(pr, "reviewDecision").as_deref() {
        Some("APPROVED") => "approved",
        Some("CHANGES_REQUESTED") => "changes",
        Some("REVIEW_REQUIRED") => owed,
        // No decision is what a repository without required reviews always says; what the
        // latest review of each reviewer said is then the closest thing to one.
        None => {
            let states: Vec<&str> = pr
                .pointer("/latestOpinionatedReviews/nodes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|n| n.get("state").and_then(Value::as_str))
                .collect();
            if states.contains(&"CHANGES_REQUESTED") {
                "changes"
            } else if states.contains(&"APPROVED") {
                "approved"
            } else {
                owed
            }
        }
        _ => "none",
    };
    let rollup = pr.pointer("/commits/nodes/0/commit/statusCheckRollup");
    let ci = rollup.map(checks).unwrap_or_default();
    let title: String = word(pr, "title")
        .unwrap_or_default()
        .chars()
        .take(ISSUE_TITLE_CAP)
        .collect();
    (
        state,
        Some(PrStatus {
            state: shown.to_string(),
            title,
            review: review.to_string(),
            ci,
        }),
    )
}

/// The checks of a commit, counted. A run is a pass, a failure, or not finished; a status
/// context says the same in its own words.
fn checks(rollup: &Value) -> CheckCounts {
    let mut ci = CheckCounts::default();
    let counted = |key: &str, run: bool, ci: &mut CheckCounts| {
        let list = rollup
            .pointer(&format!("/contexts/{key}"))
            .and_then(Value::as_array);
        for entry in list.into_iter().flatten() {
            let state = entry.get("state").and_then(Value::as_str).unwrap_or("");
            let count = entry.get("count").and_then(Value::as_u64).unwrap_or(0) as u32;
            let slot = match (run, state) {
                (true, "SUCCESS" | "NEUTRAL" | "SKIPPED") | (false, "SUCCESS") => &mut ci.pass,
                (
                    true,
                    "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE",
                )
                | (false, "FAILURE" | "ERROR") => &mut ci.fail,
                _ => &mut ci.pending,
            };
            *slot += count;
        }
    };
    counted("checkRunCountsByState", true, &mut ci);
    counted("statusContextCountsByState", false, &mut ci);
    if ci == CheckCounts::default() {
        // The rollup has a verdict but the counts did not come: say what it says.
        match rollup.get("state").and_then(Value::as_str) {
            Some("FAILURE" | "ERROR") => ci.fail = 1,
            Some("PENDING" | "EXPECTED") => ci.pending = 1,
            _ => {}
        }
    }
    ci
}

#[cfg(test)]
mod tests;
