use super::*;

// GitHub's upper-case enum values are passed in rather than written into the JSON, so
// that no fixture holds a bare upper-case string after a colon: the guard over everything
// that ships reads one as a tracker key.
fn pr_json(
    state: &str,
    draft: bool,
    decision: Option<&str>,
    requests: u64,
    counts: &str,
) -> String {
    let decision = decision.map_or("null".to_string(), |d| format!("\"{d}\""));
    format!(
        r#"{{"state":"{state}","isDraft":{draft},"title":"Add a thing","reviewDecision":{decision},"reviewRequests":{{"totalCount":{requests}}},"latestOpinionatedReviews":{{"nodes":[]}},"commits":{{"nodes":[{{"commit":{{"statusCheckRollup":{counts}}}}}]}}}}"#
    )
}

fn count(state: &str, n: u32) -> String {
    format!(r#"{{"state":"{state}","count":{n}}}"#)
}

fn rollup(rollup_state: &str, runs: &[String], contexts: &[String]) -> String {
    format!(
        r#"{{"state":"{rollup_state}","contexts":{{"checkRunCountsByState":[{}],"statusContextCountsByState":[{}]}}}}"#,
        runs.join(","),
        contexts.join(",")
    )
}

fn data(prs: &[String]) -> String {
    let aliases: Vec<String> = prs
        .iter()
        .enumerate()
        .map(|(i, pr)| format!(r#""p{i}":{{"pullRequest":{pr}}}"#))
        .collect();
    format!(r#"{{"data":{{{}}}}}"#, aliases.join(","))
}

fn one(pr: String) -> (PrState, PrStatus) {
    let (state, status) = parse_graphql(&data(&[pr]), 1).remove(0);
    (state, status.expect("a summary"))
}

#[test]
fn an_answer_with_headers_is_split_into_status_headers_and_body() {
    let crlf = "HTTP/2.0 200 OK\r\nLast-Modified: Thu, 01 Jan 2026 00:00:00 GMT\r\nx-poll-interval: 60\r\n\r\n[1]";
    let got = parse_included(crlf).unwrap();
    assert_eq!(got.status, 200);
    assert_eq!(
        got.header("Last-Modified"),
        Some("Thu, 01 Jan 2026 00:00:00 GMT")
    );
    assert_eq!(got.header("X-Poll-Interval"), Some("60"));
    assert_eq!(got.body, "[1]");
    let lf = "HTTP/2.0 200 OK\nX-Poll-Interval: 30\n\n[2]";
    let got = parse_included(lf).unwrap();
    assert_eq!(
        (got.header("x-poll-interval"), got.body),
        (Some("30"), "[2]")
    );
}

#[test]
fn a_304_is_headers_and_nothing_else() {
    let got = parse_included("HTTP/2.0 304 Not Modified\r\nX-Poll-Interval: 60\r\n\r\n").unwrap();
    assert_eq!((got.status, got.body), (304, ""));
    let got = parse_included("HTTP/2.0 304 Not Modified\n").unwrap();
    assert_eq!((got.status, got.header("X-Poll-Interval")), (304, None));
}

#[test]
fn what_is_not_an_http_answer_is_refused() {
    for odd in ["", "gh: Bad credentials", "[]", "HTTP/2.0 x\n\n"] {
        assert!(parse_included(odd).is_err(), "{odd:?}");
    }
}

#[test]
fn only_pull_requests_and_finished_suites_are_picked_out_of_the_notifications() {
    let body = r#"[
        {"subject":{"type":"PullRequest","url":"https://api.github.com/repos/acme/widget/pulls/7"},"repository":{"full_name":"acme/widget"}},
        {"subject":{"type":"Issue","url":"https://api.github.com/repos/acme/widget/issues/8"},"repository":{"full_name":"acme/widget"}},
        {"subject":{"type":"CheckSuite","url":null},"repository":{"full_name":"Acme/Gadget"}},
        {"subject":{"type":"PullRequest","url":"https://api.github.com/repos/acme/widget/pulls/x"},"repository":{"full_name":"acme/widget"}},
        {"subject":{"type":"Release"}}
    ]"#;
    let seen = prs_in_notifications(body).unwrap();
    assert_eq!(seen.prs.len(), 1);
    assert!(
        seen.prs
            .iter()
            .any(|p| p.nwo() == "acme/widget" && p.number == 7)
    );
    assert_eq!(seen.check_repos, HashSet::from(["acme/gadget".to_string()]));
    assert_eq!(seen.threads, 5);
    assert!(prs_in_notifications("not json").is_err());
}

#[test]
fn the_query_names_each_pull_request_by_a_variable() {
    let query = graphql_query(3);
    for i in 0..3 {
        assert!(query.contains(&format!("$o{i}:String!,$r{i}:String!,$p{i}:Int!")));
        assert!(query.contains(&format!(
            "p{i}:repository(owner:$o{i},name:$r{i}){{pullRequest(number:$p{i}){{...F}}}}"
        )));
    }
    assert!(!query.contains("p3"));
    assert!(query.contains("fragment F on PullRequest"));
}

#[test]
fn a_pr_is_read_into_a_state_and_a_summary() {
    let checks = rollup(
        "PENDING",
        &[
            count("SUCCESS", 2),
            count("SKIPPED", 1),
            count("FAILURE", 1),
            count("IN_PROGRESS", 1),
        ],
        &[count("SUCCESS", 1), count("ERROR", 1), count("PENDING", 1)],
    );
    let (state, got) = one(pr_json("OPEN", false, Some("APPROVED"), 0, &checks));
    assert_eq!(state, PrState::Open);
    assert_eq!(
        (got.state.as_str(), got.title.as_str()),
        ("open", "Add a thing")
    );
    assert_eq!(got.review, "approved");
    assert_eq!(
        got.ci,
        CheckCounts {
            pass: 4,
            fail: 2,
            pending: 2
        }
    );
}

#[test]
fn a_draft_is_an_open_pr_marked_so_and_only_while_open() {
    let (state, got) = one(pr_json("OPEN", true, Some("REVIEW_REQUIRED"), 1, "null"));
    assert_eq!((state, got.state.as_str()), (PrState::Open, "draft"));
    assert_eq!(got.review, "required");
    let (state, got) = one(pr_json("MERGED", true, None, 0, "null"));
    assert_eq!((state, got.state.as_str()), (PrState::Merged, "merged"));
}

#[test]
fn a_merged_or_closed_pr_and_the_reviews_gh_names() {
    let (state, got) = one(pr_json("MERGED", false, Some("APPROVED"), 0, "null"));
    assert_eq!((state, got.state.as_str()), (PrState::Merged, "merged"));
    let (state, got) = one(pr_json(
        "CLOSED",
        false,
        Some("CHANGES_REQUESTED"),
        0,
        "null",
    ));
    assert_eq!((state, got.state.as_str()), (PrState::Closed, "closed"));
    assert_eq!(got.review, "changes");
}

#[test]
fn no_decision_is_a_review_owed_only_when_somebody_was_asked() {
    let (_, asked) = one(pr_json("OPEN", false, None, 1, "null"));
    assert_eq!(asked.review, "required");
    let (_, nobody) = one(pr_json("OPEN", false, None, 0, "null"));
    assert_eq!(nobody.review, "none");
    // A decision GitHub names wins over the requests.
    let (_, approved) = one(pr_json("OPEN", false, Some("APPROVED"), 2, "null"));
    assert_eq!(approved.review, "approved");
    let (_, odd) = one(pr_json("OPEN", false, Some("SOMETHING_NEW"), 2, "null"));
    assert_eq!(odd.review, "none");
}

/// `pr_json` with the latest review of each reviewer given as states.
fn reviewed(decision: Option<&str>, requests: u64, states: &[&str]) -> String {
    let nodes: Vec<String> = states
        .iter()
        .map(|state| format!(r#"{{"state":"{state}"}}"#))
        .collect();
    pr_json("OPEN", false, decision, requests, "null").replace(
        r#""latestOpinionatedReviews":{"nodes":[]}"#,
        &format!(
            r#""latestOpinionatedReviews":{{"nodes":[{}]}}"#,
            nodes.join(",")
        ),
    )
}

#[test]
fn required_is_only_somebody_else_s_turn_when_somebody_was_asked() {
    let (_, nobody) = one(pr_json("OPEN", false, Some("REVIEW_REQUIRED"), 0, "null"));
    assert_eq!(nobody.review, "none");
    let (_, asked) = one(pr_json("OPEN", false, Some("REVIEW_REQUIRED"), 1, "null"));
    assert_eq!(asked.review, "required");
}

#[test]
fn without_a_decision_the_latest_reviews_stand_in_for_one() {
    let review =
        |decision, requests, states: &[&str]| one(reviewed(decision, requests, states)).1.review;
    assert_eq!(
        review(None, 0, &["APPROVED", "CHANGES_REQUESTED"]),
        "changes"
    );
    assert_eq!(review(None, 1, &["APPROVED"]), "approved");
    assert_eq!(review(None, 0, &["COMMENTED"]), "none");
    assert_eq!(review(None, 1, &[]), "required");
    assert_eq!(review(None, 0, &[]), "none");
    // A decision GitHub names is not second-guessed by the reviews.
    assert_eq!(
        review(Some("APPROVED"), 0, &["CHANGES_REQUESTED"]),
        "approved"
    );
}

#[test]
fn a_stale_run_is_pending_not_failed() {
    let checks = rollup("PENDING", &[count("STALE", 2), count("FAILURE", 1)], &[]);
    let (_, got) = one(pr_json("OPEN", false, None, 0, &checks));
    assert_eq!((got.ci.fail, got.ci.pending), (1, 2));
}

#[test]
fn an_error_that_is_not_not_found_is_a_failed_read() {
    let answer = |kind: Option<&str>| {
        let kind = kind.map_or(String::new(), |k| format!(r#""type":"{k}","#));
        serde_json::from_str::<Value>(&format!(
            r#"{{"data":{{"p0":null}},"errors":[{{{kind}"path":["p0"],"message":"nope"}}]}}"#
        ))
        .unwrap()
    };
    for kind in ["NOT_FOUND", "FORBIDDEN", "INSUFFICIENT_SCOPES"] {
        assert_eq!(unread_error(&answer(Some(kind))), None, "{kind}");
    }
    // No alias to pin it on: the read failed, whatever the type says.
    let kind = "FORBIDDEN";
    let unpinned = serde_json::json!({"data": {}, "errors": [{"type": kind, "message": "no"}]});
    assert_eq!(unread_error(&unpinned).as_deref(), Some("no"));
    for kind in [
        Some("SERVICE_UNAVAILABLE"),
        Some("RATE_LIMITED"),
        Some("TIMEOUT"),
        None,
    ] {
        assert_eq!(
            unread_error(&answer(kind)).as_deref(),
            Some("nope"),
            "{kind:?}"
        );
    }
    assert_eq!(unread_error(&serde_json::json!({"data":{}})), None);
}

#[test]
fn an_http_date_becomes_the_time_since_takes() {
    assert_eq!(
        iso_from_http_date("Thu, 01 Jan 2026 00:00:05 GMT").as_deref(),
        Some("2026-01-01T00:00:05Z")
    );
    assert_eq!(
        iso_from_http_date("Mon, 9 Dec 2024 23:59:59 GMT").as_deref(),
        Some("2024-12-09T23:59:59Z")
    );
    for odd in [
        "",
        "X",
        "Thu, 01 Foo 2026 00:00:05 GMT",
        "Thu, 01 Jan 2026 25:00:05 GMT",
        "Thu, 01 Jan 2026 00:00 GMT",
        "Thu, 01 Jan 2026 00:00:05 PST",
        "Thu, 01 Jan 2026 00:00:05 GMT extra",
        "01 Jan 2026 00:00:05 GMT",
    ] {
        assert_eq!(iso_from_http_date(odd), None, "{odd:?}");
    }
}

#[test]
fn the_interval_github_asks_for_is_honoured_up_to_an_hour() {
    let at = |value: &str| {
        let head = format!(
            "HTTP/2.0 200 OK
X-Poll-Interval: {value}

"
        );
        poll_interval(&parse_included(&head).unwrap())
    };
    assert_eq!(at("60"), Some(60));
    assert_eq!(at("1"), Some(1));
    assert_eq!(at("86400"), Some(MAX_INTERVAL));
    assert_eq!(at("soon"), None);
    let none = parse_included(
        "HTTP/2.0 200 OK

",
    )
    .unwrap();
    assert_eq!(poll_interval(&none), None);
}

#[test]
fn no_checks_are_counted_as_none() {
    for checks in ["null".to_string(), rollup("SUCCESS", &[], &[])] {
        let (_, got) = one(pr_json("OPEN", false, None, 0, &checks));
        assert_eq!(got.ci, CheckCounts::default());
    }
    // A commit with no rollup at all.
    let bare = pr_json("OPEN", false, None, 0, "null").replace(
        r#""commits":{"nodes":[{"commit":{"statusCheckRollup":null}}]}"#,
        r#""commits":{"nodes":[]}"#,
    );
    let (_, got) = one(bare);
    assert_eq!(got.ci, CheckCounts::default());
}

#[test]
fn missing_counts_fall_back_to_the_rollups_own_verdict() {
    let (_, failed) = one(pr_json(
        "OPEN",
        false,
        None,
        0,
        &rollup("FAILURE", &[], &[]),
    ));
    assert_eq!((failed.ci.fail, failed.ci.pending), (1, 0));
    let (_, error) = one(pr_json("OPEN", false, None, 0, &rollup("ERROR", &[], &[])));
    assert_eq!(error.ci.fail, 1);
    let (_, waiting) = one(pr_json(
        "OPEN",
        false,
        None,
        0,
        &rollup("PENDING", &[], &[]),
    ));
    assert_eq!((waiting.ci.fail, waiting.ci.pending), (0, 1));
    // The counts, when they came, are what is believed.
    let counted = rollup("FAILURE", &[count("SUCCESS", 3)], &[]);
    let (_, got) = one(pr_json("OPEN", false, None, 0, &counted));
    assert_eq!((got.ci.pass, got.ci.fail), (3, 0));
}

#[test]
fn a_pr_gh_could_not_find_says_why() {
    let good = pr_json("OPEN", false, None, 0, "null");
    let stdout = format!(
        r#"{{"data":{{"p0":{{"pullRequest":{good}}},"p1":{{"pullRequest":null}},"p2":null}},"errors":[{{"path":["p1","pullRequest"],"message":"Could not resolve to a PullRequest with the number of 99."}},{{"path":["p2"],"message":"Could not resolve to a Repository with the name 'acme/nope'."}}]}}"#
    );
    let got = parse_graphql(&stdout, 3);
    assert_eq!(got[0].0, PrState::Open);
    assert!(matches!(&got[1].0, PrState::Unreadable(why) if why.contains("number of 99")));
    assert!(matches!(&got[2].0, PrState::Unreadable(why) if why.contains("acme/nope")));
    assert!(got[1].1.is_none() && got[2].1.is_none());
    // Nothing said about it: still not a state.
    let silent = parse_graphql(r#"{"data":{"p0":null}}"#, 1);
    assert!(matches!(&silent[0].0, PrState::Unreadable(_)));
}

#[test]
fn a_full_page_may_have_left_news_behind() {
    assert!(!page_is_full(0));
    assert!(!page_is_full(PAGE - 1));
    assert!(page_is_full(PAGE));
    assert!(page_is_full(PAGE + 1));
    let body = format!("[{}]", vec!["{}"; PAGE].join(","));
    assert!(page_is_full(prs_in_notifications(&body).unwrap().threads));
}

#[test]
fn an_error_that_names_no_alias_is_what_an_unfound_pr_says() {
    // A rate limit, or no data at all: not tied to any pull request.
    let limited =
        r#"{"data":null,"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded"}]}"#;
    let got = parse_graphql(limited, 2);
    for (state, status) in &got {
        assert!(
            matches!(state, PrState::Unreadable(why) if why.contains("rate limit")),
            "{state:?}"
        );
        assert!(status.is_none());
    }
    assert_eq!(
        first_error(limited).as_deref(),
        Some("API rate limit exceeded")
    );
    assert_eq!(first_error(r#"{"data":{}}"#), None);
}

#[test]
fn an_answer_that_is_not_the_query_is_not_guessed_at() {
    for odd in ["", "not json", "[]"] {
        let got = parse_graphql(odd, 2);
        assert_eq!(got.len(), 2);
        assert!(
            got.iter()
                .all(|(state, status)| matches!(state, PrState::Unreadable(_)) && status.is_none())
        );
    }
    let unknown = pr_json("DRAFT", false, None, 0, "null");
    let got = parse_graphql(&data(&[unknown]), 1);
    assert!(matches!(&got[0].0, PrState::Unreadable(why) if why.contains("DRAFT")));
}
