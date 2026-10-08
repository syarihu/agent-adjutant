//! What the resident does on its own clock: the gate sweep, the parent-task titles and the PR
//! poll, each run against stub tools.

mod common;

use common::*;

/// Open a waiting question gate in `worktree`, with `openedAt` written as given so a test
/// does not have to sleep to get gates in order.
fn open_question_in(fixture: &Fixture, worktree: &Path, kind: &str, opened_at: &str) -> String {
    let file = fixture.repo.join("gate.json");
    std::fs::write(
        &file,
        serde_json::json!({
            "kind": kind,
            "task": "t-1",
            "title": "どちらにするか",
            "worktree": worktree.to_str().unwrap(),
        })
        .to_string(),
    )
    .unwrap();
    let opened = fixture.json(&["gate", "open", "--file", file.to_str().unwrap(), "--json"]);
    let id = opened["gate"]["id"].as_str().unwrap().to_string();
    let path = fixture
        .state
        .join("gates")
        .join(SLUG)
        .join(format!("{id}.json"));
    let mut gate: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    gate["openedAt"] = serde_json::json!(opened_at);
    std::fs::write(&path, gate.to_string()).unwrap();
    id
}

fn open_ids(resident: &Resident) -> Vec<String> {
    let (status, body) = resident.get(&format!("/b/{SLUG}/api/state"));
    assert_eq!(status, 200, "{body}");
    let state: serde_json::Value = serde_json::from_str(&body).unwrap();
    state["gates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["id"].as_str().unwrap().to_string())
        .collect()
}

/// Long enough for a sweep that starts after now to have run: every board sweeps its gates
/// every two seconds.
fn after_a_sweep() {
    std::thread::sleep(std::time::Duration::from_millis(2_500));
}

fn archived_gate(fixture: &Fixture, id: &str) -> Option<serde_json::Value> {
    let path = fixture
        .state
        .join("gates")
        .join(SLUG)
        .join("answered")
        .join(format!("{id}.json"));
    Some(serde_json::from_str(&std::fs::read_to_string(path).ok()?).unwrap())
}

#[test]
fn a_gate_whose_worker_moved_to_a_later_phase_is_closed_as_answered_in_the_terminal() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let worktree = fixture.repo.clone();
    write_worker(&worktree, "20260922T040000Z", Some(LATER_SECS));
    let id = open_question_in(&fixture, &worktree, "question", "20260922T041233Z");

    wait_until("the gate closed by a sweep", || {
        let open = open_ids(&resident);
        (!open.contains(&id), open)
    });
    let gate = archived_gate(&fixture, &id).expect("archived");
    assert_eq!(gate["decision"], "terminal", "{gate}");
    assert_eq!(gate["answeredAt"], LATER_STAMP, "{gate}");
    assert!(
        gate["comment"].as_str().unwrap().contains("implement"),
        "{gate}"
    );
}

#[test]
fn a_gate_stays_open_unless_the_same_worker_visibly_moved_on() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let worktree = fixture.repo.clone();

    // A worker started after the gate was opened is not the one that opened it.
    write_worker(&worktree, "20260922T045000Z", Some(LATER_SECS));
    let late_worker = open_question_in(&fixture, &worktree, "question", "20260922T041233Z");
    after_a_sweep();
    assert!(open_ids(&resident).contains(&late_worker));

    // Nothing happened after it was opened.
    write_worker(&worktree, "20260922T040000Z", None);
    after_a_sweep();
    assert!(open_ids(&resident).contains(&late_worker));
    std::fs::remove_file(
        fixture
            .state
            .join("gates")
            .join(SLUG)
            .join(format!("{late_worker}.json")),
    )
    .unwrap();

    // The hub's gates are never swept, whatever the worktree's worker did.
    write_worker(&worktree, "20260922T040000Z", Some(LATER_SECS));
    let dispatch = open_question_in(&fixture, &worktree, "dispatch", "20260922T041233Z");
    after_a_sweep();
    assert!(open_ids(&resident).contains(&dispatch));
    assert!(archived_gate(&fixture, &dispatch).is_none());

    // No worker record at all.
    std::fs::remove_file(worktree.join(".claude").join("adjutant-worker.json")).unwrap();
    let orphan = open_question_in(&fixture, &worktree, "question", "20260922T041233Z");
    after_a_sweep();
    assert!(open_ids(&resident).contains(&orphan));
}

#[test]
fn opening_a_later_gate_closes_the_earlier_one_as_answered_in_the_terminal() {
    let fixture = Fixture::new(QUIET);
    let worktree = fixture.repo.clone();
    write_worker(&worktree, "20260922T040000Z", None);
    let first = open_question_in(&fixture, &worktree, "question", "20260922T041233Z");
    let second = open_question_in(&fixture, &worktree, "question", "20260922T042000Z");
    // Started only now: a sweep between `adj gate open` stamping the time and the helper
    // rewriting `openedAt` would close the first gate with a real answeredAt.
    let resident = Resident::start(&fixture);

    let open = wait_until("the earlier gate closed by a sweep", || {
        let open = open_ids(&resident);
        (!open.contains(&first), open)
    });
    assert!(!open.contains(&first), "{open:?}");
    assert!(open.contains(&second), "{open:?}");
    let gate = archived_gate(&fixture, &first).expect("archived");
    assert_eq!(gate["decision"], "terminal", "{gate}");
    assert_eq!(gate["answeredAt"], "20260922T042000Z", "{gate}");
    assert!(
        gate["comment"].as_str().unwrap().contains(&second),
        "{gate}"
    );
}

// ── the parent task's title on a parent-task hub ──

/// A `gh` that answers every issue with the title in `gh-title`, fails once `gh-fail` exists, and
/// writes down the URL of each issue it is asked about. Returns the `PATH` to run the server
/// with and that log.
fn stub_gh_for_titles(fixture: &Fixture) -> (String, PathBuf) {
    let stubs = fixture.repo.join("stub-bin");
    let asked = fixture.repo.join("gh-asked");
    let fail = fixture.repo.join("gh-fail");
    stub_bin(
        &stubs,
        "gh",
        &format!(
            "#!/bin/sh\n\
             for a; do u=$a; done\n\
             echo \"$u\" >> {asked}\n\
             [ -e {fail} ] && {{ echo 'Could not resolve to an issue' >&2; exit 1; }}\n\
             printf '{{\"title\":\"The parent task\",\"body\":\"B\"}}'\n",
            asked = shell_quoted(&asked.to_string_lossy()),
            fail = shell_quoted(&fail.to_string_lossy()),
        ),
    );
    let path = path_with(&stubs);
    (path, asked)
}

fn gh_asked(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// The `title` of the hub `hub-wid-957` in the state, null while it has none.
fn feature_title(resident: &Resident) -> serde_json::Value {
    state_of(resident)["hubs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["id"] == "hub-wid-957")
        .unwrap_or_else(|| panic!("no hub-wid-957"))["title"]
        .clone()
}

/// Polls until the hub has a title, or panics with what it last saw.
fn wait_for_feature_title(resident: &Resident) -> serde_json::Value {
    wait_until("the parent hub to have a title", || {
        let title = feature_title(resident);
        (!title.is_null(), title)
    })
}

/// Polls long enough for a read that was going to start to have started and finished.
fn poll_feature_title(resident: &Resident, times: usize) {
    for _ in 0..times {
        let _ = feature_title(resident);
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

#[test]
fn a_parent_hubs_title_is_read_once_and_kept_across_a_restart() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let (path, asked) = stub_gh_for_titles(&fixture);
    {
        let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
        assert_eq!(wait_for_feature_title(&resident), "The parent task");
    }
    assert_eq!(
        gh_asked(&asked),
        ["https://github.com/acme/widget/issues/957"]
    );
    // The repository's own hub has no parent task, so no title.
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    assert_eq!(wait_for_feature_title(&resident), "The parent task");
    poll_feature_title(&resident, 5);
    assert_eq!(gh_asked(&asked).len(), 1, "{:?}", gh_asked(&asked));
    let hubs = state_of(&resident)["hubs"].clone();
    assert!(hubs[0]["title"].is_null(), "{hubs}");
}

#[test]
fn a_parent_key_no_issue_key_matches_is_never_asked_about() {
    let fixture = Fixture::new(
        r#"{"notification": "true", "defaults": {"ide": "code"},
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
                                      "issueKeys": {"acme/widget": "GAMMA"}}}}"#,
    );
    listed_parent_hub(&fixture);
    let (path, asked) = stub_gh_for_titles(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    poll_feature_title(&resident, 8);
    assert!(feature_title(&resident).is_null());
    assert!(gh_asked(&asked).is_empty(), "{:?}", gh_asked(&asked));
}

#[test]
fn the_parent_a_child_task_names_is_preferred_to_the_issue_key() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let tasks = fixture.state.join("tasks").join(FEATURE_SLUG);
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        tasks.join("child.json"),
        serde_json::json!({
            "id": "child", "kind": "investigate", "title": "Child", "doneWhen": "report-only",
            "autoStart": true, "status": "backlog", "createdAt": "20260101T000000Z",
            "updatedAt": "20260101T000000Z",
            "parent": "https://github.com/acme/elsewhere/issues/5",
        })
        .to_string(),
    )
    .unwrap();
    let (path, asked) = stub_gh_for_titles(&fixture);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    assert_eq!(wait_for_feature_title(&resident), "The parent task");
    assert_eq!(
        gh_asked(&asked),
        ["https://github.com/acme/elsewhere/issues/5"]
    );
}

#[test]
fn an_issue_that_could_not_be_read_is_not_asked_for_again_on_the_next_poll() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let (path, asked) = stub_gh_for_titles(&fixture);
    std::fs::write(fixture.repo.join("gh-fail"), "").unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    // Wait for the one read to have happened, so a slow machine does not pass with none.
    // Generous: the read starts on a thread of its own, and a loaded machine is slow to run it.
    wait_until("the stub gh to be asked", || {
        let _ = feature_title(&resident);
        let lines = gh_asked(&asked);
        (!lines.is_empty(), lines)
    });
    poll_feature_title(&resident, 10);
    assert!(feature_title(&resident).is_null());
    assert_eq!(gh_asked(&asked).len(), 1, "{:?}", gh_asked(&asked));
}

// ── the pull requests the resident keeps up to date ──

const POLLED_PR: &str = "https://github.com/acme/widget/pull/7";

/// A card at `pr`, pointing at `url`, made before the server starts: the poll looks at the
/// records when it begins, and a card added later waits for its next round.
fn card_on_pr(fixture: &Fixture, url: &str) -> String {
    let added = fixture.json(&["task", "add", "--title", "A card", "--body", "b", "--json"]);
    let id = added["task"]["id"].as_str().unwrap().to_string();
    fixture.ok(&[
        "task",
        "update",
        "--id",
        &id,
        "--status",
        "pr",
        "--pr",
        url,
        "--no-hand-over",
    ]);
    id
}

/// What `gh api graphql` prints for PR 7. GitHub's upper-case values are passed in rather than
/// written into the JSON: a bare upper-case string after a colon reads as a tracker key to the
/// guard over everything that ships.
fn graphql_of_pr(state: &str, decision: &str) -> String {
    format!(
        r#"{{"data":{{"p0":{{"pullRequest":{{"state":"{state}","isDraft":false,"title":"A pull request","reviewDecision":"{decision}","reviewRequests":{{"totalCount":0}},"commits":{{"nodes":[]}}}}}}}}}}"#
    )
}

/// A `gh` for the poll. Notifications answer 200 with `Last-Modified: Thu, 01 Jan 2026 00:00:00 GMT` and one thread, the
/// PR 7, the first time and 304 (which `gh` exits 1 on) while asked with `If-Modified-Since`,
/// until a file `changed` exists: then one 200, with `Last-Modified: Thu, 01 Jan 2026 00:00:05 GMT`, and the file is
/// taken away. GraphQL answers with `gh-graphql`, and the query for the pull request of a
/// branch (one that asks `pullRequests(headRefName`) with `gh-branch-graphql`, counted as `branch`;
/// the query for the parent of an issue (one that names `parent{`) with `gh-parent-graphql`,
/// counted as `parent`, or fails while a file `gh-parent-fail` exists. Every call is written down in `gh-calls`,
/// whole, so a test can see what was sent.
fn stub_gh_for_polling(fixture: &Fixture, interval: u64) -> String {
    let stubs = fixture.repo.join("stub-bin");
    let dir = fixture.repo.to_string_lossy().to_string();
    let script = r#"#!/bin/sh
D=@DIR@
echo "$*" >> "$D/gh-calls"
if [ "$1 $2" = "api graphql" ]; then
  case "$*" in
    *'parent{'*)
      echo parent >> "$D/gh-kinds"
      if [ -e "$D/gh-parent-fail" ]; then
        echo "gh: could not reach GitHub" >&2
        exit 1
      fi
      cat "$D/gh-parent-graphql"
      exit $?
      ;;
    *'pullRequests(headRefName'*)
      echo branch >> "$D/gh-kinds"
      cat "$D/gh-branch-graphql"
      exit $?
      ;;
  esac
  echo graphql >> "$D/gh-kinds"
  cat "$D/gh-graphql"
  exit 0
fi
echo notifications >> "$D/gh-kinds"
case "$*" in
  *If-Modified-Since*)
    if [ -e "$D/changed" ]; then
      rm "$D/changed"
      printf 'HTTP/2.0 200 OK\r\nLast-Modified: Thu, 01 Jan 2026 00:00:05 GMT\r\nX-Poll-Interval: @INTERVAL@\r\n\r\n'
      printf '[{"subject":{"type":"PullRequest","url":"https://api.github.com/repos/acme/widget/pulls/7"},"repository":{"full_name":"acme/widget"}}]'
      exit 0
    fi
    printf 'HTTP/2.0 304 Not Modified\r\nX-Poll-Interval: @INTERVAL@\r\n\r\n'
    exit 1
    ;;
esac
printf 'HTTP/2.0 200 OK\r\nLast-Modified: Thu, 01 Jan 2026 00:00:00 GMT\r\nX-Poll-Interval: @INTERVAL@\r\n\r\n'
printf '[{"subject":{"type":"PullRequest","url":"https://api.github.com/repos/acme/widget/pulls/7"},"repository":{"full_name":"acme/widget"}}]'
"#
    .replace("@DIR@", &shell_quoted(&dir))
    .replace("@INTERVAL@", &interval.to_string());
    stub_bin(&stubs, "gh", &script);
    std::fs::write(
        fixture.repo.join("gh-graphql"),
        graphql_of_pr("OPEN", "APPROVED"),
    )
    .unwrap();
    std::fs::write(
        fixture.repo.join("gh-parent-graphql"),
        graphql_of_parent(None),
    )
    .unwrap();
    path_with(&stubs)
}

/// What `gh api graphql` prints for the branch query of one branch whose only pull request is
/// number 12. Upper-case values are passed in, as for `graphql_of_pr`.
fn graphql_of_branch(state: &str, owner: &str) -> String {
    format!(
        r#"{{"data":{{"b0":{{"open":{{"nodes":[]}},"recent":{{"nodes":[{{"number":12,"url":"https://github.com/acme/widget/pull/12","state":"{state}","isDraft":false,"headRepositoryOwner":{{"login":"{owner}"}}}}]}}}}}}}}"#
    )
}

/// What `gh api graphql` prints for the parent query of one issue: the parent issue 549 with
/// nine sub-issues, or none. Upper-case values are passed in, as for `graphql_of_pr`.
fn graphql_of_parent(parent: Option<&str>) -> String {
    let issue = match parent {
        Some(state) => format!(
            r#"{{"parent":{{"url":"https://github.com/acme/widget/issues/549","number":549,"title":"The parent","state":"{state}","subIssuesSummary":{{"total":9,"completed":2}}}}}}"#
        ),
        None => r#"{"parent":null}"#.to_string(),
    };
    format!(r#"{{"data":{{"i0":{{"issue":{issue}}}}}}}"#)
}

/// How many times the stub `gh` was run for `kind`: `notifications`, `graphql`, `branch` or
/// `parent`.
fn gh_ran(fixture: &Fixture, kind: &str) -> usize {
    std::fs::read_to_string(fixture.repo.join("gh-kinds"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == kind)
        .count()
}

fn turn_of(resident: &Resident, id: &str) -> serde_json::Value {
    state_of(resident)["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap_or_else(|| panic!("no task {id}"))["prTurn"]
        .clone()
}

#[test]
fn a_card_follows_its_pr_without_anybody_asking() {
    let fixture = Fixture::new(QUIET);
    let id = card_on_pr(&fixture, POLLED_PR);
    let path = stub_gh_for_polling(&fixture, 1);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);

    // The first round reads the PR: approved, so it is the person's to merge.
    wait_until("the PR was read and kept", || {
        let shown = fixture.json(&["task", "show", "--id", &id]);
        (shown["prStatus"]["review"] == "approved", shown)
    });
    assert_eq!(turn_of(&resident, &id), "merge");
    assert_eq!(board_of(&resident, SLUG)["waiting"], 1);
    assert_eq!(gh_ran(&fixture, "graphql"), 1);

    // Nothing is said to have changed, so the PR is not read again, even though GitHub would
    // now answer differently.
    std::fs::write(
        fixture.repo.join("gh-graphql"),
        graphql_of_pr("MERGED", "APPROVED"),
    )
    .unwrap();
    let rounds = gh_ran(&fixture, "notifications");
    wait_until("another round asked for news", || {
        let now = gh_ran(&fixture, "notifications");
        (now >= rounds + 2, now)
    });
    assert_eq!(gh_ran(&fixture, "graphql"), 1);
    assert_eq!(fixture.json(&["task", "show", "--id", &id])["status"], "pr");

    // A notification for the PR: it is read, and being merged it is done.
    std::fs::write(fixture.repo.join("changed"), "").unwrap();
    wait_until("the merged PR was moved to done", || {
        let shown = fixture.json(&["task", "show", "--id", &id]);
        (shown["status"] == "done", shown)
    });
    assert_eq!(gh_ran(&fixture, "graphql"), 2);
    assert_eq!(board_of(&resident, SLUG)["waiting"], 0);

    // Only ever read: nothing is marked as read, nor written to in any other way.
    let calls = std::fs::read_to_string(fixture.repo.join("gh-calls")).unwrap();
    for sent in calls.lines() {
        for write in ["PATCH", "PUT", "POST", "DELETE", "--method", "-X"] {
            assert!(!sent.contains(write), "{write} in {sent}");
        }
    }
    // And the second round sent back what the first one was told, asking only for the threads
    // changed since, so a page of old read ones is not taken for news.
    assert!(
        calls.lines().any(
            |c| c.contains("If-Modified-Since: Thu, 01 Jan 2026 00:00:00 GMT")
                && c.contains("participating=true")
                && c.contains("since=2026-01-01T00:00:00Z")
        ),
        "{calls}"
    );
    // The first round had no stamp, so it asked for no `since`.
    let first = calls.lines().next().unwrap();
    assert!(!first.contains("since="), "{first}");
}

#[test]
fn the_board_poll_never_reaches_github_and_a_long_interval_is_kept() {
    let fixture = Fixture::new(QUIET);
    let id = card_on_pr(&fixture, POLLED_PR);
    let path = stub_gh_for_polling(&fixture, 3600);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    wait_until("the PR was read", || {
        let shown = fixture.json(&["task", "show", "--id", &id]);
        (shown["prStatus"].is_object(), shown)
    });
    let before = std::fs::read_to_string(fixture.repo.join("gh-calls")).unwrap();
    for _ in 0..10 {
        let _ = state_of(&resident);
        let _ = boards_of(&resident);
    }
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let after = std::fs::read_to_string(fixture.repo.join("gh-calls")).unwrap();
    assert_eq!(before, after, "something asked GitHub again");
    assert!(state_of(&resident)["prPoll"]["error"].is_null());
    assert_eq!(state_of(&resident)["prPoll"]["active"], true);
}

#[test]
fn with_no_pr_on_a_card_github_is_never_asked() {
    let fixture = Fixture::new(QUIET);
    // A card whose PR is on another host is not polled either.
    card_on_pr(&fixture, "https://example.com/acme/widget/pull/7");
    let path = stub_gh_for_polling(&fixture, 1);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    std::thread::sleep(std::time::Duration::from_millis(2500));
    assert!(!fixture.repo.join("gh-calls").exists());
    assert!(state_of(&resident)["prPoll"]["error"].is_null());
}

/// A card with no PR on `issue`, made before the server starts. Its body is given, so nothing
/// reads the issue through the real `gh` while it is made.
fn card_on_issue(fixture: &Fixture, issue: &str, parent: Option<&str>) -> String {
    let mut args = vec![
        "task",
        "add",
        "--title",
        "A child",
        "--body",
        "b",
        "--issue-url",
        issue,
        "--json",
    ];
    if let Some(parent) = parent {
        args.extend(["--parent", parent]);
    }
    let added = fixture.json(&args);
    added["task"]["id"].as_str().unwrap().to_string()
}

fn parent_of(resident: &Resident, id: &str) -> serde_json::Value {
    state_of(resident)["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap_or_else(|| panic!("no task {id}"))["parentIssue"]
        .clone()
}

const ISSUE: &str = "https://github.com/acme/widget/issues/20";

/// Waits for the first `parent` query, then a moment more, and asserts there was still only one:
/// bounded on both sides, so a slow machine does not pass with none and a poll that asks every
/// round does not pass by being quick.
fn parent_asked_exactly_once(fixture: &Fixture) {
    wait_until("the parent was asked for", || {
        let ran = gh_ran(fixture, "parent");
        (ran >= 1, ran)
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(gh_ran(fixture, "parent"), 1);
}

#[test]
fn a_task_with_no_pr_gets_the_parent_the_tracker_names_and_the_state_does_not_ask_again() {
    let fixture = Fixture::new(QUIET);
    let id = card_on_issue(
        &fixture,
        ISSUE,
        Some("https://github.com/acme/widget/issues/99"),
    );
    let path = stub_gh_for_polling(&fixture, 1);
    std::fs::write(
        fixture.repo.join("gh-parent-graphql"),
        graphql_of_parent(Some("OPEN")),
    )
    .unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);

    // The record names issue 99 and the tracker says 549: the tracker's word is shown.
    let shown = wait_until("the parent was read", || {
        let shown = parent_of(&resident, &id);
        (shown["source"] == "tracker", shown)
    });
    assert_eq!(shown["key"], "acme/widget#549");
    assert_eq!(shown["number"], 549);
    assert_eq!(shown["title"], "The parent");
    let state = state_of(&resident);
    assert_eq!(state["parents"][0]["key"], "acme/widget#549");
    assert_eq!(state["parents"][0]["total"], 9);
    // The record is left as it was.
    assert_eq!(
        fixture.json(&["task", "show", "--id", &id])["parent"],
        "https://github.com/acme/widget/issues/99"
    );
    // One query for the one issue, and neither the PR nor the branch queries; polling the
    // state does not ask again.
    for _ in 0..5 {
        let _ = state_of(&resident);
    }
    parent_asked_exactly_once(&fixture);
    assert_eq!(gh_ran(&fixture, "graphql"), 0);
    assert_eq!(gh_ran(&fixture, "branch"), 0);
}

#[test]
fn a_parent_that_could_not_be_read_leaves_the_records_and_is_not_asked_every_round() {
    let fixture = Fixture::new(QUIET);
    let id = card_on_issue(
        &fixture,
        ISSUE,
        Some("https://github.com/acme/widget/issues/99"),
    );
    let path = stub_gh_for_polling(&fixture, 1);
    std::fs::write(fixture.repo.join("gh-parent-fail"), "").unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    parent_asked_exactly_once(&fixture);
    // The tracker was never heard from, so the record is what there is, and it is asked again
    // only when it is due, not each round.
    let shown = parent_of(&resident, &id);
    assert_eq!(shown["source"], "record");
    assert_eq!(shown["key"], "acme/widget#99");
    assert!(state_of(&resident)["prPoll"]["error"].is_null());
}

#[test]
fn a_task_on_an_issue_of_another_host_asks_the_tracker_nothing() {
    let fixture = Fixture::new(QUIET);
    let id = card_on_issue(
        &fixture,
        "https://git.example.test/acme/widget/issues/20",
        Some("https://git.example.test/acme/widget/issues/99"),
    );
    let path = stub_gh_for_polling(&fixture, 1);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    std::thread::sleep(std::time::Duration::from_millis(2500));
    assert!(!fixture.repo.join("gh-calls").exists());
    assert_eq!(parent_of(&resident, &id)["source"], "record");
}

#[test]
fn a_session_with_no_task_shows_the_pr_of_its_branch_and_a_tasks_worker_does_not() {
    let fixture = Fixture::new(QUIET);
    let running = Sleeper::new();
    session_worktree(&fixture, "feature", None, None, running.pid());
    session_worktree(&fixture, "tasked", None, Some("t-1"), running.pid());
    let path = stub_gh_for_polling(&fixture, 1);
    std::fs::write(
        fixture.repo.join("gh-branch-graphql"),
        graphql_of_branch("OPEN", "acme"),
    )
    .unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);

    // No card holds a PR, and the branch is looked up all the same.
    let feature = wait_until("the PR of the branch was found", || {
        let state = state_of(&resident);
        let session = session_of(&state, "worker-feature").clone();
        (session.get("branchPr").is_some(), session)
    });
    assert_eq!(feature["branchPr"]["number"], 12, "{feature}");
    assert_eq!(feature["branchPr"]["state"], "open");
    assert_eq!(
        feature["branchPr"]["url"],
        "https://github.com/acme/widget/pull/12"
    );
    let state = state_of(&resident);
    assert!(
        session_of(&state, "worker-tasked")
            .get("branchPr")
            .is_none()
    );
    assert!(session_of(&state, "hub").get("branchPr").is_none());
    // One query for the one branch; the cards' own query was never made, and polling the state
    // does not ask again.
    assert_eq!(gh_ran(&fixture, "branch"), 1);
    assert_eq!(gh_ran(&fixture, "graphql"), 0);
}

#[test]
fn a_branch_whose_only_pr_comes_from_a_fork_has_none() {
    let fixture = Fixture::new(QUIET);
    let running = Sleeper::new();
    session_worktree(&fixture, "feature", None, None, running.pid());
    let path = stub_gh_for_polling(&fixture, 1);
    std::fs::write(
        fixture.repo.join("gh-branch-graphql"),
        graphql_of_branch("OPEN", "someone"),
    )
    .unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    wait_until("the branch was asked about", || {
        let ran = gh_ran(&fixture, "branch");
        (ran >= 1, ran)
    });
    // A moment for the answer to be stored, then it is still no PR.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let state = state_of(&resident);
    assert!(
        session_of(&state, "worker-feature")
            .get("branchPr")
            .is_none()
    );
    assert!(state["prPoll"]["error"].is_null());
}

#[test]
fn a_poll_that_cannot_run_says_so_and_leaves_the_card_where_it_was() {
    let fixture = Fixture::new(QUIET);
    let id = card_on_pr(&fixture, POLLED_PR);
    // Only what the server needs besides `gh`: `git`, in a directory of its own.
    let git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let git = PathBuf::from(String::from_utf8_lossy(&git.stdout).trim());
    let only_git = fixture.repo.join("git-only-bin");
    std::fs::create_dir_all(&only_git).unwrap();
    std::os::unix::fs::symlink(&git, only_git.join("git")).unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", only_git.to_str().unwrap())]);
    let error = wait_until("the poll reported its failure", || {
        let error = state_of(&resident)["prPoll"]["error"].clone();
        (error.is_string(), error)
    });
    assert!(error.as_str().unwrap().contains("gh"), "{error}");
    // Nothing was read, so the card is placed as it was before there was a poll.
    assert!(turn_of(&resident, &id).is_null());
    assert_eq!(board_of(&resident, SLUG)["waiting"], 1);
}

#[test]
fn a_pr_that_cannot_be_found_is_not_the_poll_failing() {
    let fixture = Fixture::new(QUIET);
    card_on_pr(&fixture, POLLED_PR);
    let path = stub_gh_for_polling(&fixture, 1);
    let missing = r#"{"data":{"p0":{"pullRequest":null}},"errors":[{"type":"NOT_FOUND","path":["p0","pullRequest"],"message":"Could not resolve to a PullRequest with the number of 7."}]}"#;
    std::fs::write(fixture.repo.join("gh-graphql"), missing).unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    wait_until("the PR was asked about", || {
        let ran = gh_ran(&fixture, "graphql");
        (ran >= 1, ran)
    });
    // The first round asked for news once, so a third ask means that round (which read the PR)
    // and the `304` one after it have both been recorded.
    wait_until("two rounds were recorded", || {
        let asked = gh_ran(&fixture, "notifications");
        (asked >= 3, asked)
    });
    let poll = state_of(&resident)["prPoll"].clone();
    assert!(poll["error"].is_null(), "{poll}");
}

#[test]
fn a_read_that_could_not_be_made_is_the_poll_failing_and_does_not_advance() {
    let fixture = Fixture::new(QUIET);
    let id = card_on_pr(&fixture, POLLED_PR);
    let path = stub_gh_for_polling(&fixture, 1);
    let limited =
        r#"{"data":null,"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded"}]}"#;
    std::fs::write(fixture.repo.join("gh-graphql"), limited).unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    let error = wait_until("the poll reported the failure", || {
        let error = state_of(&resident)["prPoll"]["error"].clone();
        (error.is_string(), error)
    });
    assert!(error.as_str().unwrap().contains("rate limit"), "{error}");
    assert!(turn_of(&resident, &id).is_null());
    // The stamp was not advanced, so the news comes again: no round was sent `If-Modified-Since`.
    let calls = std::fs::read_to_string(fixture.repo.join("gh-calls")).unwrap();
    assert!(!calls.contains("If-Modified-Since"), "{calls}");
}

#[test]
fn a_card_whose_first_read_failed_is_read_again_and_gets_its_state() {
    let fixture = Fixture::new(QUIET);
    let id = card_on_pr(&fixture, POLLED_PR);
    let path = stub_gh_for_polling(&fixture, 1);
    let unavailable = r#"{"data":{"p0":null},"errors":[{"type":"SERVICE_UNAVAILABLE","path":["p0"],"message":"Something went wrong"}]}"#;
    std::fs::write(fixture.repo.join("gh-graphql"), unavailable).unwrap();
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    wait_until("the failed read was reported", || {
        let error = state_of(&resident)["prPoll"]["error"].clone();
        (error.is_string(), error)
    });
    assert!(fixture.json(&["task", "show", "--id", &id])["prStatus"].is_null());
    std::fs::write(
        fixture.repo.join("gh-graphql"),
        graphql_of_pr("OPEN", "APPROVED"),
    )
    .unwrap();
    wait_until("the card got its state", || {
        let shown = fixture.json(&["task", "show", "--id", &id]);
        (shown["prStatus"]["review"] == "approved", shown)
    });
    wait_until("the warning went away", || {
        let error = state_of(&resident)["prPoll"]["error"].clone();
        (error.is_null(), error)
    });
}
