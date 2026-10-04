//! What `gh` says about pull requests: who has been notified of one.
//!
//! Everything goes through the `gh` binary, so the login, the host and the proxy are the ones
//! the person already set up. Nothing here writes: notifications are read and never marked as
//! read, since that would take the person's own inbox out from under them.

use std::collections::HashSet;
use std::time::Instant;

use serde_json::Value;

use crate::infra::gh::{GhRun, run};

use crate::task::{self, PrRef};

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

#[cfg(test)]
mod tests;
