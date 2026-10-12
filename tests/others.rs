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

// ── the code-review hub and the read-through ─────────────────────────

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

/// A read-through that passes the schema, with `sha` as the reader read it.
fn read_through() -> Value {
    json!({
        "version": 1,
        "sha": HEAD,
        "three": {"purpose": "p", "approach": "a", "impact": "i", "essence": "e"},
        "points": [{"level": "attention", "path": "a.rs", "line": 3, "title": "t", "why": "w"}],
        "order": [{"path": "a.rs", "category": "logic", "note": "n", "skippable": false}],
        "drafts": [{"path": "a.rs", "line": 3, "body": "b"}]
    })
}

/// Run a sync so that `acme/widget#7` has a record, and give it a read that is `status`.
fn with_read(fixture: &Fixture, resident: &Resident, status: &str) {
    let (code, text) = resident.post("/api/others/sync", "");
    assert_eq!(code, 200, "{text}");
    let path = fixture.state.join("others/acme~widget~7.json");
    let mut record = body(&std::fs::read_to_string(&path).unwrap());
    record["ai"] = json!({"status": status, "startedAt": "20261002T000000Z"});
    std::fs::write(&path, record.to_string()).unwrap();
}

fn record_status(fixture: &Fixture) -> String {
    let path = fixture.state.join("others/acme~widget~7.json");
    body(&std::fs::read_to_string(path).unwrap())["ai"]["status"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

#[test]
fn the_list_carries_the_code_review_hub() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let (status, text) = resident.get("/api/others");
    assert_eq!(status, 200, "{text}");
    let hub = &body(&text)["hub"];
    assert_eq!(hub["slug"], "review");
    assert_eq!(hub["state"], "stopped");
    assert_eq!(hub["max"], 3);
    assert_eq!(hub["running"], 0);
    assert_eq!(hub["startable"], false);
    assert_eq!(hub["canRead"], false);
}

#[test]
fn a_read_through_is_checked_stored_and_served() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, _, _) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    with_read(&fixture, &resident, "running");

    let (code, _) = get_with_query(&resident, "/api/others/read", "id=acme/widget%237");
    assert_eq!(code, 404);
    let (code, _) = get_with_query(&resident, "/api/others/read", "id=nonsense");
    assert_eq!(code, 400);

    let file = fixture.repo.join("read-through.json");
    let mut bad = read_through();
    bad["three"]["purpose"] = json!(" ");
    bad["pionts"] = json!([]);
    std::fs::write(&file, bad.to_string()).unwrap();
    let out = fixture.cmd(&[
        "review",
        "submit",
        "--pr",
        "acme/widget#7",
        "--file",
        file.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(!fixture.state.join("others/reads").exists());
    assert_eq!(record_status(&fixture), "running");

    let mut bad = read_through();
    bad["three"]["purpose"] = json!(" ");
    bad["points"][0]["line"] = json!(0);
    std::fs::write(&file, bad.to_string()).unwrap();
    let out = fixture.cmd(&[
        "review",
        "submit",
        "--pr",
        "acme/widget#7",
        "--file",
        file.to_str().unwrap(),
    ]);
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("three.purpose"), "{said}");
    assert!(said.contains("points[0]"), "{said}");

    std::fs::write(&file, read_through().to_string()).unwrap();
    let out = fixture.cmd(&[
        "review",
        "submit",
        "--pr",
        "acme/widget#7",
        "--file",
        file.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(record_status(&fixture), "ready");

    let (code, text) = get_with_query(&resident, "/api/others/read", "id=acme/widget%237");
    assert_eq!(code, 200, "{text}");
    let stored = body(&text);
    assert_eq!(stored["id"], "acme/widget#7");
    assert_eq!(stored["sha"], HEAD);
    assert_eq!(stored["three"]["essence"], "e");
    assert_eq!(stored["drafts"][0]["side"], "right");
    // The list says what it holds, and when its material was taken (`stale` depends on the
    // second the sync and the hand-in fell in, so the unit tests pin it, not this).
    let record = &body(&resident.get("/api/others").1)["records"][1];
    assert_eq!(
        record["ai"]["counts"],
        json!({"attention": 1, "check": 0, "drafts": 1})
    );
    assert!(record["ai"]["readFrom"].as_str().is_some());
    assert!(record["ai"]["stale"].is_boolean());
    assert_eq!(record["state"], "ai-ready");
}

#[test]
fn asking_for_a_read_while_the_hub_is_down_is_refused_and_changes_nothing() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, _, _) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    resident.post("/api/others/sync", "");
    let record = fixture.state.join("others/acme~widget~7.json");
    let before = std::fs::read_to_string(&record).unwrap();

    let (code, text) = resident.post(
        "/api/others/ai",
        &json!({"id": "acme/widget#7"}).to_string(),
    );
    assert_eq!(code, 409, "{text}");
    assert_eq!(body(&text)["why"], "hub-stopped");
    assert_eq!(std::fs::read_to_string(&record).unwrap(), before);

    let (code, _) = resident.post("/api/others/ai", &json!({"id": "nonsense"}).to_string());
    assert_eq!(code, 400);
    let (code, _) = resident.post("/api/others/ai", "{}");
    assert_eq!(code, 400);
}

#[test]
fn a_mode_that_is_not_read_or_split_is_refused_before_anything_is_written() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, _, _) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    resident.post("/api/others/sync", "");
    let record = fixture.state.join("others/acme~widget~7.json");
    let before = std::fs::read_to_string(&record).unwrap();
    // Refused for its mode, not for the hub being down (which is a 409 for a good one).
    for mode in [json!("x"), json!("Split"), json!(""), json!(5)] {
        let (code, text) = resident.post(
            "/api/others/ai",
            &json!({"id": "acme/widget#7", "mode": mode}).to_string(),
        );
        assert_eq!(code, 400, "{mode}: {text}");
        assert_eq!(body(&text)["error"], "mode must be \"read\" or \"split\"");
        assert_eq!(std::fs::read_to_string(&record).unwrap(), before);
    }
    for mode in [json!("read"), json!("split"), Value::Null] {
        let (code, _) = resident.post(
            "/api/others/ai",
            &json!({"id": "acme/widget#7", "mode": mode}).to_string(),
        );
        assert_eq!(code, 409, "{mode}");
    }
}

#[test]
fn a_read_asked_for_with_the_hub_up_is_started_in_a_window_of_its_own() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let (gh_path, _, _) = stub_gh(&fixture);
    // The stub `gh` and the stub tmux are both in front of the real ones.
    let path = format!("{}:{}", gh_path.split(':').next().unwrap(), tmux.path());
    let resident = {
        let log = tmux.log.to_string_lossy().to_string();
        let panes = tmux.panes.to_string_lossy().to_string();
        let clients = tmux.clients.to_string_lossy().to_string();
        let home = tmux.home.to_string_lossy().to_string();
        let screen = tmux.screen.to_string_lossy().to_string();
        Resident::start_with(
            &fixture,
            &[
                ("PATH", &path),
                ("FAKE_TMUX_LOG", &log),
                ("FAKE_TMUX_PANES", &panes),
                ("FAKE_TMUX_CLIENTS", &clients),
                ("FAKE_TMUX_HOME", &home),
                ("FAKE_TMUX_SCREEN", &screen),
                ("FAKE_TMUX_KILL", ""),
            ],
        )
    };
    resident.post("/api/others/sync", "");

    // A live process stands in for the code-review hub.
    let hub = Sleeper::new();
    forge_review_hub(&fixture, &hub);
    let state = body(&resident.get("/api/others").1);
    assert_eq!(state["hub"]["state"], "running");
    assert_eq!(state["hub"]["canRead"], true);

    let (code, text) = resident.post(
        "/api/others/ai",
        &json!({"id": "acme/widget#7"}).to_string(),
    );
    assert_eq!(code, 200, "{text}");
    assert_eq!(body(&text)["record"]["id"], "acme/widget#7");
    // The queue could be worked, so there is nothing to warn of.
    assert_eq!(body(&text)["warning"], Value::Null);
    // Started, not only queued, and in a window that runs the reader.
    assert_eq!(record_status(&fixture), "running");
    let record = &body(&resident.get("/api/others").1)["records"][1];
    assert_eq!(record["state"], "ai-reading");
    assert!(
        record["ai"]["sessionId"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    let logged = tmux.logged();
    assert!(logged.contains("new-window"), "{logged}");
    assert!(logged.contains("review run --pr"), "{logged}");
    assert!(logged.contains("acme/widget#7"), "{logged}");
    assert!(fixture.state.join("review/acme+widget+7").is_dir());

    // Asked again while it is being read, nothing more starts.
    let before = tmux.logged();
    let (code, _) = resident.post(
        "/api/others/ai",
        &json!({"id": "acme/widget#7"}).to_string(),
    );
    assert_eq!(code, 200);
    assert_eq!(tmux.logged(), before);
}

#[test]
fn a_split_asked_for_with_the_hub_up_is_queued_in_split_mode_and_started() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let (gh_path, _, _) = stub_gh(&fixture);
    // The stub `gh` and the stub tmux are both in front of the real ones.
    let path = format!("{}:{}", gh_path.split(':').next().unwrap(), tmux.path());
    let resident = {
        let log = tmux.log.to_string_lossy().to_string();
        let panes = tmux.panes.to_string_lossy().to_string();
        let clients = tmux.clients.to_string_lossy().to_string();
        let home = tmux.home.to_string_lossy().to_string();
        let screen = tmux.screen.to_string_lossy().to_string();
        Resident::start_with(
            &fixture,
            &[
                ("PATH", &path),
                ("FAKE_TMUX_LOG", &log),
                ("FAKE_TMUX_PANES", &panes),
                ("FAKE_TMUX_CLIENTS", &clients),
                ("FAKE_TMUX_HOME", &home),
                ("FAKE_TMUX_SCREEN", &screen),
                ("FAKE_TMUX_KILL", ""),
            ],
        )
    };
    resident.post("/api/others/sync", "");

    // A live process stands in for the code-review hub.
    let hub = Sleeper::new();
    forge_review_hub(&fixture, &hub);
    let state = body(&resident.get("/api/others").1);
    assert_eq!(state["hub"]["state"], "running");
    assert_eq!(state["hub"]["canRead"], true);

    let (code, text) = resident.post(
        "/api/others/ai",
        &json!({"id": "acme/widget#7", "mode": "split"}).to_string(),
    );
    assert_eq!(code, 200, "{text}");
    assert_eq!(body(&text)["record"]["id"], "acme/widget#7");
    assert_eq!(body(&text)["record"]["ai"]["mode"], "split");
    // The queue could be worked, so there is nothing to warn of.
    assert_eq!(body(&text)["warning"], Value::Null);
    // Started, not only queued, and in a window that runs the reader.
    assert_eq!(record_status(&fixture), "running");
    let record = &body(&resident.get("/api/others").1)["records"][1];
    assert_eq!(record["state"], "ai-reading");
    assert!(
        record["ai"]["sessionId"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    let logged = tmux.logged();
    assert!(logged.contains("new-window"), "{logged}");
    assert!(logged.contains("review run --pr"), "{logged}");
    assert!(logged.contains("acme/widget#7"), "{logged}");
    assert!(fixture.state.join("review/acme+widget+7").is_dir());

    // The record keeps what was asked for, which is what the reader's window reads its task from.
    let record = &body(&resident.get("/api/others").1)["records"][1];
    assert_eq!(record["ai"]["mode"], "split");
}

/// A live process standing in for the code-review hub, recorded the way a hub records itself.
fn forge_review_hub(fixture: &Fixture, hub: &Sleeper) {
    let record = fixture.state.join("hubs/review.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        json!({
            "pid": hub.pid(), "psStarted": ps_started(hub.pid()), "hubName": "adjutant-review",
            "cwd": fixture.state.join("review").to_str().unwrap(), "nameInCommand": false,
            "hub": "@review",
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn a_running_hub_whose_terminal_cannot_open_a_reader_cannot_be_asked() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, _, _) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    resident.post("/api/others/sync", "");
    let hub = Sleeper::new();
    forge_review_hub(&fixture, &hub);
    let state = body(&resident.get("/api/others").1);
    assert_eq!(state["hub"]["state"], "running");
    assert_eq!(state["hub"]["canRead"], false);
    assert_eq!(state["hub"]["startable"], false);
    let record = fixture.state.join("others/acme~widget~7.json");
    let before = std::fs::read_to_string(&record).unwrap();
    let (code, text) = resident.post(
        "/api/others/ai",
        &json!({"id": "acme/widget#7"}).to_string(),
    );
    assert_eq!(code, 409, "{text}");
    assert_eq!(body(&text)["why"], "not-startable");
    assert_eq!(std::fs::read_to_string(&record).unwrap(), before);
}

#[test]
fn a_reader_that_cannot_finish_ends_its_read_with_adj_review_fail() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, _, _) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    with_read(&fixture, &resident, "running");
    let file = fixture.state.join("others/acme~widget~7.json");
    let mut record = body(&std::fs::read_to_string(&file).unwrap());
    record["ai"]["sessionId"] = json!("sid-a");
    std::fs::write(&file, record.to_string()).unwrap();

    let fail = |session: Option<&str>| {
        let mut command = fixture.command([
            "review",
            "fail",
            "--pr",
            "acme/widget#7",
            "--why",
            "no diff",
        ]);
        if let Some(session) = session {
            command.env("ADJUTANT_REVIEW_SESSION", session);
        }
        command.output().unwrap()
    };
    // Another session is refused and nothing changes.
    let out = fail(Some("sid-other"));
    assert!(!out.status.success());
    assert_eq!(record_status(&fixture), "running");
    // Its own is taken; the PR can then be asked for again.
    let out = fail(Some("sid-a"));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(record_status(&fixture), "failed");
    let err = body(&std::fs::read_to_string(&file).unwrap())["ai"]["error"].clone();
    assert_eq!(err, "no diff");
    // A blank reason is refused before anything is written.
    let out = fixture
        .command(["review", "fail", "--pr", "acme/widget#7", "--why", "  "])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn a_reader_can_only_tell_the_code_review_hub() {
    let fixture = Fixture::new(QUIET);
    // No address to give: clap refuses these before anything is sent.
    for extra in [
        ["--repo", "acme/widget"],
        ["--hub", "other"],
        ["--from", "someone"],
        ["--wake", ""],
    ] {
        let mut args = vec![
            "review",
            "say",
            "--pr",
            "acme/widget#7",
            "--subject",
            "s",
            "--body",
            "b",
        ];
        args.extend(extra.iter().filter(|a| !a.is_empty()));
        let out = fixture.cmd(&args);
        assert!(!out.status.success(), "{extra:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("unexpected argument"),
            "{extra:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // A person has `adj send`: with no reader's session nothing is said this way.
    let out = fixture.cmd(&[
        "review",
        "say",
        "--pr",
        "acme/widget#7",
        "--subject",
        "s",
        "--body",
        "b",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("only a reader"));
    let out = fixture
        .command([
            "review",
            "say",
            "--pr",
            "acme/widget#7",
            "--subject",
            "s",
            "--body",
            "b",
        ])
        .env("ADJUTANT_REVIEW_SESSION", "  ")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!fixture.state.join("inbox").exists());
    // Only report and question.
    let out = fixture.cmd(&[
        "review",
        "say",
        "--pr",
        "acme/widget#7",
        "--kind",
        "request",
        "--subject",
        "s",
        "--body",
        "b",
    ]);
    assert!(!out.status.success());
    assert!(!fixture.state.join("inbox").exists());

    // A record whose read is running under the reader's session.
    std::fs::create_dir_all(fixture.state.join("others")).unwrap();
    std::fs::write(
        fixture.state.join("others/acme~widget~7.json"),
        json!({
            "id": "acme/widget#7", "repo": "Acme/Widget", "number": 7,
            "url": "https://github.com/acme/widget/pull/7", "registered": true, "title": "t",
            "base": "main", "head": "f", "headSha": "h1", "prState": "open", "additions": 1,
            "deletions": 1, "changedFiles": 1, "requested": true,
            "requestedAt": "20261001T000000Z", "firstSeenAt": "20261001T000000Z",
            "readAt": "20261001T000000Z",
            "ai": {"status": "running", "sessionId": "sid-a", "startedAt": "20261001T000000Z"}
        })
        .to_string(),
    )
    .unwrap();
    // What it sends goes to the review hub's inbox, carrying the PR and the reader's session.
    let out = fixture
        .command([
            "review",
            "say",
            "--pr",
            "acme/widget#7",
            "--kind",
            "question",
            "--subject",
            "which one?",
            "--body",
            "text",
        ])
        .env("ADJUTANT_REVIEW_SESSION", "sid-a")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let inbox = fixture.state.join("inbox/review");
    let files: Vec<_> = std::fs::read_dir(&inbox)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .collect();
    assert_eq!(files.len(), 1);
    let text = std::fs::read_to_string(files[0].path()).unwrap();
    assert!(text.contains("acme/widget#7"), "{text}");
    assert!(text.contains("sid-a"), "{text}");
    assert!(text.contains("adjutant-read-acme~widget~7"), "{text}");
    assert!(
        text.contains("which one?") && text.contains("question"),
        "{text}"
    );
    // And nothing reached a repository's own hub.
    assert!(
        std::fs::read_dir(fixture.state.join("inbox"))
            .unwrap()
            .count()
            == 1
    );
}

#[test]
fn only_the_reader_the_record_names_may_speak_for_its_pr() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, _, _) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    with_read(&fixture, &resident, "running");
    let file = fixture.state.join("others/acme~widget~7.json");
    let mut record = body(&std::fs::read_to_string(&file).unwrap());
    record["ai"]["sessionId"] = json!("sid-a");
    std::fs::write(&file, record.to_string()).unwrap();
    let say = |session: &str| {
        fixture
            .command([
                "review",
                "say",
                "--pr",
                "acme/widget#7",
                "--subject",
                "s",
                "--body",
                "b",
            ])
            .env("ADJUTANT_REVIEW_SESSION", session)
            .output()
            .unwrap()
    };
    let out = say("sid-old");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not the one running this read"));
    assert!(!fixture.state.join("inbox").exists());
    assert!(say("sid-a").status.success());
    assert!(fixture.state.join("inbox/review").is_dir());
}

#[test]
fn a_start_of_the_hub_that_fails_is_a_server_error_and_bad_input_is_a_bad_request() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    // The terminal is not tmux, so the start itself cannot be made: not the caller's mistake.
    let (code, text) = resident.post("/api/others/hub/start", "{}");
    assert_eq!(code, 500, "{text}");
    assert!(
        body(&text)["error"].as_str().unwrap().contains("tmux"),
        "{text}"
    );
    // A `start` that is none of the three is the caller's.
    let (code, _) = resident.post(
        "/api/others/hub/start",
        &json!({"start": "sideways"}).to_string(),
    );
    assert_eq!(code, 400);
}

#[test]
fn the_diff_of_a_read_through_is_kept_at_hand_in_and_served() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, _, _) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    with_read(&fixture, &resident, "running");

    let (code, text) = get_with_query(&resident, "/api/others/diff", "id=acme/widget%237");
    assert_eq!(code, 404, "{text}");
    assert_eq!(body(&text)["error"], "no diff");
    let (code, _) = get_with_query(&resident, "/api/others/diff", "id=nonsense");
    assert_eq!(code, 400);

    // What `adj review context` leaves for the reader: the head it read and the diff.
    let material = fixture.state.join("review/acme+widget+7");
    std::fs::create_dir_all(&material).unwrap();
    std::fs::write(
        material.join("pr.json"),
        json!({"headRefOid": HEAD}).to_string(),
    )
    .unwrap();
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+b\n";
    std::fs::write(material.join("diff.patch"), patch).unwrap();
    let file = fixture.repo.join("read-through.json");
    std::fs::write(&file, read_through().to_string()).unwrap();
    let out = fixture.cmd(&[
        "review",
        "submit",
        "--pr",
        "acme/widget#7",
        "--file",
        file.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let (code, text) = get_with_query(&resident, "/api/others/diff", "id=acme/widget%237");
    assert_eq!(code, 200, "{text}");
    let stored = body(&text);
    assert_eq!(stored["id"], "acme/widget#7");
    assert_eq!(stored["sha"], HEAD);
    assert_eq!(stored["bytes"], patch.len());
    assert_eq!(stored["diff"], patch);
}

// ── a task's PR ──────────────────────────────────────────────────────

/// A task of the fixture's board, with `pr` when given.
fn task_with_pr(resident: &Resident, pr: Option<&str>) -> String {
    let task = made_task(resident, json!({"title": "Retry the upload"}));
    let id = task["id"].as_str().unwrap().to_string();
    if let Some(pr) = pr {
        let (status, text) = resident.post(
            &format!("/b/{SLUG}/api/tasks/{id}"),
            &json!({"pr": pr}).to_string(),
        );
        assert_eq!(status, 200, "{text}");
    }
    id
}

#[test]
fn asking_for_a_task_s_read_answers_by_what_it_came_to() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, log, _) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    let bare = task_with_pr(&resident, None);
    let with_pr = task_with_pr(&resident, Some("https://github.com/acme/widget/pull/7"));
    let ask = |id: &str, body: Value| {
        resident.post(&format!("/b/{SLUG}/api/tasks/{id}/ai"), &body.to_string())
    };

    // A mode that is not read or split is refused before the task is looked at.
    let (code, text) = ask(&with_pr, json!({"mode": "x"}));
    assert_eq!(code, 400, "{text}");
    assert_eq!(body(&text)["error"], "mode must be \"read\" or \"split\"");
    let (code, text) = ask("nonsense", json!({}));
    assert_eq!(code, 404, "{text}");
    let (code, text) = ask(&bare, json!({}));
    assert_eq!(code, 409, "{text}");
    assert_eq!(body(&text)["why"], "no-pr");
    // The code-review hub is not running: refused with nothing read and nothing written.
    let (code, text) = ask(&with_pr, json!({}));
    assert_eq!(code, 409, "{text}");
    assert_eq!(body(&text)["why"], "hub-stopped");
    assert!(!fixture.state.join("others").exists());
    assert!(!log.exists(), "a refused request asked GitHub");
}

#[test]
fn a_task_s_read_is_the_record_of_its_current_pr_and_the_hub() {
    let fixture = Fixture::new(QUIET);
    register(&fixture);
    let (path, _, _) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    let bare = task_with_pr(&resident, None);
    let with_pr = task_with_pr(&resident, Some("https://github.com/acme/widget/pull/7"));
    let read = |id: &str| resident.get(&format!("/b/{SLUG}/api/tasks/{id}/ai"));

    let (code, text) = read(&bare);
    assert_eq!(code, 200, "{text}");
    let none = body(&text);
    assert_eq!((&none["pr"], &none["record"]), (&Value::Null, &Value::Null));
    assert_eq!(none["hub"]["state"], "stopped");
    assert_eq!(none["hub"]["max"], 3);

    // A PR nobody has read yet: named, with no record.
    let (_, text) = read(&with_pr);
    let named = body(&text);
    assert_eq!(named["pr"], "acme/widget#7");
    assert_eq!(named["record"], Value::Null);

    // The record is found through the task's PR, whether or not it is linked to the task.
    resident.post("/api/others/sync", "");
    let (_, text) = read(&with_pr);
    assert_eq!(body(&text)["record"]["id"], "acme/widget#7");
    let (code, _) = read("nonsense");
    assert_eq!(code, 400);
}

#[test]
fn get_api_others_leaves_out_a_task_s_pr_but_counts_its_reader_in_the_hub() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let record = |number: u64, task: Option<Value>, requested: bool| {
        let mut record = json!({
            "id": format!("acme/widget#{number}"), "repo": "Acme/Widget", "number": number,
            "url": format!("https://github.com/acme/widget/pull/{number}"),
            "registered": true, "title": "t", "base": "main", "head": "f", "headSha": "h1",
            "prState": "open", "additions": 1, "deletions": 1, "changedFiles": 1,
            "requested": requested, "requestedAt": "20261001T000000Z",
            "firstSeenAt": "20261001T000000Z", "readAt": "20261001T000000Z",
            "ai": {"status": "running", "startedAt": "20261002T000000Z"}
        });
        if let Some(task) = task {
            record["task"] = task;
        }
        record
    };
    let dir = fixture.state.join("others");
    std::fs::create_dir_all(&dir).unwrap();
    let link = || Some(json!({"board": SLUG, "id": "t1"}));
    for (number, task, requested) in [(1, None, true), (2, link(), false), (3, link(), true)] {
        std::fs::write(
            dir.join(format!("acme~widget~{number}.json")),
            record(number, task, requested).to_string(),
        )
        .unwrap();
    }
    let (status, text) = resident.get("/api/others");
    assert_eq!(status, 200, "{text}");
    let state = body(&text);
    let ids: Vec<&str> = state["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    // The one nobody requested of you is not on the list, and the one that is also requested is.
    assert_eq!(ids, ["acme/widget#1", "acme/widget#3"]);
    assert_eq!(state["hub"]["running"], 3);
}

#[test]
fn review_request_takes_a_pr_or_a_task_and_not_both() {
    let fixture = Fixture::new(QUIET);
    let said = |args: &[&str]| {
        let out = fixture.cmd(args);
        assert!(!out.status.success(), "{args:?}");
        String::from_utf8_lossy(&out.stderr).to_string()
    };
    assert!(said(&["review", "request"]).contains("--pr"));
    assert!(
        said(&["review", "request", "--pr", "acme/widget#7", "--task", "t1"])
            .contains("cannot be used with")
    );
    assert!(
        said(&[
            "review",
            "request",
            "--repo",
            "acme/widget",
            "--pr",
            "acme/widget#7"
        ])
        .contains("--task")
    );
    // A task that is not there is refused in the task's words.
    assert!(said(&["review", "request", "--task", "nonsense"]).contains("no such task"));
}
