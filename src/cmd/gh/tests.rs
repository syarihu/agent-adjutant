use super::*;

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
fn a_full_page_may_have_left_news_behind() {
    assert!(!page_is_full(0));
    assert!(!page_is_full(PAGE - 1));
    assert!(page_is_full(PAGE));
    assert!(page_is_full(PAGE + 1));
    let body = format!("[{}]", vec!["{}"; PAGE].join(","));
    assert!(page_is_full(prs_in_notifications(&body).unwrap().threads));
}
