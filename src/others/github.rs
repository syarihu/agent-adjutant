//! What GitHub says about the PRs that asked for your review: who you are, the search, the
//! details of each PR and the pages of its files, through `gh`.
//!
//! Every `gh` call goes through one runner argument, so the tests answer without a network.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use serde_json::Value;

use crate::infra::clock::stamp_from_iso;
use crate::infra::gh::GhRun;
use crate::task::{CheckCounts, ISSUE_TITLE_CAP, PrRef, checks, first_error, pr_ref, unread_error};

use super::model::{FileStat, MyReview, PrOpenState, ReviewState, Reviewer};

/// How many PRs one details query asks about. A PR here brings its files, reviews, requests and
/// commits, which is far heavier than the few fields the poll asks of fifty.
const DETAIL_CHUNK: usize = 10;
/// How many details queries run at once.
const DETAIL_IN_FLIGHT: usize = 4;
/// How many more pages of files are read for a PR with over 100 of them.
const FILE_PAGES: usize = 9;
/// What the search returns at most; a full answer may have left some out.
pub(super) const SEARCH_LIMIT: usize = 100;

/// Runs `gh` with these arguments and gives up at the deadline.
pub(super) type Gh<'a> = &'a (dyn Fn(&[&str], Instant) -> Result<GhRun, String> + Sync);

/// What a PR looks like on GitHub right now: everything a record takes from GitHub. The times
/// are adj stamps already.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Facts {
    pub title: String,
    pub author: Option<String>,
    pub base: String,
    pub head: String,
    pub head_sha: String,
    pub draft: bool,
    pub pr_state: PrOpenState,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    /// The first page of files.
    pub files: Vec<FileStat>,
    /// Where the next page of files starts, when there is one.
    pub files_after: Option<String>,
    pub ci: CheckCounts,
    pub reviewers: Vec<Reviewer>,
    pub my_review: Option<MyReview>,
    pub commits_since_review: Option<u32>,
    /// When the first commit after your reviewed one was committed; `None` when there is none
    /// or the reviewed commit is not among the last 100.
    pub first_push_at: Option<String>,
    pub requested: bool,
    pub requested_at: Option<String>,
}

/// What reading some PRs came to, one answer each.
pub(super) type Read = Vec<Result<Facts, String>>;

/// A PR to read: its key, its URL and its repository as GitHub spells it. What a search hit is,
/// and what a stored record turns into when the sync reads it again.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Hit {
    pub pr: PrRef,
    pub repo: String,
    pub url: String,
}

/// Why a PR was not read when the deadline came first.
pub(super) const OUT_OF_TIME: &str = "not read: the sync ran out of time";

fn answer(run: GhRun) -> Result<String, String> {
    if run.ok {
        return Ok(run.stdout);
    }
    Err(if run.stderr.is_empty() {
        "gh exited without succeeding".to_string()
    } else {
        run.stderr
    })
}

/// Who `gh` is logged in as. Also the check that `gh` is there and logged in at all.
pub(super) fn viewer(gh: Gh, deadline: Instant) -> Result<String, String> {
    let out = answer(gh(&["api", "user", "--jq", ".login"], deadline)?)?;
    let login = out.trim();
    if login.is_empty() {
        return Err("gh answered no login".to_string());
    }
    Ok(login.to_string())
}

/// The open PRs that asked for your review, among the repositories of `owners`, and whether the
/// search came back full. Team requests come too: `--review-requested` matches them, and the
/// details tell them apart.
pub(super) fn search(
    gh: Gh,
    owners: &[String],
    deadline: Instant,
) -> Result<(Vec<Hit>, bool), String> {
    let limit = SEARCH_LIMIT.to_string();
    let owners: Vec<String> = owners.iter().map(|o| format!("--owner={o}")).collect();
    let mut args = vec![
        "search",
        "prs",
        "--review-requested=@me",
        "--state=open",
        "--limit",
        &limit,
    ];
    args.extend(owners.iter().map(String::as_str));
    args.extend(["--json", "number,url,repository"]);
    let out = answer(gh(&args, deadline)?)?;
    parse_search(&out)
}

pub(super) fn parse_search(stdout: &str) -> Result<(Vec<Hit>, bool), String> {
    let value: Value =
        serde_json::from_str(stdout).map_err(|e| format!("cannot read gh's search answer: {e}"))?;
    let list = value.as_array().ok_or("gh's search answer is not a list")?;
    let hits = list
        .iter()
        .filter_map(|hit| {
            let url = hit.get("url")?.as_str()?;
            let pr = pr_ref(url, None)?;
            // Only the host the search ran against.
            if pr.host != "github.com" {
                return None;
            }
            let repo = hit
                .pointer("/repository/nameWithOwner")
                .and_then(Value::as_str)
                .map_or_else(|| pr.nwo(), str::to_string);
            Some(Hit {
                pr,
                repo,
                url: url.to_string(),
            })
        })
        .collect();
    Ok((hits, list.len() >= SEARCH_LIMIT))
}

const FILES: &str =
    "files(first:100){pageInfo{hasNextPage endCursor} nodes{path additions deletions changeType}}";

/// The query for `n` PRs: the values travel as variables (`me`, `o0`, `r0`, `p0`, ...), so
/// nothing from a record is spliced into the text.
pub(super) fn detail_query(n: usize) -> String {
    let variables: Vec<String> = (0..n)
        .map(|i| format!("$o{i}:String!,$r{i}:String!,$p{i}:Int!"))
        .collect();
    let aliases: Vec<String> = (0..n)
        .map(|i| {
            format!("p{i}:repository(owner:$o{i},name:$r{i}){{pullRequest(number:$p{i}){{...F}}}}")
        })
        .collect();
    format!(
        "query($me:String!,{}){{{}}} fragment F on PullRequest {{ \
         number url title state isDraft author{{login}} baseRefName headRefName headRefOid \
         additions deletions changedFiles {FILES} \
         reviewRequests(first:100){{nodes{{requestedReviewer{{__typename ...on User{{login}} \
         ...on Team{{combinedSlug}} ...on Bot{{login}} ...on Mannequin{{login}}}}}}}} \
         latestReviews(first:100){{nodes{{author{{login}} state submittedAt}}}} \
         myReviews:reviews(last:1,author:$me,states:[APPROVED,CHANGES_REQUESTED,COMMENTED,DISMISSED]){{nodes{{state submittedAt commit{{oid}}}}}} \
         requests:timelineItems(last:100,itemTypes:[REVIEW_REQUESTED_EVENT]){{nodes{{...on ReviewRequestedEvent{{createdAt requestedReviewer{{...on User{{login}}}}}}}}}} \
         commits(last:100){{nodes{{commit{{oid committedDate}}}}}} \
         lastCommit:commits(last:1){{nodes{{commit{{statusCheckRollup{{state \
         contexts(first:1){{checkRunCountsByState{{state count}} \
         statusContextCountsByState{{state count}}}}}}}}}}}} }}",
        variables.join(","),
        aliases.join(" ")
    )
}

/// Read each of `refs`, in chunks of `DETAIL_CHUNK`, `DETAIL_IN_FLIGHT` at a time. One answer per
/// ref, in order. A chunk that cannot start before the deadline fails its PRs rather than
/// holding the sync past it.
pub(super) fn read_details(gh: Gh, me: &str, refs: &[&PrRef], deadline: Instant) -> Read {
    let chunks: Vec<&[&PrRef]> = refs.chunks(DETAIL_CHUNK).collect();
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<Option<Read>>> = Mutex::new(vec![None; chunks.len()]);
    std::thread::scope(|scope| {
        for _ in 0..DETAIL_IN_FLIGHT.min(chunks.len()) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, Ordering::SeqCst);
                    let Some(chunk) = chunks.get(at) else { break };
                    let read = read_chunk(gh, me, chunk, deadline);
                    done.lock().unwrap_or_else(|e| e.into_inner())[at] = Some(read);
                }
            });
        }
    });
    done.into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .zip(chunks)
        .flat_map(|(read, chunk)| {
            // A chunk whose thread died is not read, which says so for each of its PRs.
            read.unwrap_or_else(|| {
                chunk
                    .iter()
                    .map(|_| Err("not read: the sync stopped".to_string()))
                    .collect()
            })
        })
        .collect()
}

fn read_chunk(gh: Gh, me: &str, refs: &[&PrRef], deadline: Instant) -> Read {
    let all = |why: String| -> Vec<Result<Facts, String>> {
        refs.iter().map(|_| Err(why.clone())).collect()
    };
    if Instant::now() >= deadline {
        return all(OUT_OF_TIME.to_string());
    }
    let query = format!("query={}", detail_query(refs.len()));
    let who = format!("me={me}");
    let mut owned: Vec<String> = Vec::new();
    for (i, r) in refs.iter().enumerate() {
        owned.push(format!("o{i}={}", r.owner));
        owned.push(format!("r{i}={}", r.repo));
        owned.push(format!("p{i}={}", r.number));
    }
    let mut args: Vec<&str> = vec!["api", "graphql", "-f", &query, "-f", &who];
    for i in 0..refs.len() {
        args.extend(["-f", &owned[i * 3], "-f", &owned[i * 3 + 1]]);
        args.extend(["-F", &owned[i * 3 + 2]]);
    }
    match gh(&args, deadline) {
        // Read whatever the exit status was: GraphQL answers a query that found some of its PRs
        // and not others with data and errors together, and `gh` exits 1.
        Ok(run) => match serde_json::from_str::<Value>(&run.stdout) {
            Ok(value) if value.get("data").is_some_and(Value::is_object) => {
                match unread_error(&value) {
                    Some(why) => all(why),
                    None => parse_details(&run.stdout, refs.len(), me),
                }
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

/// `n` answers out of what `gh api graphql` printed for `detail_query(n)`.
pub(super) fn parse_details(stdout: &str, n: usize, me: &str) -> Vec<Result<Facts, String>> {
    let value: Value = match serde_json::from_str(stdout) {
        Ok(value) => value,
        Err(e) => {
            return (0..n)
                .map(|_| Err(format!("cannot read gh's answer: {e}")))
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
                Some(pr) => parse_pr(pr, me),
                None => {
                    let said = errors
                        .iter()
                        .find(|e| {
                            e.pointer("/path/0").and_then(Value::as_str) == Some(alias.as_str())
                        })
                        .or_else(|| errors.first())
                        .and_then(|e| e.get("message").and_then(Value::as_str));
                    Err(said.unwrap_or("no such pull request").to_string())
                }
            }
        })
        .collect()
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn count(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn nodes<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value
        .pointer(&format!("/{key}/nodes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn file_stats(files: &Value) -> Vec<FileStat> {
    files
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| {
            Some(FileStat {
                path: text(f, "path")?,
                additions: count(f, "additions"),
                deletions: count(f, "deletions"),
                change: text(f, "changeType")
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            })
        })
        .collect()
}

/// Where the page after this one starts, when GitHub says there is one.
fn next_page(files: &Value) -> Option<String> {
    files
        .pointer("/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
        .filter(|&more| more)?;
    text(files.get("pageInfo")?, "endCursor").filter(|c| !c.is_empty())
}

/// What a review's state on GitHub comes to here; a pending review is not one.
fn review_state(state: &str) -> Option<ReviewState> {
    match state {
        "APPROVED" => Some(ReviewState::Approved),
        "CHANGES_REQUESTED" => Some(ReviewState::ChangesRequested),
        "COMMENTED" | "DISMISSED" => Some(ReviewState::Commented),
        _ => None,
    }
}

fn parse_pr(pr: &Value, me: &str) -> Result<Facts, String> {
    let pr_state = match text(pr, "state").as_deref() {
        Some("OPEN") => PrOpenState::Open,
        Some("MERGED") => PrOpenState::Merged,
        Some("CLOSED") => PrOpenState::Closed,
        other => return Err(format!("gh answered {:?}", other.unwrap_or(""))),
    };
    let head_sha = text(pr, "headRefOid").unwrap_or_default();
    let is_me = |login: Option<&str>| login.is_some_and(|l| l.eq_ignore_ascii_case(me));

    // Who was asked, a team by its `org/slug`.
    let mut reviewers: Vec<Reviewer> = Vec::new();
    let mut requested = false;
    for node in nodes(pr, "reviewRequests") {
        let Some(who) = node.get("requestedReviewer") else {
            continue;
        };
        let (login, team) = match text(who, "__typename").as_deref() {
            Some("Team") => (text(who, "combinedSlug"), true),
            _ => (text(who, "login"), false),
        };
        let Some(login) = login else { continue };
        requested |= !team && is_me(Some(&login));
        reviewers.push(Reviewer {
            login,
            team,
            state: ReviewState::Requested,
            at: None,
        });
    }
    for node in nodes(pr, "latestReviews") {
        let Some(state) = text(node, "state").as_deref().and_then(review_state) else {
            continue;
        };
        let Some(login) = node.pointer("/author/login").and_then(Value::as_str) else {
            continue;
        };
        if reviewers
            .iter()
            .any(|r| r.login.eq_ignore_ascii_case(login))
        {
            continue;
        }
        reviewers.push(Reviewer {
            login: login.to_string(),
            team: false,
            state,
            at: text(node, "submittedAt").and_then(|t| stamp_from_iso(&t)),
        });
    }

    // The latest request that named you: the current one while you are still asked.
    let requested_at = nodes(pr, "requests")
        .filter(|n| {
            is_me(
                n.pointer("/requestedReviewer/login")
                    .and_then(Value::as_str),
            )
        })
        .filter_map(|n| text(n, "createdAt").and_then(|t| stamp_from_iso(&t)))
        .max();

    let my_review = nodes(pr, "myReviews").next().and_then(|r| {
        Some(MyReview {
            state: review_state(&text(r, "state")?)?,
            submitted_at: stamp_from_iso(&text(r, "submittedAt")?)?,
            commit: r.pointer("/commit/oid")?.as_str()?.to_string(),
        })
    });
    let commits_since_review = my_review.as_ref().and_then(|m| {
        if m.commit == head_sha {
            return Some(0);
        }
        let oids: Vec<&str> = nodes(pr, "commits")
            .filter_map(|c| c.pointer("/commit/oid").and_then(Value::as_str))
            .collect();
        let at = oids.iter().position(|oid| *oid == m.commit)?;
        u32::try_from(oids.len() - 1 - at).ok()
    });
    let first_push_at = my_review
        .as_ref()
        .filter(|m| m.commit != head_sha)
        .and_then(|m| {
            let commits: Vec<&Value> = nodes(pr, "commits").collect();
            let at = commits.iter().position(|c| {
                c.pointer("/commit/oid").and_then(Value::as_str) == Some(m.commit.as_str())
            })?;
            let date = commits
                .get(at + 1)?
                .pointer("/commit/committedDate")?
                .as_str()?;
            stamp_from_iso(date)
        });

    let ci = nodes(pr, "lastCommit")
        .next()
        .and_then(|c| c.pointer("/commit/statusCheckRollup"))
        .filter(|rollup| rollup.is_object())
        .map(checks)
        .unwrap_or_default();
    let files = pr.get("files");
    Ok(Facts {
        title: text(pr, "title")
            .unwrap_or_default()
            .chars()
            .take(ISSUE_TITLE_CAP)
            .collect(),
        author: pr
            .pointer("/author/login")
            .and_then(Value::as_str)
            .map(str::to_string),
        base: text(pr, "baseRefName").unwrap_or_default(),
        head: text(pr, "headRefName").unwrap_or_default(),
        head_sha,
        draft: pr.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        pr_state,
        additions: count(pr, "additions"),
        deletions: count(pr, "deletions"),
        changed_files: count(pr, "changedFiles"),
        files: files.map(file_stats).unwrap_or_default(),
        files_after: files.and_then(next_page),
        ci,
        reviewers,
        my_review,
        commits_since_review,
        first_push_at,
        requested,
        requested_at,
    })
}

/// The files after `after`, up to `FILE_PAGES` more pages of 100, and the cursor to go on from
/// when the last page still had more. Sequential, and each page inside the deadline.
pub(super) fn more_files(
    gh: Gh,
    r: &PrRef,
    after: &str,
    deadline: Instant,
) -> Result<(Vec<FileStat>, Option<String>), String> {
    const QUERY: &str = "query($o:String!,$r:String!,$p:Int!,$after:String!){repository(owner:$o,name:$r){pullRequest(number:$p){\
        files(first:100,after:$after){pageInfo{hasNextPage endCursor} nodes{path additions deletions changeType}}}}}";
    let (query, owner, repo, number) = (
        format!("query={QUERY}"),
        format!("o={}", r.owner),
        format!("r={}", r.repo),
        format!("p={}", r.number),
    );
    let mut files = Vec::new();
    let mut cursor = Some(after.to_string());
    for _ in 0..FILE_PAGES {
        let Some(from) = cursor.clone() else { break };
        let from = format!("after={from}");
        let page = (|| {
            let out = answer(gh(
                &[
                    "api", "graphql", "-f", &query, "-f", &owner, "-f", &repo, "-F", &number, "-f",
                    &from,
                ],
                deadline,
            )?)?;
            let value: Value =
                serde_json::from_str(&out).map_err(|e| format!("cannot read gh's answer: {e}"))?;
            value
                .pointer("/data/repository/pullRequest/files")
                .filter(|f| f.is_object())
                .map(|page| (file_stats(page), next_page(page)))
                .ok_or_else(|| {
                    first_error(&out).unwrap_or_else(|| "GitHub answered without files".to_string())
                })
        })();
        match page {
            Ok((more, next)) => {
                files.extend(more);
                cursor = next;
            }
            // What was read stays, and the cursor says there is more: the caller sees a list
            // that is not complete, not a failure of the PR.
            Err(_) if !files.is_empty() => break,
            Err(why) => return Err(why),
        }
    }
    Ok((files, cursor))
}

#[cfg(test)]
pub(super) mod tests;
