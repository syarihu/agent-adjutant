//! `adj task next`: which queued task a free worker slot takes, and which ones ask first.

mod common;

use common::*;

#[test]
fn next_skips_held_tasks_and_names_the_ones_that_need_a_dispatch_gate() {
    let fixture = Fixture::new(QUIET);
    // Added in this order, so this is the queue order too.
    let add = |title: &str, extra: &[&str]| -> String {
        let mut args = vec!["task", "add", "--title", title, "--body", "x"];
        args.extend(extra.iter().copied());
        args.push("--json");
        fixture.json(&args)["task"]["id"]
            .as_str()
            .unwrap()
            .to_string()
    };

    let asks = add("asks first", &["--ask-first", "--queue"]);
    let failed = add("failed", &["--queue"]);
    fixture.ok(&[
        "task",
        "update",
        "--id",
        &failed,
        "--note",
        "Could not start: boom",
    ]);
    let ready = add("ready", &["--queue"]);
    let later = add("later", &["--queue"]);

    let next = fixture.json(&["task", "next", "--json"]);
    assert_eq!(next["task"]["id"], ready, "{}", next);
    assert_eq!(next["needsDispatchGate"][0]["id"], asks, "{}", next);
    assert_eq!(
        next["needsDispatchGate"].as_array().unwrap().len(),
        1,
        "{}",
        next
    );

    // Open a dispatch gate for `asks`
    let gate_file = fixture.repo.join("gate.json");
    std::fs::write(
        &gate_file,
        format!(
            "{{\"kind\":\"dispatch\",\"task\":\"{asks}\",\"title\":\"{asks}\",\"worktree\":\"{}\"}}",
            fixture.repo.display()
        ),
    )
    .unwrap();
    fixture.json(&[
        "gate",
        "open",
        "--file",
        gate_file.to_str().unwrap(),
        "--json",
    ]);

    let next_after_gate = fixture.json(&["task", "next", "--json"]);
    assert_eq!(next_after_gate["task"]["id"], ready, "{}", next_after_gate);
    assert!(
        next_after_gate["needsDispatchGate"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{}",
        next_after_gate
    );

    fixture.ok(&["task", "update", "--id", &ready, "--status", "dispatched"]);

    let next_after_ready_dispatched = fixture.json(&["task", "next", "--json"]);
    assert_eq!(
        next_after_ready_dispatched["task"]["id"], later,
        "{}",
        next_after_ready_dispatched
    );

    fixture.ok(&["task", "update", "--id", &later, "--status", "dispatched"]);
    let next_end = fixture.json(&["task", "next", "--json"]);
    assert!(next_end["task"].is_null(), "{}", next_end);
    let output_bytes = fixture.cmd(&["task", "next"]).stdout;
    let output = String::from_utf8_lossy(&output_bytes);
    assert!(
        output.contains("Nothing queued can be started."),
        "{}",
        output
    );
}

#[test]
fn a_task_parent_is_set_and_cleared_by_update_and_a_bad_one_changes_nothing() {
    let fixture = Fixture::new(QUIET);
    let id =
        fixture.json(&["task", "add", "--title", "child", "--body", "x", "--json"])["task"]["id"]
            .as_str()
            .unwrap()
            .to_string();
    let parent = || fixture.json(&["task", "show", "--id", &id])["parent"].clone();
    assert!(parent().is_null());

    let url = "https://github.com/acme/widget/issues/549";
    fixture.ok(&["task", "update", "--id", &id, "--parent", url]);
    assert_eq!(parent(), url);

    // A bad value is refused, and the record keeps what it had.
    let refused = fixture.cmd(&["task", "update", "--id", &id, "--parent", "a; b"]);
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("a; b"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert_eq!(parent(), url);

    // An empty one clears it.
    fixture.ok(&["task", "update", "--id", &id, "--parent", ""]);
    assert!(parent().is_null());
}
