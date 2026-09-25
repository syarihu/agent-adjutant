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

#[test]
fn a_record_is_kept_without_waiting_and_an_answer_to_it_reaches_the_outbox() {
    let fixture = Fixture::new(QUIET);
    let worktree = fixture.repo.to_str().unwrap();
    let payload_file = fixture.repo.join("gate.json");
    std::fs::write(
        &payload_file,
        serde_json::json!({
            "kind": "diff",
            "wait": false,
            "title": "セルフレビュー: 2ラウンドで収束",
            "worktree": worktree,
            "reviewRounds": [
                { "engine": "claude", "must": 1, "falsePositives": 1 },
                { "engine": "claude" },
            ],
            "findings": [{
                "severity": "must",
                "location": "src/gate.rs:10",
                "text": "unwrap on a missing file",
                "outcome": "fixed",
            }],
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
    assert_eq!(opened["wait"], false, "{opened}");
    assert!(
        opened["note"].as_str().unwrap().contains("Do not wait"),
        "{opened}"
    );
    let id = opened["gate"]["id"].as_str().unwrap().to_string();
    assert!(id.ends_with("-diff-record"), "{id}");

    // Stored, and asks nobody for anything.
    assert!(
        fixture
            .json(&["gate", "list", "--json"])
            .as_array()
            .unwrap()
            .is_empty()
    );
    let shown = fixture.json(&["gate", "show", "--id", &id]);
    assert_eq!(shown["reviewRounds"][0]["falsePositives"], 1, "{shown}");
    assert_eq!(shown["findings"][0]["outcome"], "fixed", "{shown}");

    // Only sending it back means anything to a worker that is not waiting.
    let refused = fixture.cmd(&["gate", "answer", "--id", &id, "--decision", "approve"]);
    assert!(!refused.status.success(), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("only be answered with changes"),
        "{refused:?}"
    );
    let refused = fixture.cmd(&[
        "gate",
        "answer",
        "--id",
        &id,
        "--decision",
        "changes",
        "--choice",
        "a",
    ]);
    assert!(!refused.status.success(), "{refused:?}");

    fixture.ok(&[
        "gate",
        "answer",
        "--id",
        &id,
        "--decision",
        "changes",
        "--comment",
        "テストを足してほしいのだ",
    ]);
    let outbox = fixture.ok(&["outbox", "--worktree", worktree]);
    assert!(outbox.contains(&format!("[gate {id}] changes")), "{outbox}");
    assert!(outbox.contains("(diff, 記録)"), "{outbox}");
    assert!(outbox.contains("テストを足してほしいのだ"), "{outbox}");

    // And the record stays, with the answer on it.
    let shown = fixture.json(&["gate", "show", "--id", &id]);
    assert_eq!(shown["answers"][0]["decision"], "changes", "{shown}");
    assert_eq!(
        shown["answers"][0]["comment"], "テストを足してほしいのだ",
        "{shown}"
    );
    assert!(shown["decision"].is_null(), "{shown}");
}

#[test]
fn a_plan_cannot_be_kept_as_a_record() {
    // A plan always waits: nothing is written before a person has seen it.
    let fixture = Fixture::new(QUIET);
    let payload_file = fixture.repo.join("gate.json");
    std::fs::write(
        &payload_file,
        serde_json::json!({
            "kind": "plan",
            "wait": false,
            "title": "設計方針の確認",
            "worktree": fixture.repo.to_str().unwrap(),
        })
        .to_string(),
    )
    .unwrap();
    let out = fixture.cmd(&["gate", "open", "--file", payload_file.to_str().unwrap()]);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot be kept as a record"),
        "{out:?}"
    );
}

#[test]
fn only_a_diff_or_verify_gate_that_waits_says_what_stopped_it() {
    // The board shows why a worker stopped. A plan always stops, and a record did not stop
    // anything, so a reason on either is a payload contradicting itself.
    let fixture = Fixture::new(QUIET);
    let open = |payload: serde_json::Value| {
        let file = fixture.repo.join("gate.json");
        let mut payload = payload;
        payload["worktree"] = serde_json::json!(fixture.repo.to_str().unwrap());
        std::fs::write(&file, payload.to_string()).unwrap();
        fixture.cmd(&["gate", "open", "--file", file.to_str().unwrap(), "--json"])
    };

    let out = open(serde_json::json!({
        "kind": "verify",
        "title": "動作確認",
        "stoppedBy": ["manual-check", "stop-at"],
    }));
    assert!(out.status.success(), "{out:?}");
    let opened: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        opened["gate"]["stoppedBy"],
        serde_json::json!(["manual-check", "stop-at"])
    );

    for (payload, says) in [
        (
            serde_json::json!({ "kind": "diff", "wait": false, "title": "差分", "stoppedBy": ["unsure"] }),
            "a record does not stop the worker",
        ),
        (
            serde_json::json!({ "kind": "plan", "title": "計画", "stoppedBy": ["stop-at"] }),
            "stoppedBy is for diff and verify",
        ),
        (
            serde_json::json!({ "kind": "diff", "title": "差分", "stoppedBy": ["just-because"] }),
            "no such stop rule",
        ),
        (
            serde_json::json!({ "kind": "verify", "title": "動作確認", "stoppedBy": ["round-limit"] }),
            "round-limit cannot stop a verify gate",
        ),
        (
            serde_json::json!({ "kind": "diff", "title": "差分", "stoppedBy": ["manual-check"] }),
            "manual-check cannot stop a diff gate",
        ),
        (
            serde_json::json!({ "kind": "diff", "title": "差分", "stoppedBy": "unsure" }),
            "stoppedBy must be a list",
        ),
        (
            serde_json::json!({ "kind": "diff", "title": "差分" }),
            "a waiting diff gate needs stoppedBy",
        ),
        (
            serde_json::json!({ "kind": "verify", "title": "動作確認", "stoppedBy": [] }),
            "a waiting verify gate needs stoppedBy",
        ),
    ] {
        let out = open(payload.clone());
        assert!(!out.status.success(), "{payload}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(says),
            "{payload}: {out:?}"
        );
    }

    // A record with an empty list says nothing false, and is kept.
    let out = open(serde_json::json!({
        "kind": "diff", "wait": false, "title": "差分", "stoppedBy": [],
    }));
    assert!(out.status.success(), "{out:?}");
}
