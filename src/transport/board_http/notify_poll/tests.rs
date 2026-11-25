use super::*;
use crate::registry::enqueue_notice;

fn request(headers: &[(&str, &str)]) -> Request {
    Request {
        method: String::from("GET"),
        path: "/api/boards".to_string(),
        query: Vec::new(),
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: Vec::new(),
    }
}

const APP: &[(&str, &str)] = &[("X-Adjutant-Notify", "1"), ("X-Adjutant-Shell", "app")];
const PAGE: &[(&str, &str)] = &[("X-Adjutant-Notify", "1")];

fn header<'a>(headers: &'a [(&'static str, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.as_str())
}

#[test]
fn a_client_counts_when_it_can_show_notifications_and_rings_waits_only_with_the_preference() {
    let client = |headers: &[(&str, &str)]| notify_client(&request(headers));
    assert_eq!(client(APP), Some((Client::App, true)));
    assert_eq!(client(PAGE), Some((Client::Page, true)));
    assert_eq!(
        client(&[("x-adjutant-notify", "1"), ("x-adjutant-shell", "app")]),
        Some((Client::App, true))
    );
    // Permission granted with 確認待ち off: open, and not for waits.
    assert_eq!(
        client(&[
            ("X-Adjutant-Notify", "0"),
            ("X-Adjutant-Can-Notify", "1"),
            ("X-Adjutant-Shell", "app")
        ]),
        Some((Client::App, false))
    );
    assert_eq!(
        client(&[("X-Adjutant-Notify", "0"), ("X-Adjutant-Can-Notify", "1")]),
        Some((Client::Page, false))
    );
    // An app that cannot show notifications is neither: it must not silence a browser.
    assert_eq!(
        client(&[
            ("X-Adjutant-Notify", "0"),
            ("X-Adjutant-Can-Notify", "0"),
            ("X-Adjutant-Shell", "app")
        ]),
        None
    );
    assert_eq!(client(&[("X-Adjutant-Shell", "app")]), None);
    assert_eq!(client(&[]), None);
    // Another shell is a page.
    assert_eq!(
        client(&[("X-Adjutant-Notify", "1"), ("X-Adjutant-Shell", "other")]),
        Some((Client::Page, true))
    );
}

#[test]
fn an_app_that_cannot_notify_does_not_open_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let headers = poll_headers(
        dir.path(),
        &request(&[
            ("X-Adjutant-Notify", "0"),
            ("X-Adjutant-Can-Notify", "0"),
            ("X-Adjutant-Shell", "app"),
        ]),
        1000,
        &[],
    );
    assert_eq!(header(&headers, APP_OPEN_HEADER), Some("0"));
    let headers = poll_headers(dir.path(), &request(PAGE), 1001, &[]);
    assert_eq!(header(&headers, APP_OPEN_HEADER), Some("0"));
}

#[test]
fn an_app_with_the_waits_preference_off_is_still_open_and_takes_the_notices() {
    let dir = tempfile::tempdir().unwrap();
    let app = [
        ("X-Adjutant-Notify", "0"),
        ("X-Adjutant-Can-Notify", "1"),
        ("X-Adjutant-Shell", "app"),
    ];
    let headers = poll_headers(dir.path(), &request(&app), 1000, &[]);
    assert_eq!(header(&headers, APP_OPEN_HEADER), Some("1"));
    assert_eq!(notify_clients(dir.path()).app_waits_at, 0);
    enqueue_notice(dir.path(), "o/r", "t", "m", 1000).unwrap();
    let page = poll_headers(dir.path(), &request(PAGE), 1001, &[]);
    assert!(header(&page, NOTICES_HEADER).is_none());
    let got = poll_headers(dir.path(), &request(&app), 1002, &[]);
    assert!(header(&got, NOTICES_HEADER).is_some());
}

#[test]
fn a_notice_goes_to_the_app_while_it_is_open_and_to_a_page_otherwise() {
    let dir = tempfile::tempdir().unwrap();
    // A page is open: it takes the notice.
    poll_headers(dir.path(), &request(PAGE), 1000, &[]);
    enqueue_notice(dir.path(), "o/r", "adjutant / r", "one", 1000).unwrap();
    let got = poll_headers(dir.path(), &request(PAGE), 1001, &[]);
    assert_eq!(header(&got, APP_OPEN_HEADER), Some("0"));
    let notices: serde_json::Value =
        serde_json::from_str(header(&got, NOTICES_HEADER).unwrap()).unwrap();
    assert_eq!(notices[0]["message"], "one");
    assert!(notices[0]["id"].as_str().is_some());
    // Handed over once.
    let again = poll_headers(dir.path(), &request(PAGE), 1002, &[]);
    assert!(header(&again, NOTICES_HEADER).is_none());

    // The app opens: it takes them, and a page is told the app is open and takes nothing.
    poll_headers(dir.path(), &request(APP), 1003, &[]);
    enqueue_notice(dir.path(), "o/r", "adjutant / r", "two", 1003).unwrap();
    let page = poll_headers(dir.path(), &request(PAGE), 1004, &[]);
    assert_eq!(header(&page, APP_OPEN_HEADER), Some("1"));
    assert!(header(&page, NOTICES_HEADER).is_none());
    let silent = poll_headers(dir.path(), &request(&[]), 1004, &[]);
    assert!(header(&silent, NOTICES_HEADER).is_none());
    let app = poll_headers(dir.path(), &request(APP), 1005, &[]);
    assert!(header(&app, NOTICES_HEADER).unwrap().contains("two"));
}

#[test]
fn the_notices_travel_as_ascii_that_reads_back_as_the_same_text() {
    let notice = Notice {
        id: "i".to_string(),
        title: "adjutant / widget".to_string(),
        message: "確認待ち \"quoted\" \u{1F600}\ndel\u{7f}".to_string(),
        nwo: "o/r".to_string(),
        created_at: 1,
        tag: Some("gate-o-r/g1".to_string()),
    };
    let text = notices_header(std::slice::from_ref(&notice));
    assert!(
        text.is_ascii() && !text.contains('\n') && !text.contains('\u{7f}'),
        "{text}"
    );
    let back: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(back[0]["message"], notice.message.as_str());
    assert_eq!(back[0]["tag"], "gate-o-r/g1");
}

#[test]
fn the_boards_with_a_notifier_of_their_own_are_named_and_do_not_narrow_the_app_or_the_notices() {
    let dir = tempfile::tempdir().unwrap();
    let app = [("X-Adjutant-Notify", "1"), ("X-Adjutant-Shell", "app")];
    let own = vec!["acme-cmd".to_string(), "caf\u{e9}".to_string()];
    // One repository's own notifier does not turn the app, or the notices of the others, off.
    let got = poll_headers(dir.path(), &request(&app), 1000, &own);
    assert_eq!(header(&got, APP_OPEN_HEADER), Some("1"));
    let text = header(&got, OWN_NOTIFIER_HEADER).unwrap();
    assert!(text.is_ascii(), "{text}");
    let back: Vec<String> = serde_json::from_str(text).unwrap();
    assert_eq!(back, own);
    enqueue_notice(dir.path(), "o/r", "t", "m", 1000).unwrap();
    let got = poll_headers(dir.path(), &request(&app), 1001, &own);
    assert!(header(&got, NOTICES_HEADER).is_some());
    // None to name: no header.
    let got = poll_headers(dir.path(), &request(&app), 1002, &[]);
    assert!(header(&got, OWN_NOTIFIER_HEADER).is_none());
}
