//! The sessions on the resident's board: starting one with no task, linking it to a task, and
//! what the state shows about each.

mod common;

use common::*;

// ── sessions the board starts with no task, and links to one afterwards ──

fn children_of(state: &serde_json::Value, hub: &str) -> u64 {
    state["hubs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["id"] == hub)
        .unwrap_or_else(|| panic!("no hub {hub} in {state}"))["children"]
        .as_u64()
        .unwrap()
}

#[test]
fn the_state_names_the_command_a_hub_runs() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);

    // Unset, it is the built-in line, with its placeholders still in it.
    let runner = state_of(&resident)["hubRunner"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        runner.contains("{name}") && runner.contains("{sessionId}"),
        "{runner}"
    );
    drop(resident);

    // Set, it is what was written, and the `{name}` is left for the page to fill in.
    write_tmux_config_with(&fixture, |c| {
        c["hubRunner"] = serde_json::json!("my-agent {name}");
    });
    let resident = Resident::start(&fixture);
    assert_eq!(state_of(&resident)["hubRunner"], "my-agent {name}");
}

#[test]
fn a_parent_hubs_board_gives_the_sidebar_its_tasks_and_history() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let resident = Resident::start(&fixture);

    let (status, body) = resident.post(
        &format!("/b/{FEATURE_SLUG}/api/tasks"),
        &serde_json::json!({"title": "A child of the feature"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The task is on that hub's board and not on the repository's.
    let listed = |path: &str| -> Vec<String> {
        let (status, body) = resident.get(path);
        assert_eq!(status, 200, "{body}");
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(listed(&format!("/b/{FEATURE_SLUG}/api/state")).contains(&id));
    assert!(!listed(&format!("/b/{SLUG}/api/state")).contains(&id));

    // Its history is read from the same board, which is where the page asks for it.
    let (status, body) = resident.get(&format!("/b/{FEATURE_SLUG}/api/tasks/{id}/history"));
    assert_eq!(status, 200, "{body}");
    let history: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(history["answered"].is_array(), "{history}");
    assert!(history["records"].is_array(), "{history}");
}

#[test]
fn a_queued_task_from_the_board_says_which_inbox_file_it_was_handed_in() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(
        &format!("/b/{SLUG}/api/tasks"),
        &serde_json::json!({"title": "Goes to the hub", "status": "queued"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(answer["handed"]["present"].is_boolean(), "{body}");
    let path = std::path::PathBuf::from(answer["handed"]["path"].as_str().expect(&body));
    assert!(path.is_file(), "{body}");
    assert_eq!(
        path.parent().unwrap(),
        fixture.state.join("inbox").join(SLUG),
        "{body}"
    );
    let pending = fixture.json(&["pending", "--json"]);
    assert_eq!(
        pending["messages"][0]["name"].as_str().unwrap(),
        path.file_name().unwrap().to_str().unwrap(),
        "{pending}"
    );
}

#[test]
fn the_polled_state_carries_no_diffs_and_the_history_still_does() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(
        &format!("/b/{SLUG}/api/tasks"),
        &serde_json::json!({"title": "Has a diff"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let diff = "diff --git a/f b/f\n+one\n";
    let records = fixture.state.join("gates").join(SLUG).join("records");
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(
        records.join("20260922T041233Z-diff-record.json"),
        serde_json::json!({
            "id": "20260922T041233Z-diff-record",
            "kind": "diff",
            "task": id,
            "worktree": fixture.repo.to_str().unwrap(),
            "title": "round one",
            "wait": false,
            "openedAt": "20260922T041233Z",
            "diff": diff,
        })
        .to_string(),
    )
    .unwrap();

    let state = state_of(&resident);
    let task = state["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap();
    let record = &task["records"][0];
    assert_eq!(record["id"], "20260922T041233Z-diff-record", "{task}");
    assert!(record.get("diff").is_none(), "{record}");
    assert_eq!(record["diffSize"], diff.len(), "{record}");

    let (status, body) = resident.get(&format!("/b/{SLUG}/api/tasks/{id}/history"));
    assert_eq!(status, 200, "{body}");
    let history: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(history["records"][0]["diff"], diff, "{history}");
}

#[test]
fn a_session_request_lands_in_the_chosen_hubs_inbox_with_its_instruction() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let instruction = "Try a retry on the upload.\n## Not a header\nkeep 'quotes' and $vars";
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": instruction, "worktreeName": "try-retry"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["worktreeName"], "try-retry");
    assert_eq!(answer["hubStarted"], false, "{body}");
    assert_eq!(answer["handed"]["present"], false, "{body}");

    let pending = fixture.json(&["pending", "--json"]);
    assert_eq!(pending["count"], 1, "{pending}");
    let message = &pending["messages"][0];
    assert_eq!(message["kind"], "session");
    assert_eq!(message["from"], "dashboard");
    assert_eq!(message["subject"], "start a session: try-retry");
    let read = fixture.ok(&["pending", "--read", message["name"].as_str().unwrap()]);
    assert!(read.contains("## Worktree name try-retry"), "{read}");
    assert!(read.contains("## Agent         claude"), "{read}");
    assert!(read.contains(instruction), "{read}");

    // Left out, the name comes from the instruction.
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "Look at the flaky upload test"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains("\"worktreeName\":\"look-at-the-flaky\""),
        "{body}"
    );

    // The reply says which hub took it and under what file name the hub will find it.
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["hub"], "hub", "{body}");
    let inbox = state_of(&resident)["hubs"][0]["inbox"].clone();
    let names: Vec<&str> = inbox
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&answer["message"].as_str().unwrap()),
        "{names:?} {body}"
    );
}

#[test]
fn a_session_request_with_no_instruction_gets_a_dated_name_and_says_so() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(&sessions_url(""), "{}");
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    let name = answer["worktreeName"].as_str().unwrap();
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let rest = name.strip_prefix("session-").expect(name);
    let (day, time) = rest.split_once('-').expect(name);
    assert!(digits(day, 8) && digits(time, 4), "{name}");

    let pending = fixture.json(&["pending", "--json"]);
    let read = fixture.ok(&[
        "pending",
        "--read",
        pending["messages"][0]["name"].as_str().unwrap(),
    ]);
    assert!(read.contains("## Instruction\n-\n"), "{read}");
}

#[test]
fn a_session_started_from_the_board_names_the_agent_its_runner_is_set_to() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    assert_eq!(state_of(&resident)["sessionStart"]["agent"], "claude");
    drop(resident);

    write_tmux_config_with(&fixture, |c| {
        c["agentRunner"] = serde_json::json!("codex exec {prompt}");
    });
    let resident = Resident::start(&fixture);
    assert_eq!(state_of(&resident)["sessionStart"]["agent"], "codex");
}

#[test]
fn a_session_request_for_a_parent_hub_goes_to_that_hubs_inbox() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "Sketch it", "hub": "hub-wid-957"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["hub"], "hub-wid-957", "{body}");
    let parent = fixture.json(&["pending", "--json", "--hub", FEATURE]);
    assert_eq!(parent["count"], 1, "{parent}");
    assert_eq!(parent["messages"][0]["kind"], "session");
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

#[test]
fn a_session_request_with_a_bad_name_another_agent_or_a_non_text_instruction_is_refused() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    for (input, wanted) in [
        (
            serde_json::json!({"instruction": "x", "worktreeName": "bad name"}),
            "worktree name",
        ),
        (
            serde_json::json!({"instruction": "x", "worktreeName": "-flag"}),
            "branch",
        ),
        (serde_json::json!({"instruction": 5}), "instruction"),
        (serde_json::json!({"instruction": " - "}), "no instruction"),
        (
            serde_json::json!({"instruction": "x", "agent": ["claude"]}),
            "agent",
        ),
        (
            serde_json::json!({"instruction": "x", "worktreeName": 7}),
            "worktreeName",
        ),
        (
            serde_json::json!({"instruction": "x", "agent": "codex"}),
            "only claude",
        ),
        (
            serde_json::json!({"instruction": "x", "hub": "hub-nope"}),
            "no such hub",
        ),
    ] {
        let (status, body) = resident.post(&sessions_url(""), &input.to_string());
        assert_eq!(status, 400, "{input}: {body}");
        assert!(body.contains(wanted), "{input}: {body}");
    }
    // The agent the runner starts is fine to name.
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "x", "agent": "claude"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 1);
}

#[test]
fn a_session_request_is_refused_when_no_worker_slot_is_free() {
    let fixture = Fixture::new(
        r#"{"notification": "true", "defaults": {"ide": "code", "maxWorkers": 1},
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget"}}}"#,
    );
    let running = Sleeper::new();
    session_worktree(&fixture, "busy", None, None, running.pid());
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "x"}).to_string(),
    );
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("worker limit"), "{body}");
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

#[test]
fn linking_a_taskless_session_to_a_task_gives_it_a_card() {
    let fixture = Fixture::new(QUIET);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.pid());
    let resident = Resident::start(&fixture);
    let task = made_task(&resident, serde_json::json!({"title": "Retry the upload"}));
    let id = task["id"].as_str().unwrap();

    // Before: the session is on the board with no task.
    let before = state_of(&resident);
    let session = before["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "worker-try-retry")
        .unwrap();
    assert!(session["task"].is_null(), "{session}");
    assert!(session["taskTitle"].is_null(), "{session}");

    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({"task": id}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["task"]["status"], "dispatched", "{body}");
    assert_eq!(
        answer["task"]["worktree"],
        worktree.to_string_lossy().as_ref(),
        "{body}"
    );

    let record = worker_record(&worktree);
    assert_eq!(record["task"], id);
    assert_eq!(record["phase"], "implement");
    assert!(record.get("hub").is_none(), "{record}");
    // What says it is the same worker is untouched.
    assert_eq!(record["pid"], running.pid());
    assert_eq!(saved_session(&worktree)["task"], id);

    let after = state_of(&resident);
    let session = after["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "worker-try-retry")
        .unwrap();
    assert_eq!(session["task"], id, "{session}");
    assert_eq!(session["taskTitle"], "Retry the upload", "{session}");
    let worker = after["workers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "try-retry")
        .unwrap();
    assert_eq!(worker["task"], id, "{worker}");
    let stored = after["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap();
    assert_eq!(stored["status"], "dispatched");

    let outbox =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-outbox.md")).unwrap();
    assert!(outbox.contains(&format!("[linked {id}]")), "{outbox}");
    assert!(outbox.contains("adj skill adj-worker"), "{outbox}");
}

#[test]
fn linking_a_new_task_on_a_parent_hubs_board_moves_the_worker_to_that_hub() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.pid());
    let resident = Resident::start(&fixture);
    assert_eq!(children_of(&state_of(&resident), "hub-wid-957"), 0);

    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({"newTask": {"title": "Child of the feature"}, "hub": "hub-wid-957"})
            .to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = answer["task"]["id"].as_str().unwrap();
    assert_eq!(answer["task"]["status"], "dispatched");

    // Stored in that hub's task directory, and not the repository's.
    let there = fixture
        .state
        .join("tasks")
        .join(FEATURE_SLUG)
        .join(format!("{id}.json"));
    assert!(there.exists(), "{}", there.display());
    assert!(
        !fixture
            .state
            .join("tasks")
            .join(SLUG)
            .join(format!("{id}.json"))
            .exists()
    );
    assert_eq!(worker_record(&worktree)["hub"], FEATURE);
    assert_eq!(worker_record(&worktree)["task"], id);
    assert_eq!(saved_session(&worktree)["hub"], FEATURE);
    let state = state_of(&resident);
    assert_eq!(children_of(&state, "hub-wid-957"), 1);
    let session = state["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "worker-try-retry")
        .unwrap();
    assert_eq!(session["hub"], "hub-wid-957", "{session}");
    // The task is on the parent hub's board, not this one's, and still names the session.
    assert_eq!(session["taskTitle"], "Child of the feature", "{session}");
}

/// The messages of `kind` waiting for the hub at `hub_args` (`[]` for the repository's).
fn messages_of_kind(fixture: &Fixture, hub: &[&str], kind: &str) -> Vec<serde_json::Value> {
    let mut args = vec!["pending", "--json"];
    args.extend_from_slice(hub);
    fixture.json(&args)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] == kind)
        .cloned()
        .collect()
}

/// Makes the fake tmux refuse to open a window, which is how a hub fails to start.
fn tmux_refuses_windows(tmux: &FakeTmux) {
    std::fs::write(format!("{}.failnew", tmux.log.display()), "").unwrap();
}

#[test]
fn a_session_request_starts_a_stopped_hub_after_the_message_is_in_its_inbox() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let resident = resident_with_tmux(&fixture, &tmux, None);
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "Look around"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["hubStarted"], true, "{body}");
    assert!(answer.get("hubStartError").is_none(), "{body}");
    assert!(tmux.logged().contains("new-window"), "{}", tmux.logged());
    assert_eq!(messages_of_kind(&fixture, &[], "session").len(), 1);
}

#[test]
fn a_hub_that_cannot_start_leaves_the_session_request_queued_and_says_why() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    tmux_refuses_windows(&tmux);
    let resident = resident_with_tmux(&fixture, &tmux, None);
    let (status, body) = resident.post(
        &sessions_url(""),
        &serde_json::json!({"instruction": "Look around"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["hubStarted"], false, "{body}");
    assert!(
        answer["hubStartError"]
            .as_str()
            .is_some_and(|e| !e.is_empty()),
        "{body}"
    );
    assert!(answer["message"].is_string(), "{body}");
    assert_eq!(messages_of_kind(&fixture, &[], "session").len(), 1);
}

#[test]
fn a_file_issue_request_starts_a_stopped_target_hub_and_reports_when_it_cannot() {
    for refuse in [false, true] {
        let fixture = Fixture::new(QUIET);
        write_tmux_config(&fixture);
        listed_parent_hub(&fixture);
        let tmux = FakeTmux::new(&fixture);
        if refuse {
            tmux_refuses_windows(&tmux);
        }
        let running = Sleeper::new();
        let worktree = session_worktree(&fixture, "try-retry", None, None, running.pid());
        let resident = resident_with_tmux(&fixture, &tmux, None);
        let (status, body) = resident.post(
            &sessions_url("/worker-try-retry/link"),
            &serde_json::json!({
                "newTask": {"title": "Needs an issue", "kind": "file-and-start"},
                "hub": "hub-wid-957",
            })
            .to_string(),
        );
        assert_eq!(status, 200, "{body}");
        let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
        // The link stands either way; only what the hub was told differs.
        assert_eq!(worker_record(&worktree)["hub"], FEATURE);
        assert_eq!(
            messages_of_kind(&fixture, &["--hub", FEATURE], "file-issue").len(),
            1
        );
        assert_eq!(answer["fileIssue"]["handed"]["present"], false, "{body}");
        if refuse {
            assert_eq!(answer["hubStarted"], false, "{body}");
            assert!(answer["hubStartError"].is_string(), "{body}");
        } else {
            assert_eq!(answer["hubStarted"], true, "{body}");
            assert!(answer.get("hubStartError").is_none(), "{body}");
        }
    }
}

#[test]
fn linking_a_new_task_that_needs_an_issue_asks_the_hub_to_file_it() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.pid());
    let plain = session_worktree(&fixture, "plain", None, None, running.pid());
    let resident = Resident::start(&fixture);

    // Without the ask, the hub hears nothing.
    let (status, body) = resident.post(
        &sessions_url("/worker-plain/link"),
        &serde_json::json!({"newTask": {"title": "No issue"}, "hub": "hub-wid-957"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    assert!(messages_of_kind(&fixture, &["--hub", FEATURE], "file-issue").is_empty());
    assert!(
        serde_json::from_str::<serde_json::Value>(&body)
            .unwrap()
            .get("fileIssue")
            .is_none()
    );
    let outbox = std::fs::read_to_string(plain.join(".claude").join("adjutant-outbox.md")).unwrap();
    assert!(outbox.contains("none will be filed"), "{outbox}");

    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({
            "newTask": {"title": "Needs an issue", "kind": "file-and-start"},
            "hub": "hub-wid-957",
        })
        .to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = answer["task"]["id"].as_str().unwrap();
    assert_eq!(answer["fileIssue"]["handed"]["present"], false, "{body}");
    assert!(answer.get("fileIssueError").is_none(), "{body}");

    let found = messages_of_kind(&fixture, &["--hub", FEATURE], "file-issue");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["from"], "dashboard");
    assert_eq!(found[0]["subject"], format!("[file {id}] Needs an issue"));
    let read = fixture.ok(&[
        "pending",
        "--hub",
        FEATURE,
        "--read",
        found[0]["name"].as_str().unwrap(),
    ]);
    assert!(read.contains(id), "{read}");
    assert!(read.contains("file and start"), "{read}");
    assert!(
        read.contains(&format!("## Worker running in {}", worktree.display())),
        "{read}"
    );
    // Nothing went to the repository's hub.
    assert!(messages_of_kind(&fixture, &[], "file-issue").is_empty());

    let outbox =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-outbox.md")).unwrap();
    assert!(outbox.contains("files the issue"), "{outbox}");
}

#[test]
fn a_hub_that_cannot_be_told_to_file_an_issue_leaves_the_link_and_tells_the_worker() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.pid());
    // A file where the hub's inbox directory would be: nothing can be delivered there.
    let inbox = fixture.state.join("inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(inbox.join(FEATURE_SLUG), "").unwrap();
    let resident = Resident::start(&fixture);
    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({
            "newTask": {"title": "Needs an issue", "kind": "file-and-start"},
            "hub": "hub-wid-957",
        })
        .to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(answer["fileIssueError"].is_string(), "{body}");
    assert!(answer.get("fileIssue").is_none(), "{body}");
    assert_eq!(worker_record(&worktree)["hub"], FEATURE);
    let outbox =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-outbox.md")).unwrap();
    assert!(outbox.contains("not filed"), "{outbox}");
}

#[test]
fn linking_an_existing_file_and_start_task_files_nothing_again() {
    let fixture = Fixture::new(QUIET);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.pid());
    let resident = Resident::start(&fixture);
    let task = made_task(
        &resident,
        serde_json::json!({"title": "Filed already", "kind": "file-and-start", "status": "backlog"}),
    );
    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({"task": task["id"]}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(answer.get("fileIssue").is_none(), "{body}");
    assert!(messages_of_kind(&fixture, &[], "file-issue").is_empty());
    let outbox =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-outbox.md")).unwrap();
    assert!(!outbox.contains("files the issue"), "{outbox}");
}

#[test]
fn linking_an_existing_task_is_refused_when_asked_to_file_an_issue_for_it() {
    let fixture = Fixture::new(QUIET);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.pid());
    let resident = Resident::start(&fixture);
    let task = made_task(&resident, serde_json::json!({"title": "Already there"}));
    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({"task": task["id"], "kind": "file-and-start"}).to_string(),
    );
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("file-and-start"), "{body}");
    assert!(worker_record(&worktree).get("task").is_none());
}

#[test]
fn linking_to_a_repository_task_moves_a_parent_hub_worker_back_so_its_hub_can_close() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "try-retry", Some(FEATURE), None, running.pid());
    let resident = Resident::start(&fixture);
    assert_eq!(children_of(&state_of(&resident), "hub-wid-957"), 1);
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub-wid-957/close"), "{}");
    assert_eq!(status, 400, "{body}");

    let task = made_task(&resident, serde_json::json!({"title": "Back home"}));
    let id = task["id"].as_str().unwrap();
    let (status, body) = resident.post(
        &sessions_url("/worker-try-retry/link"),
        &serde_json::json!({"task": id}).to_string(),
    );
    assert_eq!(status, 200, "{body}");

    let record = worker_record(&worktree);
    assert!(record.get("hub").is_none(), "{record}");
    assert_eq!(record["task"], id);
    assert!(saved_session(&worktree).get("hub").is_none());
    assert_eq!(children_of(&state_of(&resident), "hub-wid-957"), 0);
    // Nothing reports to it any more, so the board can close it.
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/hubs/hub-wid-957/close"), "{}");
    assert_eq!(status, 200, "{body}");
}

#[test]
fn a_link_is_refused_for_a_hub_a_session_not_started_a_finished_task_or_one_held_by_another_worker()
{
    let fixture = Fixture::new(QUIET);
    let running = Sleeper::new();
    let worktree = session_worktree(&fixture, "try-retry", None, None, running.pid());
    let other = session_worktree(&fixture, "other", None, None, running.pid());
    // A worktree the board lists that no worker ever registered in.
    let unstarted = fixture.repo.parent().unwrap().join("unstarted");
    let out = Command::new("git")
        .hermetic()
        .args(["worktree", "add", "-q", "-b", "unstarted"])
        .arg(&unstarted)
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    let resident = Resident::start(&fixture);
    let task = made_task(&resident, serde_json::json!({"title": "Open one"}));
    let id = task["id"].as_str().unwrap().to_string();
    let link = |session: &str, input: serde_json::Value| {
        let (status, body) = resident.post(
            &sessions_url(&format!("/{session}/link")),
            &input.to_string(),
        );
        assert_eq!(status, 400, "{input}: {body}");
        body
    };

    assert!(
        link(
            "worker-try-retry",
            serde_json::json!({"task": id, "hub": "hub-nope"})
        )
        .contains("no such hub")
    );
    assert!(link("worker-nope", serde_json::json!({"task": id})).contains("no such session"));
    assert!(link("hub", serde_json::json!({"task": id})).contains("worker session"));
    assert!(
        link("worker-unstarted", serde_json::json!({"task": id})).contains("has not started yet")
    );
    assert!(link("worker-try-retry", serde_json::json!({})).contains("required"));
    assert!(
        link(
            "worker-try-retry",
            serde_json::json!({"task": "no-such-task"})
        )
        .contains("no-such-task")
    );

    // Finished.
    let (status, body) = resident.post(
        &format!("/b/{SLUG}/api/tasks/{id}"),
        &serde_json::json!({"status": "done"}).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    assert!(link("worker-try-retry", serde_json::json!({"task": id})).contains("finished"));

    // Held by a worker that is running in another worktree.
    let held = made_task(&resident, serde_json::json!({"title": "Held"}));
    let held_id = held["id"].as_str().unwrap();
    let (status, body) = resident.post(
        &format!("/b/{SLUG}/api/tasks/{held_id}"),
        &serde_json::json!({"status": "dispatched", "worktree": other.to_string_lossy()})
            .to_string(),
    );
    assert_eq!(status, 200, "{body}");
    assert!(
        link("worker-try-retry", serde_json::json!({"task": held_id}))
            .contains("already has a worker")
    );

    // For Jules.
    let jules = made_task(
        &resident,
        serde_json::json!({"title": "For Jules", "executor": "jules"}),
    );
    assert!(
        link(
            "worker-try-retry",
            serde_json::json!({"task": jules["id"].as_str().unwrap()})
        )
        .contains("Jules")
    );

    // A newTask for Jules is refused as an existing one is, and leaves no record behind.
    let tasks_dir = fixture.state.join("tasks").join(SLUG);
    let count = || std::fs::read_dir(&tasks_dir).unwrap().count();
    let before = count();
    assert!(
        link(
            "worker-try-retry",
            serde_json::json!({"newTask": {"title": "Jules too", "executor": "jules"}})
        )
        .contains("Jules")
    );
    assert_eq!(count(), before);

    // An id that is not a plain file name never reaches the filesystem.
    for bad in ["../../x", "a/b", "..", "a\\b"] {
        assert!(
            link("worker-try-retry", serde_json::json!({"task": bad})).contains("no such task")
        );
    }
    assert!(!fixture.state.join("x.json").exists());
    assert!(!fixture.state.join("x.lock").exists());

    // A session whose worker has ended stays listed, and cannot be linked.
    let mut finished = Command::new("true").spawn().unwrap();
    finished.wait().unwrap();
    session_worktree(&fixture, "gone", None, None, finished.id());
    assert!(link("worker-gone", serde_json::json!({"task": id.clone()})).contains("has ended"));

    // A session that already has a task keeps it.
    std::fs::write(
        worktree.join(".claude").join("adjutant-worker.json"),
        serde_json::json!({"pid": running.pid(), "psStarted": ps_started(running.pid()), "task": "task-x"})
            .to_string(),
    )
    .unwrap();
    let free = made_task(&resident, serde_json::json!({"title": "Free"}));
    assert!(
        link(
            "worker-try-retry",
            serde_json::json!({"task": free["id"].as_str().unwrap()})
        )
        .contains("already has a task")
    );

    // Nothing was written by any of the refusals.
    let stored = state_of(&resident);
    let free_now = stored["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == free["id"])
        .unwrap();
    assert_eq!(free_now["status"], "backlog");
    assert!(free_now["worktree"].is_null(), "{free_now}");
}

// ── what the board tells sessions apart by ───────────────────────────

#[test]
fn a_session_says_when_its_window_was_last_active_and_how_many_are_attached() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    let running = Sleeper::new();
    let one = session_worktree(&fixture, "one", None, None, running.pid());
    let two = session_worktree(&fixture, "two", None, None, running.pid());
    let elsewhere = session_worktree(&fixture, "elsewhere", None, None, running.pid());
    place_worker(&one, "@5");
    place_worker(&two, "@6");
    place_worker(&elsewhere, "@9");
    std::fs::write(
        &tmux.panes,
        "%5\t1\t/dev/ttys005\t@5\tadjutant-test\t1\tone\t1790000000\n\
         %5\t1\t/dev/ttys005\t@5\tadjboard-1-1\t1\tone\t1790000000\n\
         %6\t2\t/dev/ttys006\t@6\tadjutant-test\t2\ttwo\t1790000100\n",
    )
    .unwrap();
    // The board's own session, a client looking at @5, and iTerm2's control client, which
    // has every window of its session open.
    std::fs::write(
        &tmux.clients,
        "adjboard-1-1\t0\t@5\nadjutant-test\t0\t@5\nadjutant-test\t1\t@6\n",
    )
    .unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, None);

    let state = state_of(&resident);
    let one = session_of(&state, "worker-one");
    assert_eq!(one["lastActivityAt"], 1_790_000_000, "{one}");
    assert_eq!(one["attached"], 2, "{one}");
    let two = session_of(&state, "worker-two");
    assert_eq!(two["lastActivityAt"], 1_790_000_100, "{two}");
    assert_eq!(two["attached"], 1, "{two}");
    // A window tmux does not list says nothing rather than zero.
    let gone = session_of(&state, "worker-elsewhere");
    assert!(gone.get("lastActivityAt").is_none(), "{gone}");
    assert!(gone.get("attached").is_none(), "{gone}");

    // A tmux with nothing to say about clients leaves the rest of the poll as it was.
    std::fs::write(&tmux.clients, "").unwrap();
    let quiet = state_of(&resident);
    assert_eq!(session_of(&quiet, "worker-one")["attached"], 0);
    assert_eq!(
        session_of(&quiet, "worker-one")["lastActivityAt"],
        1_790_000_000
    );
}

#[test]
fn a_session_says_what_its_pane_last_showed_only_when_asked() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    std::fs::write(
        &tmux.screen,
        include_str!("../src/fixtures/panes/claude-idle-after-turn.txt"),
    )
    .unwrap();
    let running = Sleeper::new();
    let one = session_worktree(&fixture, "one", None, None, running.pid());
    place_worker(&one, "@5");
    let panes = |activity: i64| {
        std::fs::write(
            &tmux.panes,
            format!("%5\t1\t/dev/ttys005\t@5\tadjutant-test\t1\tone\t{activity}\n"),
        )
        .unwrap();
    };
    panes(1_790_000_000);
    let resident = resident_with_tmux(&fixture, &tmux, None);
    let captures = || tmux.logged().matches("capture-pane").count();
    let state_at = |query: &str| -> serde_json::Value {
        let (status, body) = get_with_query(&resident, &format!("/b/{SLUG}/api/state"), query);
        assert_eq!(status, 200, "{body}");
        serde_json::from_str(&body).unwrap()
    };

    // The ordinary poll reads no screen and says nothing of one.
    let plain = state_of(&resident);
    assert!(session_of(&plain, "worker-one").get("lastLine").is_none());
    assert_eq!(captures(), 0, "{}", tmux.logged());
    // Nor does a poll that leaves the sessions out.
    let lean = state_at("sessions=0&lines=1");
    assert_eq!(lean["sessions"], serde_json::json!([]));
    assert_eq!(captures(), 0, "{}", tmux.logged());

    // Asked for, the line above the input box.
    let asked = state_at("lines=1");
    assert_eq!(
        session_of(&asked, "worker-one")["lastLine"],
        "✻ Brewed for 3s · done 2:20",
        "{asked}"
    );
    assert_eq!(captures(), 1);
    // A window that has not moved is not read again.
    state_at("lines=1");
    assert_eq!(captures(), 1, "{}", tmux.logged());
    // How soon a window that has moved is read again depends on the clock, so that is left to
    // the unit test of `LastLines`.
}

#[test]
fn a_poll_for_the_hubs_lines_reads_the_hubs_pane_and_no_worker_s() {
    let fixture = Fixture::new(QUIET);
    write_tmux_config(&fixture);
    let tmux = FakeTmux::new(&fixture);
    std::fs::write(
        &tmux.screen,
        include_str!("../src/fixtures/panes/claude-idle-after-turn.txt"),
    )
    .unwrap();
    let sleeper = Sleeper::new();
    let running = Sleeper::new();
    let one = session_worktree(&fixture, "one", None, None, running.pid());
    place_worker(&one, "@5");
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": sleeper.pid(),
            "psStarted": ps_started(sleeper.pid()),
            "hubName": HUB,
            "cwd": fixture.repo.to_str().unwrap(),
            "nameInCommand": false,
            "terminal": {"backend": "tmux", "socket": "scratch", "window": "@1"},
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &tmux.panes,
        format!(
            "%3\t{}\t/dev/ttys999\t@1\tadjutant-test\t1\tmain\t1790000000\n\
             %5\t1\t/dev/ttys005\t@5\tadjutant-test\t1\tone\t1790000000\n",
            sleeper.pid()
        ),
    )
    .unwrap();
    let resident = resident_with_tmux(&fixture, &tmux, None);
    let (status, body) = get_with_query(&resident, &format!("/b/{SLUG}/api/state"), "lines=hub");
    assert_eq!(status, 200, "{body}");
    let asked: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(
        session_of(&asked, "hub")["lastLine"],
        "✻ Brewed for 3s · done 2:20",
        "{asked}"
    );
    assert!(session_of(&asked, "worker-one").get("lastLine").is_none());
    assert_eq!(tmux.logged().matches("capture-pane").count(), 1);
}

#[test]
fn a_session_shows_the_gate_it_waits_on_even_from_a_parent_hubs_directory() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let parent = session_worktree(&fixture, "under-parent", Some(FEATURE), None, 1);
    let moved = session_worktree(&fixture, "moved-on", Some(FEATURE), None, 1);
    let own = session_worktree(&fixture, "own-hub", None, None, 1);
    write_gate_file(&fixture, FEATURE_SLUG, "g-parent", "question", &parent);
    write_gate_file(&fixture, FEATURE_SLUG, "g-moved", "question", &moved);
    write_gate_file(&fixture, SLUG, "g-own", "question", &own);
    write_gate_file(&fixture, SLUG, "g-dispatch", "dispatch", &fixture.repo);
    let choosing = session_worktree(&fixture, "choosing", Some(FEATURE), None, 1);
    write_gate_file_with(
        &fixture,
        FEATURE_SLUG,
        "g-choose",
        "plan",
        &choosing,
        serde_json::json!({
            "focus": "あ".repeat(500),
            "choices": [
                {"id": "a", "label": "A 案", "recommended": true},
                {"id": "b", "label": "B 案"},
            ],
        }),
    );
    // This one's worker went on to another phase after opening the gate.
    let mut record = worker_record(&moved);
    record["phaseAt"] = serde_json::json!(LATER_SECS);
    std::fs::write(
        moved.join(".claude").join("adjutant-worker.json"),
        record.to_string(),
    )
    .unwrap();
    let resident = Resident::start(&fixture);

    let state = state_of(&resident);
    let waiting = &session_of(&state, "worker-under-parent")["waiting"];
    assert_eq!(waiting["id"], "g-parent", "{waiting}");
    assert_eq!(waiting["kind"], "question");
    assert_eq!(waiting["hub"], format!("hub-{FEATURE}"));
    assert_eq!(waiting["slug"], FEATURE_SLUG);
    assert_eq!(waiting["openedAt"], "20260922T041233Z");
    assert_eq!(waiting["count"], 1);
    // A gate that names no options gets its kind's own, so the page needs no table of them.
    assert_eq!(waiting["options"], serde_json::json!(["answer"]));
    assert!(waiting.get("choices").is_none() && waiting.get("focus").is_none());
    let choosing = &session_of(&state, "worker-choosing")["waiting"];
    assert_eq!(choosing["choices"][1]["label"], "B 案", "{choosing}");
    assert_eq!(choosing["options"][0], "approve");
    let focus = choosing["focus"].as_str().unwrap();
    assert_eq!(focus.chars().count(), 401, "{focus}");
    assert!(focus.ends_with('…'));
    assert_eq!(
        session_of(&state, "worker-own-hub")["waiting"]["hub"],
        "hub"
    );
    assert!(
        session_of(&state, "worker-moved-on")
            .get("waiting")
            .is_none()
    );
    // What the repository hub opened for a person is what the hub is waiting on.
    assert_eq!(session_of(&state, "hub")["waiting"]["id"], "g-dispatch");
}

#[test]
fn a_hub_lists_its_inbox_newest_first_with_a_cap_and_the_full_count() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let inbox = fixture.state.join("inbox").join(SLUG);
    std::fs::create_dir_all(&inbox).unwrap();
    for i in 0..23 {
        let at = format!("20260922T04{i:02}00Z");
        std::fs::write(
            inbox.join(format!("{at}-report.md")),
            format!("---\nfrom: w{i}\nkind: report\nsubject: s{i}\nat: {at}\n---\n\nbody\n"),
        )
        .unwrap();
    }
    let state = state_of(&resident);
    let hub = &state["hubs"][0];
    assert_eq!(hub["inboxCount"], 23);
    // The count and the age are of the whole inbox, not of the 20 listed.
    assert_eq!(hub["unseen"], 23);
    assert_eq!(hub["seen"], 0);
    assert_eq!(hub["oldestUnseenAt"], "20260922T040000Z");
    let items = hub["inbox"].as_array().unwrap();
    assert_eq!(items.len(), 20);
    assert_eq!(items[0]["subject"], "s22");
    assert_eq!(items[0]["from"], "w22");
    assert_eq!(items[0]["kind"], "report");
    assert_eq!(items[0]["at"], "20260922T042200Z");
    assert_eq!(items[19]["subject"], "s3");
}

#[test]
fn a_session_keeps_every_phase_its_worker_said() {
    let fixture = Fixture::new(QUIET);
    let resident = Resident::start(&fixture);
    let worktree = fixture.repo.clone();
    write_worker(&worktree, "20260922T040000Z", None);
    // Written the way a worker running a version before the history was kept left it.
    let mut record = worker_record(&worktree);
    record["phaseAt"] = serde_json::json!(1_790_000_000);
    std::fs::write(
        worktree.join(".claude").join("adjutant-worker.json"),
        record.to_string(),
    )
    .unwrap();
    for phase in ["verify", "pr"] {
        let out = fixture
            .command(["phase", "--set", phase])
            .current_dir(&worktree)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let state = state_of(&resident);
    let phases = session_of(&state, "worker-main")["phases"]
        .as_array()
        .unwrap()
        .clone();
    let names: Vec<&str> = phases.iter().map(|p| p[0].as_str().unwrap()).collect();
    assert_eq!(names, ["implement", "verify", "pr"], "{phases:?}");
    assert_eq!(phases[0][1], 1_790_000_000);
    assert_eq!(session_of(&state, "worker-main")["phase"], "pr");
}

#[test]
fn the_git_route_reports_a_dirty_worktree_and_refuses_a_session_it_does_not_know() {
    let fixture = Fixture::new(QUIET);
    let worktree = session_worktree(&fixture, "dirty", None, None, 1);
    std::fs::write(worktree.join("scratch.txt"), "untracked\n").unwrap();
    std::fs::write(worktree.join("kept.txt"), "one\ntwo\n").unwrap();
    for args in [
        vec!["add", "kept.txt"],
        vec!["commit", "-q", "-m", "keep two lines"],
    ] {
        let out = Command::new("git")
            .hermetic()
            .args(&args)
            .current_dir(&worktree)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::write(worktree.join("kept.txt"), "one\nTWO\nthree\n").unwrap();
    let resident = Resident::start(&fixture);

    let (status, body) = resident.get(&sessions_url("/worker-dirty/git"));
    assert_eq!(status, 200, "{body}");
    let git: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(git["branch"], "dirty", "{body}");
    assert_eq!(git["uncommitted"]["files"], 1, "{body}");
    // The worker's own records in `.claude` are not work.
    assert_eq!(git["uncommitted"]["untracked"], 1, "{body}");
    assert_eq!(git["uncommitted"]["insertions"], 2, "{body}");
    assert_eq!(git["uncommitted"]["deletions"], 1, "{body}");
    // No remote here, so both commits are ones nobody else has.
    assert_eq!(git["unpushed"]["count"], 2, "{body}");
    assert_eq!(git["unpushed"]["commits"][0]["subject"], "keep two lines");

    let (status, body) = resident.get(&sessions_url("/worker-nobody/git"));
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("no such session"), "{body}");
}

#[test]
fn the_git_route_looks_at_its_own_worktree_and_no_other() {
    let fixture = Fixture::new(QUIET);
    let mine = Sleeper::new();
    session_worktree(&fixture, "spy-target", None, Some("WID-7"), mine.pid());
    session_worktree(&fixture, "spy-other-a", None, None, 1);
    session_worktree(&fixture, "spy-other-b", None, None, 2);
    let spy = Spy::new(fixture._dir.path());
    let resident = Resident::start_with(&fixture, &[("PATH", &spy.path())]);
    // The first request to a board has the server find its checkout, which lists the
    // worktrees once; what is counted below is the route itself.
    resident.get(&sessions_url("/worker-nobody-at-all/nothing"));
    spy.clear();

    let (status, body) = resident.get(&sessions_url("/worker-spy-target/git"));
    assert_eq!(status, 200, "{body}");

    let calls = spy.calls();
    assert!(
        calls.iter().any(|c| c.contains("spy-target")),
        "the target was never looked at: {calls:?}"
    );
    // The branch is read from the worktree listing, so no `git branch` is run for it.
    let branches = calls.iter().filter(|c| c.contains("branch --show-current"));
    assert_eq!(branches.count(), 0, "{calls:?}");
    assert!(
        !calls.iter().any(|c| c.contains("spy-other")),
        "another worktree was asked about: {calls:?}"
    );
    // Once, for the session and for the hub of its task together.
    let listings = calls.iter().filter(|c| c.contains("worktree list"));
    assert_eq!(listings.count(), 1, "{calls:?}");
    // One `ps`, for the worker the session names: the others' pids are not asked about.
    let asked: Vec<&String> = calls.iter().filter(|c| c.starts_with("ps ")).collect();
    assert!(!asked.is_empty(), "{calls:?}");
    assert!(
        asked
            .iter()
            .all(|c| c.ends_with(&format!("-p {}", mine.pid()))),
        "{calls:?}"
    );
}

#[test]
fn a_record_written_after_a_gate_shows_its_worker_moved_on_from_a_parent_hubs_board() {
    let fixture = Fixture::new(QUIET);
    listed_parent_hub(&fixture);
    let recorder = session_worktree(&fixture, "recorder", Some(FEATURE), None, 1);
    let answered = session_worktree(&fixture, "answered", Some(FEATURE), None, 1);
    write_gate_file(&fixture, FEATURE_SLUG, "g-rec", "question", &recorder);
    write_gate_file(&fixture, FEATURE_SLUG, "g-ans", "question", &answered);
    let later = |dir: &str, id: &str, wait: bool, worktree: &Path| {
        let dir = fixture.state.join("gates").join(FEATURE_SLUG).join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::json!({
                "id": id,
                "kind": "verify",
                "worktree": worktree.to_str().unwrap(),
                "title": "later",
                "wait": wait,
                "openedAt": "20260922T042000Z",
            })
            .to_string(),
        )
        .unwrap();
    };
    // A record the worker wrote without stopping, and a later gate that was answered since.
    later("records", "r-1", false, &recorder);
    later("answered", "g-next", true, &answered);
    let resident = Resident::start(&fixture);

    let state = state_of(&resident);
    assert!(
        session_of(&state, "worker-recorder")
            .get("waiting")
            .is_none()
    );
    assert!(
        session_of(&state, "worker-answered")
            .get("waiting")
            .is_none()
    );
}
