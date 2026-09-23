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
