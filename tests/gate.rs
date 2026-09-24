//! Gates: a worker held on the board until a decision comes back.

mod common;

use common::*;

#[test]
fn a_gate_can_be_closed_without_delivering_to_the_worker() {
    let fixture = Fixture::new(QUIET);
    let payload = serde_json::json!({
        "kind": "plan",
        "title": "設計方針の確認",
        "worktree": fixture.repo.to_str().unwrap(),
    });
    let payload_file = fixture.repo.join("gate.json");
    std::fs::write(&payload_file, payload.to_string()).unwrap();

    let opened = fixture.json(&[
        "gate",
        "open",
        "--file",
        payload_file.to_str().unwrap(),
        "--json",
    ]);
    let gate_id = opened["gate"]["id"].as_str().unwrap();

    let list = fixture.json(&["gate", "list", "--json"]);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], gate_id);

    // Close the gate directly (e.g. human responded in tab)
    let closed = fixture.json(&[
        "gate",
        "close",
        "--id",
        gate_id,
        "--comment",
        "dealt with in tab",
        "--json",
    ]);
    assert_eq!(closed["gate"]["id"], gate_id);
    assert_eq!(closed["gate"]["decision"], "closed");
    assert_eq!(closed["gate"]["comment"], "dealt with in tab");
    assert!(closed["closed"].as_bool().unwrap());

    // The gate is no longer open
    let list_after = fixture.json(&["gate", "list", "--json"]);
    assert!(list_after.as_array().unwrap().is_empty());

    // And nothing was written to outbox (no worker message delivered)
    let outbox = fixture.ok(&["outbox", "--worktree", fixture.repo.to_str().unwrap()]);
    assert_eq!(outbox.trim(), "(empty)");
}

#[test]
fn an_answer_to_a_gate_the_hub_opened_goes_to_the_hubs_inbox() {
    // The hub reads its inbox and never an outbox. Delivered to the outbox of the checkout it
    // sits in, the answer to "shall I start this?" would wait there for ever.
    let fixture = Fixture::new(QUIET);
    let payload_file = fixture.repo.join("gate.json");
    std::fs::write(
        &payload_file,
        serde_json::json!({
            "kind": "dispatch",
            "task": "t-1",
            "title": "着手確認: WID-7",
            "worktree": fixture.repo.to_str().unwrap(),
        })
        .to_string(),
    )
    .unwrap();
    let opened = fixture.json(&[
        "gate",
        "open",
        "--file",
        payload_file.to_str().unwrap(),
        "--json",
    ]);
    let id = opened["gate"]["id"].as_str().unwrap().to_string();

    fixture.ok(&["gate", "answer", "--id", &id, "--decision", "approve"]);

    let pending = fixture.json(&["pending", "--json"]);
    assert_eq!(pending["count"], 1, "{pending}");
    let message = &pending["messages"][0];
    assert_eq!(message["kind"], "gate", "{pending}");
    assert!(
        message["subject"]
            .as_str()
            .unwrap()
            .starts_with(&format!("[gate {id}] approve")),
        "{pending}"
    );
    // The gate is archived by now, so the message is the only place the task can be read.
    let name = message["name"].as_str().unwrap();
    let body = fixture.ok(&["pending", "--read", name]);
    assert!(body.contains("## task       t-1"), "{body}");
    let outbox = fixture.ok(&["outbox", "--worktree", fixture.repo.to_str().unwrap()]);
    assert_eq!(outbox.trim(), "(empty)");
}

#[test]
fn a_dispatch_gate_without_its_task_is_refused() {
    // Its answer is acted on by reading the task out of the message; without one the hub is
    // told "approve" and not what.
    let fixture = Fixture::new(QUIET);
    let payload_file = fixture.repo.join("gate.json");
    std::fs::write(
        &payload_file,
        serde_json::json!({
            "kind": "dispatch",
            "title": "着手確認",
            "worktree": fixture.repo.to_str().unwrap(),
        })
        .to_string(),
    )
    .unwrap();
    let out = fixture.cmd(&["gate", "open", "--file", payload_file.to_str().unwrap()]);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("needs the task"),
        "{out:?}"
    );
    assert!(
        fixture
            .json(&["gate", "list", "--json"])
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn answering_a_gate_writes_the_time_onto_its_task() {
    // The board counts a worker's stuck time from here, so time spent waiting on a person
    // does not turn the card red the moment they answer.
    let fixture = Fixture::new(QUIET);
    let added = fixture.json(&["task", "add", "--body", "WID-8", "--json"]);
    let task = added["task"]["id"].as_str().unwrap().to_string();
    assert!(added["task"]["gateAnsweredAt"].is_null(), "{added}");

    let payload_file = fixture.repo.join("gate.json");
    std::fs::write(
        &payload_file,
        serde_json::json!({
            "kind": "plan",
            "task": task,
            "title": "設計方針の確認",
            "worktree": fixture.repo.to_str().unwrap(),
        })
        .to_string(),
    )
    .unwrap();
    let opened = fixture.json(&[
        "gate",
        "open",
        "--file",
        payload_file.to_str().unwrap(),
        "--json",
    ]);
    let id = opened["gate"]["id"].as_str().unwrap().to_string();
    let answered = fixture.json(&[
        "gate",
        "answer",
        "--id",
        &id,
        "--decision",
        "approve",
        "--json",
    ]);

    let shown = fixture.json(&["task", "show", "--id", &task]);
    assert_eq!(
        shown["gateAnsweredAt"], answered["gate"]["answeredAt"],
        "{shown}"
    );
}
