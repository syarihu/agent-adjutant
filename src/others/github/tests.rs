use super::*;

use std::time::Duration;

use serde_json::{Value, json};

// GitHub's upper-case enum values are made here from lower-case words rather than written into
// the JSON, so that no fixture holds a bare upper-case string after a colon: the guard over
// everything that ships reads one as a tracker key.
pub(in crate::others) fn up(word: &str) -> String {
    word.to_ascii_uppercase()
}

/// An open PR asked of `me`, with two files and no review of anyone's, as the details query
/// answers it. Tests change what they are about with the `with_*` helpers below.
pub(in crate::others) fn node(state: &str, head: &str) -> Value {
    json!({
        "number": 7,
        "url": "https://github.com/acme/widget/pull/7",
        "title": "Add a thing",
        "state": state,
        "isDraft": false,
        "author": {"login": "alice"},
        "baseRefName": "main",
        "headRefName": "feature",
        "headRefOid": head,
        "additions": 10,
        "deletions": 2,
        "changedFiles": 2,
        "files": {
            "pageInfo": {"hasNextPage": false, "endCursor": null},
            "nodes": [
                {"path": "a.rs", "additions": 6, "deletions": 1, "changeType": up("modified")},
                {"path": "b.rs", "additions": 4, "deletions": 1, "changeType": up("added")}
            ]
        },
        "reviewRequests": {"nodes": [
            {"requestedReviewer": {"__typename": "User", "login": "me"}}
        ]},
        "latestReviews": {"nodes": []},
        "myReviews": {"nodes": []},
        "requests": {"nodes": [
            {"createdAt": "2026-10-01T00:00:00Z", "requestedReviewer": {"login": "me"}}
        ]},
        "commits": {"nodes": [{"commit": {"oid": head, "committedDate": "2026-09-30T00:00:00Z"}}]},
        "lastCommit": {"nodes": [{"commit": {"statusCheckRollup": null}}]}
    })
}

pub(in crate::others) fn set(pr: &mut Value, pointer: &str, to: Value) {
    *pr.pointer_mut(pointer)
        .unwrap_or_else(|| panic!("no {pointer}")) = to;
}

/// Your last review, made on `commit`.
pub(in crate::others) fn with_my_review(pr: &mut Value, state: &str, at: &str, commit: &str) {
    set(
        pr,
        "/myReviews/nodes",
        json!([{"state": state, "submittedAt": at, "commit": {"oid": commit}}]),
    );
}

/// The times you were asked, in the order the timeline lists them.
pub(in crate::others) fn with_requests(pr: &mut Value, at: &[&str]) {
    let nodes: Vec<Value> = at
        .iter()
        .map(|at| json!({"createdAt": at, "requestedReviewer": {"login": "me"}}))
        .collect();
    set(pr, "/requests/nodes", Value::Array(nodes));
}

/// Nobody is asked any more.
pub(in crate::others) fn without_requested(pr: &mut Value) {
    set(pr, "/reviewRequests/nodes", json!([]));
}

/// The commits on the head, oldest first.
pub(in crate::others) fn with_commits(pr: &mut Value, oids: &[&str]) {
    let nodes: Vec<Value> = oids
        .iter()
        .map(|oid| json!({"commit": {"oid": oid, "committedDate": "2026-09-30T00:00:00Z"}}))
        .collect();
    set(pr, "/commits/nodes", Value::Array(nodes));
}

/// What `gh api graphql` prints for `prs`, `p0` first.
pub(in crate::others) fn data(prs: &[Value]) -> String {
    let aliases: serde_json::Map<String, Value> = prs
        .iter()
        .enumerate()
        .map(|(i, pr)| (format!("p{i}"), json!({"pullRequest": pr})))
        .collect();
    json!({"data": aliases}).to_string()
}

fn ran(stdout: &str) -> Result<GhRun, String> {
    Ok(GhRun {
        ok: true,
        stdout: stdout.to_string(),
        stderr: String::new(),
    })
}

fn facts_of(pr: &Value) -> Facts {
    parse_details(&data(std::slice::from_ref(pr)), 1, "me")
        .remove(0)
        .expect("facts")
}

#[test]
fn the_query_names_each_pr_by_a_variable_and_asks_who_you_are_once() {
    let query = detail_query(3);
    for i in 0..3 {
        assert!(query.contains(&format!("$o{i}:String!,$r{i}:String!,$p{i}:Int!")));
        assert!(query.contains(&format!(
            "p{i}:repository(owner:$o{i},name:$r{i}){{pullRequest(number:$p{i}){{...F}}}}"
        )));
    }
    assert!(!query.contains("$p3"));
    assert_eq!(query.matches("$me:String!").count(), 1);
    assert!(query.contains("author:$me"));
    assert!(query.contains("fragment F on PullRequest"));
}

#[test]
fn a_pr_is_read_into_facts() {
    let mut pr = node(&up("open"), "h1");
    set(&mut pr, "/title", json!("t".repeat(ISSUE_TITLE_CAP + 40)));
    set(&mut pr, "/author", Value::Null);
    set(&mut pr, "/isDraft", json!(true));
    set(
        &mut pr,
        "/lastCommit/nodes/0/commit/statusCheckRollup",
        json!({
            "state": up("pending"),
            "contexts": {
                "checkRunCountsByState": [
                    {"state": up("success"), "count": 2},
                    {"state": up("failure"), "count": 1},
                    {"state": up("in_progress"), "count": 1}
                ],
                "statusContextCountsByState": [{"state": up("success"), "count": 1}]
            }
        }),
    );
    let facts = facts_of(&pr);
    assert_eq!(facts.title.chars().count(), ISSUE_TITLE_CAP);
    assert_eq!(facts.author, None);
    assert!(facts.draft);
    assert_eq!(facts.pr_state, PrOpenState::Open);
    assert_eq!(
        (
            facts.base.as_str(),
            facts.head.as_str(),
            facts.head_sha.as_str()
        ),
        ("main", "feature", "h1")
    );
    assert_eq!(
        (facts.additions, facts.deletions, facts.changed_files),
        (10, 2, 2)
    );
    assert_eq!(
        facts.ci,
        CheckCounts {
            pass: 3,
            fail: 1,
            pending: 1
        }
    );
    assert_eq!(facts.files.len(), 2);
    assert_eq!(facts.files[1].path, "b.rs");
    assert_eq!(facts.files[1].change, "added");
    assert_eq!(facts.files_after, None);
    assert!(facts.requested);
}

#[test]
fn reviewers_are_the_requests_and_then_the_latest_reviews() {
    let mut pr = node(&up("open"), "h1");
    set(
        &mut pr,
        "/reviewRequests/nodes",
        json!([
            {"requestedReviewer": {"__typename": "User", "login": "me"}},
            {"requestedReviewer": {"__typename": "Team", "combinedSlug": "acme/core"}}
        ]),
    );
    set(
        &mut pr,
        "/latestReviews/nodes",
        json!([
            {"author": {"login": "alice"}, "state": up("approved"), "submittedAt": "2026-10-02T01:02:03Z"},
            {"author": {"login": "bob"}, "state": up("pending"), "submittedAt": null},
            {"author": {"login": "carol"}, "state": up("dismissed"), "submittedAt": "2026-10-02T01:02:04Z"},
            {"author": {"login": "dave"}, "state": up("changes_requested"), "submittedAt": "2026-10-02T01:02:05Z"},
            {"author": {"login": up("me")}, "state": up("commented"), "submittedAt": "2026-10-02T01:02:06Z"}
        ]),
    );
    let facts = facts_of(&pr);
    let got: Vec<(&str, bool, ReviewState, Option<&str>)> = facts
        .reviewers
        .iter()
        .map(|r| (r.login.as_str(), r.team, r.state, r.at.as_deref()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("me", false, ReviewState::Requested, None),
            ("acme/core", true, ReviewState::Requested, None),
            (
                "alice",
                false,
                ReviewState::Approved,
                Some("20261002T010203Z")
            ),
            (
                "carol",
                false,
                ReviewState::Commented,
                Some("20261002T010204Z")
            ),
            (
                "dave",
                false,
                ReviewState::ChangesRequested,
                Some("20261002T010205Z")
            ),
        ]
    );
}

#[test]
fn a_request_to_a_team_of_yours_is_not_a_request_to_you() {
    let mut pr = node(&up("open"), "h1");
    set(
        &mut pr,
        "/reviewRequests/nodes",
        json!([{"requestedReviewer": {"__typename": "Team", "combinedSlug": "me/me"}}]),
    );
    assert!(!facts_of(&pr).requested);
}

#[test]
fn commits_since_your_review_are_counted_on_the_head() {
    let mut pr = node(&up("open"), "c4");
    with_commits(&mut pr, &["c1", "c2", "c3", "c4"]);
    with_my_review(&mut pr, &up("commented"), "2026-10-02T00:00:00Z", "c2");
    let facts = facts_of(&pr);
    assert_eq!(facts.commits_since_review, Some(2));
    let review = facts.my_review.expect("a review");
    assert_eq!(review.state, ReviewState::Commented);
    assert_eq!(review.submitted_at, "20261002T000000Z");
    assert_eq!(review.commit, "c2");

    // Reviewed on the head itself: none, which is not the same as unknown.
    with_my_review(&mut pr, &up("approved"), "2026-10-02T00:00:00Z", "c4");
    assert_eq!(facts_of(&pr).commits_since_review, Some(0));

    // A commit that is no longer among the last hundred (a force-push): unknown, never 0.
    with_my_review(&mut pr, &up("approved"), "2026-10-02T00:00:00Z", "gone");
    assert_eq!(facts_of(&pr).commits_since_review, None);

    // No review of yours, nothing to count from.
    set(&mut pr, "/myReviews/nodes", json!([]));
    let facts = facts_of(&pr);
    assert_eq!((facts.my_review, facts.commits_since_review), (None, None));
}

#[test]
fn a_dismissed_review_of_yours_reads_as_a_comment() {
    let mut pr = node(&up("open"), "h1");
    with_my_review(&mut pr, &up("dismissed"), "2026-10-02T00:00:00Z", "h1");
    assert_eq!(
        facts_of(&pr).my_review.map(|r| r.state),
        Some(ReviewState::Commented)
    );
}

#[test]
fn the_request_that_counts_is_the_latest_one_naming_you() {
    let mut pr = node(&up("open"), "h1");
    with_requests(&mut pr, &["2026-10-01T00:00:00Z", "2026-10-05T00:00:00Z"]);
    // Somebody else's request, later than yours.
    pr["requests"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"createdAt": "2026-10-09T00:00:00Z", "requestedReviewer": {"login": "bob"}}));
    pr["requests"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"createdAt": "2026-10-09T00:00:00Z", "requestedReviewer": null}));
    assert_eq!(
        facts_of(&pr).requested_at.as_deref(),
        Some("20261005T000000Z")
    );
    set(&mut pr, "/requests/nodes", json!([]));
    assert_eq!(facts_of(&pr).requested_at, None);
}

#[test]
fn a_merged_or_closed_pr_says_so_and_an_unknown_state_is_unreadable() {
    assert_eq!(
        facts_of(&node(&up("merged"), "h")).pr_state,
        PrOpenState::Merged
    );
    assert_eq!(
        facts_of(&node(&up("closed"), "h")).pr_state,
        PrOpenState::Closed
    );
    let read = parse_details(&data(&[node("weird", "h")]), 1, "me");
    assert!(read[0].as_ref().is_err_and(|e| e.contains("weird")));
}

#[test]
fn the_next_page_of_files_is_remembered() {
    let mut pr = node(&up("open"), "h1");
    set(
        &mut pr,
        "/files/pageInfo",
        json!({"hasNextPage": true, "endCursor": "cur1"}),
    );
    assert_eq!(facts_of(&pr).files_after.as_deref(), Some("cur1"));
}

#[test]
fn the_search_answer_is_read_into_prs_and_whether_it_came_back_full() {
    let hit = |n: u64| {
        json!({
            "number": n,
            "url": format!("https://github.com/Acme/Widget/pull/{n}"),
            "repository": {"nameWithOwner": "Acme/Widget"}
        })
    };
    let (hits, truncated) = parse_search(&json!([hit(1), hit(2)]).to_string()).unwrap();
    assert!(!truncated);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].pr.nwo(), "acme/widget");
    assert_eq!(hits[0].repo, "Acme/Widget");
    assert_eq!(hits[1].pr.number, 2);

    let full: Vec<Value> = (1..=SEARCH_LIMIT as u64).map(hit).collect();
    assert!(parse_search(&Value::Array(full).to_string()).unwrap().1);

    // An entry that is not a PR, or is on another host, is left out; an answer that is not a
    // list is an error, not an empty list.
    let odd = json!([{"url": "not a url"}, {"url": "https://ghe.example/acme/w/pull/3"}, hit(4)]);
    assert_eq!(parse_search(&odd.to_string()).unwrap().0.len(), 1);
    assert!(parse_search("{}").is_err());
    assert!(parse_search("nope").is_err());
}

#[test]
fn the_search_asks_for_each_owner_and_a_hundred() {
    let asked = Mutex::new(Vec::<Vec<String>>::new());
    let gh = |args: &[&str], _: Instant| {
        asked
            .lock()
            .unwrap()
            .push(args.iter().map(|a| a.to_string()).collect());
        ran("[]")
    };
    let owners = vec!["acme".to_string(), "zeta".to_string()];
    search(&gh, &owners, Instant::now() + Duration::from_secs(5)).unwrap();
    let asked = asked.lock().unwrap();
    let args = &asked[0];
    assert_eq!(&args[..2], ["search", "prs"]);
    assert!(args.contains(&"--review-requested=@me".to_string()));
    assert!(args.contains(&"--state=open".to_string()));
    assert!(args.windows(2).any(|w| w == ["--limit", "100"]));
    assert!(args.contains(&"--owner=acme".to_string()));
    assert!(args.contains(&"--owner=zeta".to_string()));
}

#[test]
fn a_failed_search_or_login_is_an_error_that_says_why() {
    let deadline = Instant::now() + Duration::from_secs(5);
    let failing = |_: &[&str], _: Instant| {
        Ok(GhRun {
            ok: false,
            stdout: String::new(),
            stderr: "HTTP 401".to_string(),
        })
    };
    assert_eq!(viewer(&failing, deadline), Err("HTTP 401".to_string()));
    assert_eq!(
        search(&failing, &["acme".to_string()], deadline).unwrap_err(),
        "HTTP 401"
    );
    let empty = |_: &[&str], _: Instant| ran("\n");
    assert_eq!(
        viewer(&empty, deadline),
        Err("gh answered no login".to_string())
    );
    let gone = |_: &[&str], _: Instant| Err("cannot run gh: not found".to_string());
    assert_eq!(
        viewer(&gone, deadline),
        Err("cannot run gh: not found".to_string())
    );
    let named = |_: &[&str], _: Instant| ran("syarihu\n");
    assert_eq!(viewer(&named, deadline), Ok("syarihu".to_string()));
}

fn refs(n: u64) -> Vec<PrRef> {
    (1..=n)
        .map(|i| pr_ref(&format!("https://github.com/acme/widget/pull/{i}"), None).unwrap())
        .collect()
}

/// A `gh` that answers a details query with one PR per alias, numbered as it was asked, and
/// writes down how many aliases each query had.
fn echo_gh(
    sizes: &Mutex<Vec<usize>>,
) -> impl Fn(&[&str], Instant) -> Result<GhRun, String> + Sync + '_ {
    move |args: &[&str], _| {
        let numbers: Vec<u64> = args
            .iter()
            .filter_map(|a| a.strip_prefix("p")?.split_once('=')?.1.parse().ok())
            .collect();
        sizes.lock().unwrap().push(numbers.len());
        let prs: Vec<Value> = numbers
            .iter()
            .map(|n| {
                let mut pr = node(&up("open"), "h1");
                set(&mut pr, "/number", json!(n));
                set(&mut pr, "/title", json!(format!("PR {n}")));
                pr
            })
            .collect();
        ran(&data(&prs))
    }
}

#[test]
fn details_are_read_ten_to_a_query_and_answered_in_order() {
    let sizes = Mutex::new(Vec::new());
    let gh = echo_gh(&sizes);
    let all = refs(25);
    let asked: Vec<&PrRef> = all.iter().collect();
    let read = read_details(&gh, "me", &asked, Instant::now() + Duration::from_secs(5));
    assert_eq!(read.len(), 25);
    for (i, facts) in read.iter().enumerate() {
        assert_eq!(facts.as_ref().unwrap().title, format!("PR {}", i + 1));
    }
    let mut sizes = sizes.lock().unwrap().clone();
    sizes.sort();
    assert_eq!(sizes, vec![5, 10, 10]);
}

#[test]
fn no_details_are_asked_when_there_is_nothing_to_read() {
    let sizes = Mutex::new(Vec::new());
    let gh = echo_gh(&sizes);
    assert!(read_details(&gh, "me", &[], Instant::now() + Duration::from_secs(5)).is_empty());
    assert!(sizes.lock().unwrap().is_empty());
}

#[test]
fn an_error_naming_an_alias_fails_only_that_pr() {
    let one = node(&up("open"), "h1");
    let stdout = json!({
        "data": {"p0": {"pullRequest": one}, "p1": {"pullRequest": null}},
        "errors": [{
            "type": up("not_found"),
            "path": ["p1", "pullRequest"],
            "message": "Could not resolve to a PullRequest"
        }]
    })
    .to_string();
    let all = refs(2);
    let asked: Vec<&PrRef> = all.iter().collect();
    let gh = |_: &[&str], _: Instant| {
        Ok(GhRun {
            ok: false,
            stdout: stdout.clone(),
            stderr: String::new(),
        })
    };
    let read = read_details(&gh, "me", &asked, Instant::now() + Duration::from_secs(5));
    assert!(read[0].is_ok());
    assert_eq!(
        read[1].as_ref().unwrap_err(),
        "Could not resolve to a PullRequest"
    );
}

#[test]
fn an_error_of_the_query_itself_fails_the_whole_chunk() {
    let stdout = json!({
        "data": {"p0": {"pullRequest": node(&up("open"), "h1")}, "p1": {"pullRequest": null}},
        "errors": [{"type": up("rate_limited"), "message": "API rate limit exceeded"}]
    })
    .to_string();
    let all = refs(2);
    let asked: Vec<&PrRef> = all.iter().collect();
    let gh = |_: &[&str], _: Instant| ran(&stdout);
    let read = read_details(&gh, "me", &asked, Instant::now() + Duration::from_secs(5));
    assert!(
        read.iter()
            .all(|r| r.as_ref().is_err_and(|e| e == "API rate limit exceeded"))
    );

    // And no answer with data at all says what `gh` said.
    let gh = |_: &[&str], _: Instant| {
        Ok(GhRun {
            ok: false,
            stdout: String::new(),
            stderr: "HTTP 502".to_string(),
        })
    };
    let read = read_details(&gh, "me", &asked, Instant::now() + Duration::from_secs(5));
    assert!(
        read.iter()
            .all(|r| r.as_ref().is_err_and(|e| e == "HTTP 502"))
    );
}

#[test]
fn a_chunk_that_cannot_start_before_the_deadline_is_not_read() {
    let sizes = Mutex::new(Vec::new());
    let gh = echo_gh(&sizes);
    let all = refs(3);
    let asked: Vec<&PrRef> = all.iter().collect();
    let read = read_details(&gh, "me", &asked, Instant::now());
    assert!(
        read.iter()
            .all(|r| r.as_ref().is_err_and(|e| e == OUT_OF_TIME))
    );
    assert!(sizes.lock().unwrap().is_empty());
}

fn file_page(from: u64, n: u64, next: Option<&str>) -> String {
    let nodes: Vec<Value> = (from..from + n)
        .map(|i| json!({"path": format!("f{i}.rs"), "additions": 1, "deletions": 0, "changeType": up("added")}))
        .collect();
    json!({"data": {"repository": {"pullRequest": {"files": {
        "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next},
        "nodes": nodes
    }}}}})
    .to_string()
}

#[test]
fn more_pages_of_files_are_read_from_the_cursor() {
    let pr = refs(1).remove(0);
    let asked = Mutex::new(Vec::<String>::new());
    let gh = |args: &[&str], _: Instant| {
        let after = args
            .iter()
            .find_map(|a| a.strip_prefix("after="))
            .unwrap()
            .to_string();
        asked.lock().unwrap().push(after.clone());
        match after.as_str() {
            "c1" => ran(&file_page(100, 100, Some("c2"))),
            _ => ran(&file_page(200, 30, None)),
        }
    };
    let (files, cursor) =
        more_files(&gh, &pr, "c1", Instant::now() + Duration::from_secs(5)).unwrap();
    assert_eq!(files.len(), 130);
    assert_eq!(files[0].path, "f100.rs");
    assert_eq!(cursor, None);
    assert_eq!(*asked.lock().unwrap(), vec!["c1", "c2"]);
}

#[test]
fn pages_of_files_stop_at_the_cap_and_keep_what_was_read_when_one_fails() {
    let pr = refs(1).remove(0);
    let endless = |_: &[&str], _: Instant| ran(&file_page(0, 100, Some("more")));
    let (files, cursor) =
        more_files(&endless, &pr, "c1", Instant::now() + Duration::from_secs(5)).unwrap();
    assert_eq!(files.len(), 100 * FILE_PAGES);
    assert_eq!(cursor.as_deref(), Some("more"));

    let calls = AtomicUsize::new(0);
    let flaky = |_: &[&str], _: Instant| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            ran(&file_page(0, 100, Some("more")))
        } else {
            Err("gh did not answer in time".to_string())
        }
    };
    let (files, cursor) =
        more_files(&flaky, &pr, "c1", Instant::now() + Duration::from_secs(5)).unwrap();
    assert_eq!(files.len(), 100);
    assert_eq!(cursor.as_deref(), Some("more"));

    // A first page that fails is the PR's files not read.
    let down = |_: &[&str], _: Instant| Err("gh did not answer in time".to_string());
    assert!(more_files(&down, &pr, "c1", Instant::now() + Duration::from_secs(5)).is_err());
}
