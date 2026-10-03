//! What `gh` says about pull requests: who has been notified of one, and the state of up to
//! fifty of them in one query.
//!
//! Everything goes through the `gh` binary, so the login, the host and the proxy are the ones
//! the person already set up. Nothing here writes: notifications are read and never marked as
//! read, since that would take the person's own inbox out from under them.

use std::collections::HashSet;
use std::time::Instant;

use serde_json::Value;

pub(super) use crate::infra::gh::{GhRun, run, run_with_input};

use super::task::PrState;
use crate::task::{self, CheckCounts, PrRef, PrStatus};

/// What `gh api -i` printed: the status line and headers, then the body.
pub(super) struct Included<'a> {
    pub status: u16,
    pub status_line: &'a str,
    headers: Vec<(&'a str, &'a str)>,
    pub body: &'a str,
}

impl Included<'_> {
    /// A header's value, whatever case it was sent in.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| *value)
    }
}

pub(super) fn parse_included(stdout: &str) -> Result<Included<'_>, String> {
    // The head ends at the first empty line, CRLF or LF; a 304 may have nothing after it.
    let cut = [("\r\n\r\n", 4), ("\n\n", 2)]
        .into_iter()
        .filter_map(|(sep, len)| stdout.find(sep).map(|at| (at, len)))
        .min();
    let (head, body) = match cut {
        Some((at, len)) => (&stdout[..at], &stdout[at + len..]),
        None => (stdout, ""),
    };
    let mut lines = head.lines();
    let status_line = lines.next().unwrap_or("").trim();
    let status = status_line
        .strip_prefix("HTTP/")
        .and_then(|rest| rest.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| format!("not an HTTP answer: {:?}", status_line))?;
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect();
    Ok(Included {
        status,
        status_line,
        headers,
        body,
    })
}

/// What the notifications of one host came to.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Notified {
    /// Nothing changed since the `If-Modified-Since` that was sent.
    NotModified { interval: Option<u64> },
    Changed {
        last_modified: Option<String>,
        /// How long GitHub asks to be left alone, in seconds.
        interval: Option<u64>,
        /// The pull requests with news.
        prs: HashSet<PrRef>,
        /// The repositories (`owner/repo`, lower case) that had a check suite finish: a bot
        /// reporting on a PR does not notify about the PR itself.
        check_repos: HashSet<String>,
        /// The page came back full, so there may be more news than it holds.
        full: bool,
    },
}

/// Ask `host` which threads the person takes part in have changed since `if_modified_since`.
///
/// `since` narrows the answer to the threads updated after the stamp (`all=true` would
/// otherwise fill the page with old, read threads). Only a read: `participating` keeps the
/// answer to what the person is in on, and nothing is marked as read. A 304 is the common answer and costs GitHub no rate limit, but `gh` exits
/// non-zero on it, so the output is read whatever the exit status was.
pub(super) fn notifications(
    host: &str,
    if_modified_since: Option<&str>,
    deadline: Instant,
) -> Result<Notified, String> {
    let header = if_modified_since.map(|value| format!("If-Modified-Since: {value}"));
    let mut args = vec!["api", "-i", "--hostname", host];
    if let Some(header) = &header {
        args.extend(["-H", header.as_str()]);
    }
    let mut endpoint = format!("notifications?participating=true&all=true&per_page={PAGE}");
    if let Some(iso) = if_modified_since.and_then(iso_from_http_date) {
        endpoint.push_str(&format!("&since={iso}"));
    }
    args.push(&endpoint);
    let run = run(None, &args, deadline)?;
    let failed = |run: &GhRun| {
        if run.stderr.is_empty() {
            format!("gh exited without an answer for {host}")
        } else {
            run.stderr.clone()
        }
    };
    let Ok(included) = parse_included(&run.stdout) else {
        return Err(failed(&run));
    };
    let interval = poll_interval(&included);
    match included.status {
        304 => Ok(Notified::NotModified { interval }),
        200 => {
            let seen = prs_in_notifications(included.body)?;
            Ok(Notified::Changed {
                last_modified: included.header("Last-Modified").map(str::to_string),
                interval,
                full: page_is_full(seen.threads),
                prs: seen.prs,
                check_repos: seen.check_repos,
            })
        }
        _ if !run.stderr.is_empty() => Err(run.stderr),
        _ => Err(included.status_line.to_string()),
    }
}

/// The longest `X-Poll-Interval` that is honoured: a proxy or a mistake that asks for days
/// would leave every card stale for as long.
const MAX_INTERVAL: u64 = 3600;

fn poll_interval(included: &Included<'_>) -> Option<u64> {
    included
        .header("X-Poll-Interval")
        .and_then(|v| v.parse::<u64>().ok())
        .map(|secs| secs.min(MAX_INTERVAL))
}

/// `2026-01-01T00:00:05Z` out of an HTTP date (`Thu, 01 Jan 2026 00:00:05 GMT`), which is the
/// form `since` takes and `Last-Modified` does not. `None` for anything else: the request is
/// then made without it.
pub(super) fn iso_from_http_date(date: &str) -> Option<String> {
    let mut parts = date.split_whitespace();
    parts.next().filter(|d| d.ends_with(','))?;
    let day: u32 = parts.next()?.parse().ok()?;
    let month = parts.next()?;
    let year: u32 = parts.next()?.parse().ok()?;
    let time = parts.next()?;
    (parts.next()? == "GMT" && parts.next().is_none()).then_some(())?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| *m == month)?
        + 1;
    let clock: Vec<u32> = time
        .split(':')
        .map(|n| n.parse().ok())
        .collect::<Option<_>>()?;
    let [h, m, sec] = clock[..] else { return None };
    let valid = (1..=31).contains(&day) && year >= 1970 && h < 24 && m < 60 && sec < 61;
    valid.then(|| format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{sec:02}Z"))
}

/// How many threads one request asks for. Only the first page is read, so a page that comes
/// back this full may have left news behind. (With `since`, the page holds only threads that
/// changed after the stamp, so a full one means that many really did.)
const PAGE: usize = 50;

pub(super) fn page_is_full(threads: usize) -> bool {
    threads >= PAGE
}

/// What a notifications body names.
pub(super) struct Seen {
    pub prs: HashSet<PrRef>,
    pub check_repos: HashSet<String>,
    /// How many threads the body held, of any kind.
    pub threads: usize,
}

/// The pull requests, and the repositories with a finished check suite, in a notifications
/// body. Other kinds of thread are not about a card.
pub(super) fn prs_in_notifications(body: &str) -> Result<Seen, String> {
    let list: Value =
        serde_json::from_str(body).map_err(|e| format!("cannot read the notifications: {e}"))?;
    let mut prs = HashSet::new();
    let mut check_repos = HashSet::new();
    let threads = list.as_array().map_or(0, Vec::len);
    for thread in list.as_array().into_iter().flatten() {
        let subject = thread.get("subject");
        let text = |value: Option<&Value>, key: &str| {
            value
                .and_then(|v| v.get(key))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        match text(subject, "type").as_deref() {
            Some("PullRequest") => {
                prs.extend(text(subject, "url").and_then(|url| task::pr_ref_from_api(&url)));
            }
            Some("CheckSuite") => {
                check_repos.extend(
                    text(thread.get("repository"), "full_name")
                        .map(|name| name.to_ascii_lowercase()),
                );
            }
            _ => {}
        }
    }
    Ok(Seen {
        prs,
        check_repos,
        threads,
    })
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
pub(super) struct PrsRead {
    /// One answer per reference, in the order they were given. A PR that could not be found is
    /// `Unreadable` here, which says something about that PR only.
    pub answers: Vec<Answer>,
    /// Why `gh` or GitHub could not be asked at all (no `gh`, no network, no login, an answer
    /// that is not the query's), when that is so for any of the queries. This is the poll
    /// failing, as opposed to a PR that is gone.
    pub failed: Option<String>,
}

/// Read each of `refs` in as few queries as the host split and the size cap allow.
pub(super) fn read_prs(refs: &[PrRef], deadline: Instant) -> PrsRead {
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
fn unread_error(answer: &Value) -> Option<String> {
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
fn first_error(stdout: &str) -> Option<String> {
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
        .take(task::ISSUE_TITLE_CAP)
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
