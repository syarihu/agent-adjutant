//! The issue's own title and body, kept on a task's record when it starts, with `gh`
//! stubbed so nothing here reaches GitHub.

mod common;

use common::*;

const ISSUE: &str = "https://github.com/acme/widget/issues";

/// A `gh` that knows three issues, and writes down every one it is asked about.
///
/// 1 answers with the title in `gh-title` (so a test can change what it says between calls),
/// 2 with a body of 40 KB, and anything else fails the way `gh` does for an issue it cannot
/// find. Once `gh-fail` exists, everything does.
fn stub_gh(fixture: &Fixture) -> (String, PathBuf) {
    let stubs = fixture.repo.join("stub-bin");
    std::fs::create_dir_all(&stubs).unwrap();
    let asked = fixture.repo.join("gh-asked");
    let title = fixture.repo.join("gh-title");
    let fail = fixture.repo.join("gh-fail");
    std::fs::write(&title, "T").unwrap();
    let gh = stubs.join("gh");
    std::fs::write(
        &gh,
        format!(
            "#!/bin/sh\n\
             for a; do u=$a; done\n\
             echo \"$u\" >> {asked}\n\
             [ -e {fail} ] && {{ echo 'Could not resolve to an issue' >&2; exit 1; }}\n\
             case \"$u\" in\n\
             {ISSUE}/1) printf '{{\"title\":\"%s\",\"body\":\"B\"}}' \"$(cat {title})\" ;;\n\
             {ISSUE}/2) printf '{{\"title\":\"T\",\"body\":\"'; head -c 40000 /dev/zero | tr '\\0' x; printf '\"}}' ;;\n\
             *) echo 'Could not resolve to an issue' >&2; exit 1 ;;\n\
             esac\n",
            asked = shell_quoted(&asked.to_string_lossy()),
            title = shell_quoted(&title.to_string_lossy()),
            fail = shell_quoted(&fail.to_string_lossy()),
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        stubs.to_string_lossy(),
        std::env::var("PATH").unwrap_or_default()
    );
    (path, asked)
}

fn asked(log: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn run(fixture: &Fixture, path: &str, args: &[&str]) -> std::process::Output {
    fixture.command(args).env("PATH", path).output().unwrap()
}

fn run_json(fixture: &Fixture, path: &str, args: &[&str]) -> serde_json::Value {
    let out = run(fixture, path, args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn shown(fixture: &Fixture, id: &str) -> serde_json::Value {
    fixture.json(&["task", "show", "--id", id])
}

/// A backlog task that points at `url`.
fn task_at(fixture: &Fixture, path: &str, url: &str) -> String {
    let added = run_json(
        fixture,
        path,
        &[
            "task",
            "add",
            "--title",
            "t",
            "--body",
            "t",
            "--issue-url",
            url,
            "--json",
        ],
    );
    added["task"]["id"].as_str().unwrap().to_string()
}

fn start(fixture: &Fixture, path: &str, id: &str, status: &str) -> std::process::Output {
    run(
        fixture,
        path,
        &[
            "task",
            "update",
            "--id",
            id,
            "--status",
            status,
            "--no-hand-over",
            "--json",
        ],
    )
}

#[test]
fn a_task_is_read_when_it_starts_and_not_before_or_again() {
    let fixture = Fixture::new(QUIET);
    let (path, log) = stub_gh(&fixture);
    let id = task_at(&fixture, &path, &format!("{ISSUE}/1"));
    assert!(shown(&fixture, &id).get("issueSnapshot").is_none());
    assert!(asked(&log).is_empty(), "a backlog task was read");

    let out = start(&fixture, &path, &id, "dispatched");
    assert!(out.status.success());
    let updated: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // The answer of the command that read it already carries it.
    assert_eq!(updated["task"]["issueSnapshot"]["title"], "T", "{updated}");
    let snap = &shown(&fixture, &id)["issueSnapshot"];
    assert_eq!(snap["url"], format!("{ISSUE}/1"));
    assert_eq!(snap["body"], "B");
    assert!(snap.get("truncated").is_none(), "{snap}");
    assert!(snap["fetchedAt"].as_str().unwrap().ends_with('Z'), "{snap}");
    assert_eq!(asked(&log), [format!("{ISSUE}/1")]);

    assert!(start(&fixture, &path, &id, "pr").status.success());
    assert_eq!(asked(&log).len(), 1, "an issue already read was read again");
}

#[test]
fn a_failing_gh_does_not_fail_the_update() {
    let fixture = Fixture::new(QUIET);
    let (path, _) = stub_gh(&fixture);
    let id = task_at(&fixture, &path, &format!("{ISSUE}/9"));
    let out = start(&fixture, &path, &id, "dispatched");
    assert!(out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("could not read the issue: Could not resolve to an issue"),
        "{said}"
    );
    let record = shown(&fixture, &id);
    assert_eq!(record["status"], "dispatched");
    assert!(record.get("issueSnapshot").is_none(), "{record}");
}

#[test]
fn an_issue_recorded_after_the_task_started_is_read_and_a_long_body_is_cut() {
    let fixture = Fixture::new(QUIET);
    let (path, log) = stub_gh(&fixture);
    let added = run_json(
        &fixture,
        &path,
        &[
            "task",
            "add",
            "--title",
            "file it",
            "--body",
            "b",
            "--kind",
            "file-and-start",
            "--json",
        ],
    );
    let id = added["task"]["id"].as_str().unwrap().to_string();
    assert!(start(&fixture, &path, &id, "dispatched").status.success());
    assert!(asked(&log).is_empty());

    let out = run(
        &fixture,
        &path,
        &[
            "task",
            "update",
            "--id",
            &id,
            "--issue",
            &format!("{ISSUE}/2"),
        ],
    );
    assert!(out.status.success());
    let snap = &shown(&fixture, &id)["issueSnapshot"];
    assert_eq!(snap["truncated"], true, "{snap}");
    assert_eq!(snap["body"].as_str().unwrap().len(), 16 * 1024);
}

#[test]
fn another_tracker_is_never_asked_about() {
    let fixture = Fixture::new(QUIET);
    let (path, log) = stub_gh(&fixture);
    let id = task_at(&fixture, &path, "https://linear.app/acme/issue/WID-1/thing");
    let out = start(&fixture, &path, &id, "dispatched");
    assert!(out.status.success());
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(asked(&log).is_empty());
    assert!(shown(&fixture, &id).get("issueSnapshot").is_none());
    // Asked to, it says why there is nothing to do.
    let out = run(&fixture, &path, &["task", "fetch-issue", "--id", &id]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no GitHub issue to read"));
}

#[test]
fn fetch_issue_reads_again_and_a_failure_keeps_what_was_kept() {
    let fixture = Fixture::new(QUIET);
    let (path, log) = stub_gh(&fixture);
    let id = task_at(&fixture, &path, &format!("{ISSUE}/1"));
    assert!(start(&fixture, &path, &id, "dispatched").status.success());
    assert_eq!(shown(&fixture, &id)["issueSnapshot"]["title"], "T");

    std::fs::write(fixture.repo.join("gh-title"), "Edited").unwrap();
    let out = run_json(
        &fixture,
        &path,
        &["task", "fetch-issue", "--id", &id, "--json"],
    );
    assert_eq!(out["task"]["issueSnapshot"]["title"], "Edited", "{out}");
    assert_eq!(shown(&fixture, &id)["issueSnapshot"]["title"], "Edited");
    assert_eq!(asked(&log).len(), 2);

    // The issue can no longer be read.
    std::fs::write(fixture.repo.join("gh-fail"), "").unwrap();
    let out = run(&fixture, &path, &["task", "fetch-issue", "--id", &id]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Could not resolve"));
    assert_eq!(shown(&fixture, &id)["issueSnapshot"]["title"], "Edited");
}

#[test]
fn the_board_reads_on_a_click_only_and_takes_no_snapshot_from_a_caller() {
    let fixture = Fixture::new(QUIET);
    let (path, log) = stub_gh(&fixture);
    let id = task_at(&fixture, &path, &format!("{ISSUE}/1"));
    let bare = fixture.json(&["task", "add", "--title", "bare", "--body", "b", "--json"]);
    let bare = bare["task"]["id"].as_str().unwrap().to_string();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);

    let (status, state) = resident.get(&format!("/b/{SLUG}/api/state"));
    assert_eq!(status, 200, "{state}");
    assert!(asked(&log).is_empty(), "the board polled the tracker");

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/tasks/{id}/issue"), "");
    assert_eq!(status, 200, "{body}");
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["task"]["issueSnapshot"]["title"], "T", "{body}");

    let (_, state) = resident.get(&format!("/b/{SLUG}/api/state"));
    let state: serde_json::Value = serde_json::from_str(&state).unwrap();
    let card = state["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id.as_str())
        .unwrap_or_else(|| panic!("no {id} in {state}"));
    assert_eq!(card["issueSnapshot"]["body"], "B", "{card}");
    assert_eq!(asked(&log).len(), 1, "reading the state asked the tracker");

    let (status, body) = resident.post(&format!("/b/{SLUG}/api/tasks/{bare}/issue"), "");
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("no GitHub issue to read"), "{body}");

    let forged = r#"{"title":"forged","issueSnapshot":{"url":"u","title":"x","fetchedAt":"s"}}"#;
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/tasks"), forged);
    assert_eq!(status, 200, "{body}");
    let made: serde_json::Value = serde_json::from_str(&body).unwrap();
    let forged_id = made["task"]["id"].as_str().unwrap();
    assert!(made["task"].get("issueSnapshot").is_none(), "{made}");
    assert!(shown(&fixture, forged_id).get("issueSnapshot").is_none());
}

#[test]
fn a_later_update_does_not_ask_about_an_issue_gh_could_not_read() {
    let fixture = Fixture::new(QUIET);
    let (path, log) = stub_gh(&fixture);
    let id = task_at(&fixture, &path, &format!("{ISSUE}/9"));
    assert!(start(&fixture, &path, &id, "dispatched").status.success());
    assert_eq!(asked(&log).len(), 1);

    let out = run(
        &fixture,
        &path,
        &["task", "update", "--id", &id, "--note", "x"],
    );
    assert!(out.status.success());
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(asked(&log).len(), 1, "a plain update asked gh again");
}

#[test]
fn a_started_task_whose_issue_moves_is_read_again() {
    let fixture = Fixture::new(QUIET);
    let (path, log) = stub_gh(&fixture);
    let added = run_json(
        &fixture,
        &path,
        &["task", "add", "--title", "t", "--body", "t", "--json"],
    );
    let id = added["task"]["id"].as_str().unwrap().to_string();
    assert!(start(&fixture, &path, &id, "dispatched").status.success());
    for n in [1, 2] {
        let issue = format!("{ISSUE}/{n}");
        let out = run(
            &fixture,
            &path,
            &["task", "update", "--id", &id, "--issue", &issue],
        );
        assert!(out.status.success());
        assert_eq!(shown(&fixture, &id)["issueSnapshot"]["url"], issue);
    }
    assert_eq!(asked(&log), [format!("{ISSUE}/1"), format!("{ISSUE}/2")]);
}

fn post_task(resident: &Resident, body: &str) -> (u16, serde_json::Value) {
    let (status, text) = resident.post(&format!("/b/{SLUG}/api/tasks"), body);
    let value = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
    (status, value)
}

#[test]
fn an_issue_url_alone_is_read_when_the_request_is_made() {
    let fixture = Fixture::new(QUIET);
    let (path, log) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);

    let (status, made) = post_task(&resident, &format!(r#"{{"issueUrl":"{ISSUE}/1"}}"#));
    assert_eq!(status, 200, "{made}");
    let task = &made["task"];
    assert_eq!(task["title"], "T", "{made}");
    assert_eq!(task["issueSnapshot"]["body"], "B", "{made}");
    assert!(task.get("titlePending").is_none(), "{made}");
    assert!(task.get("body").is_none(), "{made}");
    assert_eq!(asked(&log), [format!("{ISSUE}/1")]);

    // What the person typed wins, and the issue is still read.
    let (status, made) = post_task(
        &resident,
        &format!(r#"{{"title":"mine","issueUrl":"{ISSUE}/1"}}"#),
    );
    assert_eq!(status, 200, "{made}");
    assert_eq!(made["task"]["title"], "mine", "{made}");
    assert_eq!(made["task"]["issueSnapshot"]["title"], "T", "{made}");
    assert_eq!(asked(&log).len(), 2);

    // A typed body means no read at all.
    let (status, made) = post_task(
        &resident,
        &format!(r#"{{"body":"first line\nmore","issueUrl":"{ISSUE}/1"}}"#),
    );
    assert_eq!(status, 200, "{made}");
    assert_eq!(made["task"]["title"], "first line", "{made}");
    assert!(made["task"].get("issueSnapshot").is_none(), "{made}");
    assert_eq!(asked(&log).len(), 2, "a typed body still asked gh");
}

#[test]
fn an_unreadable_issue_keeps_the_record_under_its_name_until_it_is_read() {
    let fixture = Fixture::new(QUIET);
    let (path, log) = stub_gh(&fixture);
    std::fs::write(fixture.repo.join("gh-fail"), "").unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);

    let (status, made) = post_task(&resident, &format!(r#"{{"issueUrl":"{ISSUE}/1"}}"#));
    assert_eq!(status, 200, "{made}");
    assert_eq!(made["task"]["title"], "acme/widget#1", "{made}");
    assert_eq!(made["task"]["titlePending"], true, "{made}");
    assert!(made["task"].get("issueSnapshot").is_none(), "{made}");
    let id = made["task"]["id"].as_str().unwrap().to_string();
    assert_eq!(asked(&log).len(), 1);

    std::fs::remove_file(fixture.repo.join("gh-fail")).unwrap();
    // A read from the board's button is enough, without starting the task.
    let (status, read) = resident.post(&format!("/b/{SLUG}/api/tasks/{id}/issue"), "");
    assert_eq!(status, 200, "{read}");
    let read: serde_json::Value = serde_json::from_str(&read).unwrap();
    assert_eq!(read["task"]["title"], "T", "{read}");
    assert!(read["task"].get("titlePending").is_none(), "{read}");
    assert_eq!(shown(&fixture, &id)["title"], "T");

    // And the start path, for a record that was not read before it.
    std::fs::write(fixture.repo.join("gh-fail"), "").unwrap();
    let (_, made) = post_task(&resident, &format!(r#"{{"issueUrl":"{ISSUE}/1"}}"#));
    let id = made["task"]["id"].as_str().unwrap().to_string();
    std::fs::remove_file(fixture.repo.join("gh-fail")).unwrap();
    assert!(start(&fixture, &path, &id, "dispatched").status.success());
    let task = shown(&fixture, &id);
    assert_eq!(task["title"], "T", "{task}");
    assert!(task.get("titlePending").is_none(), "{task}");
    assert_eq!(task["issueSnapshot"]["body"], "B", "{task}");
}

#[test]
fn a_request_without_content_or_a_readable_issue_is_refused() {
    let fixture = Fixture::new(QUIET);
    let (path, log) = stub_gh(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);

    let (status, made) = post_task(&resident, "{}");
    assert_eq!(status, 400, "{made}");
    let other = r#"{"issueUrl":"https://linear.app/x/issue/ABC-1"}"#;
    let (status, made) = post_task(&resident, other);
    assert_eq!(status, 400, "{made}");
    assert!(asked(&log).is_empty(), "gh was asked");

    // Only a task that starts an issue is made from the issue alone.
    let investigate = format!(r#"{{"kind":"investigate","issueUrl":"{ISSUE}/1"}}"#);
    let (status, made) = post_task(&resident, &investigate);
    assert_eq!(status, 400, "{made}");
    assert!(asked(&log).is_empty(), "gh was asked");

    // The flag is the server's to set.
    let forged = r#"{"title":"t","body":"b","titlePending":true}"#;
    let (status, made) = post_task(&resident, forged);
    assert_eq!(status, 200, "{made}");
    assert!(made["task"].get("titlePending").is_none(), "{made}");
    let id = made["task"]["id"].as_str().unwrap();
    assert!(shown(&fixture, id).get("titlePending").is_none());
}
