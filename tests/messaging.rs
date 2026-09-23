//! Reports into a hub's inbox and messages into a worker's outbox, and the waking that follows
//! them.

mod common;

use common::*;

#[test]
fn a_report_survives_an_absent_hub_and_can_be_read_back_and_filed() {
    let fixture = Fixture::new(QUIET);
    let sent = fixture.ok(&[
        "send",
        "--from",
        "wid-1-worker",
        "--subject",
        "検索結果の画像が縦に潰れる",
        "--body",
        "## Symptom\nthe image is squashed",
    ]);
    assert!(sent.contains(HUB), "{sent}");
    assert!(sent.contains("The hub is not running"), "{sent}");

    let listed = fixture.json(&["pending", "--json"]);
    assert_eq!(listed["count"], 1);
    let name = listed["messages"][0]["name"].as_str().unwrap().to_string();
    assert_eq!(listed["messages"][0]["from"], "wid-1-worker");
    assert_eq!(listed["messages"][0]["kind"], "report");
    assert_eq!(
        listed["messages"][0]["subject"],
        "検索結果の画像が縦に潰れる"
    );

    let body = fixture.ok(&["pending", "--read", &name]);
    assert!(body.contains("the image is squashed"), "{body}");

    fixture.ok(&["pending", "--ack", &name]);
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

#[test]
fn a_report_says_which_worktree_it_came_from_not_which_repository() {
    // The address a `done` request acts on. `from` is free text — the sender picks it — so
    // a hub asked to close a tab and remove a worktree was acting on a path typed into the
    // body of the message. This header is derived from where the sender actually is.
    let fixture = Fixture::new(QUIET);
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-1");
    let added = Command::new("git")
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "wid-1",
            worktree.to_str().unwrap(),
        ])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );

    let sent = fixture
        .command([
            "send",
            "--kind",
            "done",
            "--subject",
            "終わったのだ",
            "--body",
            "b",
        ])
        .current_dir(&worktree)
        .output()
        .unwrap();
    assert!(
        sent.status.success(),
        "{}",
        String::from_utf8_lossy(&sent.stderr)
    );

    let listed = fixture.json(&["pending", "--json"]);
    let message = &listed["messages"][0];
    // The worktree it was sent from, and specifically not the main checkout: answering with
    // the checkout would identify the repository, which the inbox already knew.
    assert_eq!(message["worktree"], worktree.to_string_lossy().to_string());
    assert_ne!(
        message["worktree"],
        fixture.repo.to_string_lossy().to_string()
    );
    assert_eq!(message["kind"], "done");

    let name = message["name"].as_str().unwrap().to_string();
    let read = fixture.ok(&["pending", "--read", &name]);
    assert!(
        read.contains(&format!("worktree: {}", worktree.display())),
        "{read}"
    );

    // A sender git cannot place in a worktree still gets its message delivered, and the
    // header is simply absent. A bare clone is the case that reaches this: `git worktree
    // list` answers there — so the repository, and with it the inbox, is found — while
    // `rev-parse --show-toplevel` has nothing to say.
    let bare = fixture.repo.parent().unwrap().join("bare.git");
    for args in [
        vec!["init", "-q", "--bare", bare.to_str().unwrap()],
        vec![
            "-C",
            bare.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            "git@github.com:acme/widget.git",
        ],
    ] {
        assert!(
            Command::new("git")
                .args(&args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let placeless_send = fixture
        .command(["send", "--subject", "s2", "--body", "b2"])
        .current_dir(&bare)
        .output()
        .unwrap();
    assert!(
        placeless_send.status.success(),
        "{}",
        String::from_utf8_lossy(&placeless_send.stderr)
    );
    let listed = fixture.json(&["pending", "--json"]);
    let placeless = listed["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["subject"] == "s2")
        .unwrap();
    assert_eq!(placeless["worktree"], serde_json::Value::Null);
    let read = fixture.ok(&["pending", "--read", placeless["name"].as_str().unwrap()]);
    assert!(!read.contains("worktree:"), "{read}");

    // And an inbox that already held four-header messages when this shipped keeps working.
    let older =
        "---\nfrom: wid-2\nkind: report\nsubject: 前からあるやつ\nat: 20260908T041500Z\n---\n\nb\n";
    let dir = fixture.state.join("inbox").join(SLUG);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("20260908T041500Z-report.md"), older).unwrap();
    let listed = fixture.json(&["pending", "--json"]);
    let old = listed["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["subject"] == "前からあるやつ")
        .unwrap();
    assert_eq!(old["worktree"], serde_json::Value::Null);
    assert_eq!(old["from"], "wid-2");
}

#[test]
fn a_body_can_arrive_on_stdin_so_a_long_report_never_touches_the_command_line() {
    let fixture = Fixture::new(QUIET);
    let mut child = fixture
        .command(["send", "--from", "w", "--subject", "s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"line one\nline two\n")
        .unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    let name = fixture.json(&["pending", "--json"])["messages"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        fixture
            .ok(&["pending", "--read", &name])
            .contains("line two")
    );
}

#[test]
fn an_empty_report_is_refused_rather_than_filed() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.cmd(&["send", "--body", "   "]);
    assert!(!out.status.success());
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

#[test]
fn the_hub_leaves_a_message_in_the_worktree_and_the_worker_reads_it_there() {
    let fixture = Fixture::new(QUIET);
    let worktree = fixture.repo.to_str().unwrap().to_string();
    assert_eq!(
        fixture.ok(&["outbox", "--worktree", &worktree]).trim(),
        "(empty)"
    );

    fixture.ok(&[
        "tell",
        "--worktree",
        &worktree,
        "--subject",
        "[質問 20260908T041500Z] which screen is this about",
        "--body",
        "checking before filing WID-1",
    ]);
    let read = fixture.ok(&["outbox", "--worktree", &worktree]);
    assert!(read.contains(&format!("from {HUB}")), "{read}");
    assert!(read.contains("[質問 20260908T041500Z]"), "{read}");
    assert!(read.contains("checking before filing WID-1"), "{read}");

    // A second message appends rather than replacing the first.
    fixture.ok(&[
        "tell",
        "--worktree",
        &worktree,
        "--subject",
        "二件目",
        "--body",
        "b",
    ]);
    let both = fixture.ok(&["outbox", "--worktree", &worktree]);
    assert_eq!(both.matches("## ").count(), 2, "{both}");

    fixture.ok(&["outbox", "--worktree", &worktree, "--clear"]);
    assert_eq!(
        fixture.ok(&["outbox", "--worktree", &worktree]).trim(),
        "(empty)"
    );
}

#[test]
fn a_running_worker_gets_woken_the_same_way_a_hub_does() {
    let fixture = Fixture::new(
        r#"{"notification": "true", "workerWake": "true",
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
                      "issueKeys": {"acme/widget": "WID"}, "ide": "code"}}}"#,
    );
    let worktree = fixture.repo.to_str().unwrap().to_string();
    assert!(
        fixture
            .ok(&[
                "tell",
                "--worktree",
                &worktree,
                "--subject",
                "s",
                "--body",
                "b"
            ])
            .contains("The worker is not running")
    );

    // Register this process as the worker. A worker's command line says nothing
    // distinctive, so the record's anchor is the process start time.
    let record = fixture.repo.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    let started = Command::new("ps")
        .args(["-o", "lstart=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": std::process::id(),
            "psStarted": String::from_utf8_lossy(&started.stdout).trim(),
        })
        .to_string(),
    )
    .unwrap();
    let out = fixture.ok(&[
        "tell",
        "--worktree",
        &worktree,
        "--subject",
        "s2",
        "--body",
        "b2",
    ]);
    assert!(out.contains("Woke the worker"), "{out}");

    // A stale record — same pid, a start time from another process — must not read as alive.
    std::fs::write(
        &record,
        serde_json::json!({"pid": std::process::id(), "psStarted": "Thu Jan  1 00:00:00 1970"})
            .to_string(),
    )
    .unwrap();
    let stale = fixture.ok(&[
        "tell",
        "--worktree",
        &worktree,
        "--subject",
        "s3",
        "--body",
        "b3",
    ]);
    assert!(stale.contains("The worker is not running"), "{stale}");
}

#[test]
fn a_hub_that_is_running_gets_woken_and_the_sender_is_told_so() {
    // `true` stands in for whatever pokes the tab; what matters is that a present hub gets
    // the hook run and an absent one does not.
    let fixture = Fixture::new(
        r#"{"notification": "true", "hubWake": "true",
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
                      "issueKeys": {"acme/widget": "WID"}, "ide": "code"}}}"#,
    );
    assert!(
        fixture
            .ok(&["send", "--subject", "s", "--body", "b"])
            .contains("The hub is not running")
    );

    // Register this very process as the hub. Both anchors the presence check uses have to
    // be real: the name has to be in this process's command line, and the start time has to
    // be the one the system reports for it. The forged record used to carry neither — the
    // name was the *binary's* file name, which only matched because this repository's
    // directory is called `agent-adjutant`, and `psStarted` was left out entirely, so the
    // guard against a recycled pid was never once exercised by the test that covers it.
    let name = std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    let forge = |started: &str| {
        std::fs::write(
            &record,
            serde_json::json!({
                "pid": std::process::id(), "hubName": name, "cwd": "/", "psStarted": started,
            })
            .to_string(),
        )
        .unwrap();
    };
    forge(&ps_started(std::process::id()));
    let out = fixture.ok(&["send", "--subject", "s2", "--body", "b2"]);
    assert!(out.contains("Woke the hub"), "{out}");

    // Same live pid, a start time that is not its own: a pid that has been recycled onto
    // another process. Reporting that as present is the one mistake that loses a message.
    forge("Thu Jan  1 00:00:00 1970");
    let recycled = fixture.ok(&["send", "--subject", "s2b", "--body", "b2b"]);
    assert!(recycled.contains("The hub is not running"), "{recycled}");
    forge(&ps_started(std::process::id()));

    // Turning the hook off leaves the same present hub un-poked.
    std::fs::write(
        &fixture.config,
        r#"{"notification": "true", "hubWake": false,
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
                      "issueKeys": {"acme/widget": "WID"}, "ide": "code"}}}"#,
    )
    .unwrap();
    let quiet = fixture.ok(&["send", "--subject", "s3", "--body", "b3"]);
    assert!(quiet.contains("The hub is running"), "{quiet}");
}
