//! `GET /api/others` and `POST /api/others/sync` on the resident server, with `gh` stubbed so
//! nothing here reaches GitHub.

mod common;

use common::*;
use serde_json::{Value, json};

// GitHub's upper-case enum values are made from lower-case words rather than written into the
// JSON: a bare upper-case string after a colon reads as a tracker key to the guard over
// everything that ships.
fn up(word: &str) -> String {
    word.to_ascii_uppercase()
}

/// One PR as the details query answers it: asked of `me`, with two files.
fn pr_node(number: u64, repo: &str) -> Value {
    json!({
        "number": number,
        "url": format!("https://github.com/acme/{repo}/pull/{number}"),
        "title": format!("PR {number}"),
        "state": up("open"),
        "isDraft": false,
        "author": {"login": "alice"},
        "baseRefName": "main",
        "headRefName": "feature",
        "headRefOid": "h1",
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
        "reviewRequests": {"nodes": [{"requestedReviewer": {"__typename": "User", "login": "me"}}]},
        "latestReviews": {"nodes": []},
        "myReviews": {"nodes": []},
        "requests": {"nodes": [{"createdAt": "2026-10-01T00:00:00Z", "requestedReviewer": {"login": "me"}}]},
        "commits": {"nodes": [{"commit": {"oid": "h1", "committedDate": "2026-09-30T00:00:00Z"}}]},
        "lastCommit": {"nodes": [{"commit": {"statusCheckRollup": null}}]}
    })
}

/// A `gh` that is logged in as `me`, finds two PRs (one in the repository of the fixture, one
/// in another repository of the same owner), and writes down every call. It fails to say who
/// you are while `fail` exists.
fn stub_gh(fixture: &Fixture) -> (String, std::path::PathBuf, std::path::PathBuf) {
    let stubs = fixture.repo.join("stub-bin");
    let log = fixture.repo.join("gh-log");
    let fail = fixture.repo.join("gh-fail");
    let search = json!([
        {"number": 7, "url": "https://github.com/acme/widget/pull/7",
         "repository": {"nameWithOwner": "acme/widget"}},
        {"number": 8, "url": "https://github.com/acme/other/pull/8",
         "repository": {"nameWithOwner": "acme/other"}}
    ]);
    let script = r#"#!/bin/sh
echo "$@" >> @LOG@
case "$1 $2" in
"api user")
  if [ -f @FAIL@ ]; then echo "You are not logged in" >&2; exit 1; fi
  echo me ;;
"search prs") echo '@SEARCH@' ;;
"api graphql")
  data=''
  for a in "$@"; do
    case "$a" in
      p[0-9]*=*)
        alias=${a%%=*}
        case "${a#*=}" in
          7) pr='@PR7@' ;;
          8) pr='@PR8@' ;;
          *) pr=null ;;
        esac
        data="$data\"$alias\":{\"pullRequest\":$pr},"
        ;;
    esac
  done
  echo "{\"data\":{${data%,}}}" ;;
esac
"#
    .replace("@LOG@", &shell_quoted(log.to_str().unwrap()))
    .replace("@FAIL@", &shell_quoted(fail.to_str().unwrap()))
    .replace("@SEARCH@", &search.to_string())
    .replace("@PR7@", &pr_node(7, "widget").to_string())
    .replace("@PR8@", &pr_node(8, "other").to_string());
    stub_bin(&stubs, "gh", &script);
    (path_with(&stubs), log, fail)
}

/// The fixture's repository in the address book, as a hub would have put it there.
fn register(fixture: &Fixture) {
    let boards = fixture.state.join("boards");
    std::fs::create_dir_all(&boards).unwrap();
    std::fs::write(
        boards.join(format!("{SLUG}.json")),
        json!({"main": fixture.repo.to_str().unwrap(), "nwo": "acme/widget", "hub": null})
            .to_string(),
    )
    .unwrap();
}

fn body(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

#[test]
fn a_sync_records_the_prs_that_asked_for_your_review() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, log, _) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);

    // Before any sync there is nothing, and the page can tell.
    let (status, text) = resident.get("/api/others");
    assert_eq!(status, 200, "{text}");
    let before = body(&text);
    assert_eq!(before["lastSync"], Value::Null);
    assert_eq!(before["records"], json!([]));
    assert!(before["now"].as_i64().unwrap() > 0);
    assert!(!log.exists(), "reading asked GitHub");

    let (status, text) = resident.post("/api/others/sync", "");
    assert_eq!(status, 200, "{text}");
    let synced = body(&text);
    let records = synced["records"].as_array().unwrap();
    assert_eq!(records.len(), 2);
    let (other, widget) = (&records[0], &records[1]);
    assert_eq!(other["id"], "acme/other#8");
    assert_eq!(other["registered"], false);
    assert_eq!(widget["id"], "acme/widget#7");
    assert_eq!(widget["registered"], true);
    assert_eq!(widget["state"], "requested");
    assert_eq!(widget["author"], "alice");
    assert_eq!(widget["headSha"], "h1");
    assert_eq!(widget["files"].as_array().unwrap().len(), 2);
    assert_eq!(widget["files"][1]["change"], "added");
    assert_eq!(widget["events"]["requested"], "20261001T000000Z");
    assert_eq!(synced["arrived"], json!(["acme/other#8", "acme/widget#7"]));
    assert_eq!(synced["lastSync"]["owners"], json!(["acme"]));
    assert_eq!(synced["lastSync"]["complete"], true);

    let asked = std::fs::read_to_string(&log).unwrap();
    let search = asked.lines().find(|l| l.starts_with("search prs")).unwrap();
    assert!(search.contains("--owner=acme"), "{search}");
    assert!(search.contains("--review-requested=@me"), "{search}");
    assert!(search.contains("--limit 100"), "{search}");

    // What a later read says is what the sync made.
    let (status, text) = resident.get("/api/others");
    assert_eq!(status, 200, "{text}");
    let after = body(&text);
    assert_eq!(after["records"], synced["records"]);
    assert_eq!(after["lastSync"]["at"], synced["lastSync"]["at"]);
    assert!(after["lastSync"]["at"].as_str().unwrap().ends_with('Z'));
}

#[test]
fn a_gh_that_is_not_logged_in_fails_the_sync_with_what_it_said() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, _, fail) = stub_gh(&fixture);
    std::fs::write(&fail, "").unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);

    let (status, text) = resident.post("/api/others/sync", "");
    assert_eq!(status, 400, "{text}");
    assert!(
        body(&text)["error"]
            .as_str()
            .unwrap()
            .contains("You are not logged in"),
        "{text}"
    );
    // The failure is written down, with no sync to its name.
    let state = body(&resident.get("/api/others").1);
    assert_eq!(state["lastSync"]["at"], Value::Null);
    assert!(
        state["lastSync"]["error"]
            .as_str()
            .unwrap()
            .contains("not logged in")
    );
    assert_eq!(state["records"], json!([]));
}

#[test]
fn a_board_served_alone_has_no_such_route() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, log, _) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    let (status, _) = resident.post(&format!("/b/{SLUG}/api/others/sync"), "");
    assert_eq!(status, 404);
    assert!(!log.exists(), "a route that does not exist asked GitHub");
}
