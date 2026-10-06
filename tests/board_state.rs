//! The board's documents, pinned whole: the state a page is drawn from, one task's history,
//! the list of boards and what `adj server status --json` says of them.
//!
//! Each is compared as a parsed value, so the order of keys does not matter but a key that
//! is `null` and one that is missing do. The fixture is written to disk as fixed constants
//! instead of being made through the endpoints, whose ids and stamps change from run to run;
//! what still varies (the clock, ports, pids, the tempdir) is replaced by a placeholder.

mod common;

use common::*;
use serde_json::{Value, json};

/// A pid no process has, so a record naming it is of one that is gone.
const GONE: u64 = 4_294_967_295;

/// What changes from one run to the next, to be rewritten in what the server says.
struct Run {
    canon: String,
    raw: String,
    token: String,
    port: u16,
    test_pid: u64,
    server_pid: u64,
}

fn run(fixture: &Fixture, resident: &Resident) -> Run {
    Run {
        canon: fixture.repo.parent().unwrap().to_str().unwrap().to_string(),
        raw: fixture._dir.path().to_str().unwrap().to_string(),
        token: resident.token.clone(),
        port: resident.port,
        test_pid: std::process::id() as u64,
        server_pid: resident.child.id() as u64,
    }
}

/// Rewrites values that exist; it never adds a key, so a key the server stopped sending is
/// still missing afterwards.
fn normalize(value: &mut Value, run: &Run, top: bool) {
    match value {
        Value::String(text) => {
            let mut s = text.replace(&run.token, "<token>");
            s = s.replace(&format!("127.0.0.1:{}", run.port), "127.0.0.1:<port>");
            // The resolved tempdir contains the raw one on macOS, so it goes first.
            s = s.replace(&run.canon, "<tmp>").replace(&run.raw, "<tmp>");
            *text = s;
        }
        Value::Array(items) => items.iter_mut().for_each(|v| normalize(v, run, false)),
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                match (key.as_str(), v.as_u64()) {
                    ("now", Some(_)) if top => *v = json!("<now>"),
                    ("pid", Some(n)) if n == run.test_pid => *v = json!("<pid>"),
                    ("pid", Some(n)) if n == run.server_pid => *v = json!("<server-pid>"),
                    ("port", Some(n)) if n == run.port as u64 => *v = json!("<port>"),
                    _ => normalize(v, run, false),
                }
            }
        }
        _ => {}
    }
}

fn write(path: &Path, value: Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}

fn git_worktree(fixture: &Fixture, path: &Path, name: &str) {
    let out = Command::new("git")
        .hermetic()
        .args(["worktree", "add", "-q", "-b", name])
        .arg(path)
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A board with a hub that is not running, a parent-task hub, a worker that is, one that
/// is not, a worker on the main checkout, three tasks, and the gates and mail around them.
///
/// The gate that stays open is opened after everything the worker has done, so reading the
/// state does not close it as answered in the terminal (`close_resumed`): the worker started
/// at 04:00, its last phase began at 04:30 and the gate was opened at 04:45.
fn board() -> (Fixture, Resident) {
    let fixture = Fixture::new(QUIET);
    let canon = fixture.repo.parent().unwrap().to_path_buf();
    let wid1 = canon.join("wid-1");
    let scratch = canon.join("scratch");
    let wid1_path = wid1.to_str().unwrap();
    let state = &fixture.state;

    write(
        &fixture.config,
        json!({
            "notification": "true",
            "defaults": {"ide": "code"},
            "terminal": {"preset": "tmux", "session": "adjutant-test",
                         "socket": "board-state-none", "attach": "true {session}"},
            "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
                                      "issueKeys": {"acme/widget": "WID"}}},
        }),
    );

    // A tmux that answers `-V` and nothing else, so every terminal feature is available and
    // no pane exists.
    let bin = fixture._dir.path().join("bin");
    stub_bin(
        &bin,
        "tmux",
        "#!/bin/sh\ncase \"$*\" in -V) echo \"tmux 3.4\";; esac\nexit 0\n",
    );

    // The parent-task hub is known only through its record, of a process that is gone.
    write(
        &state.join("hubs").join(format!("{FEATURE_SLUG}.json")),
        json!({"hubName": FEATURE_HUB, "cwd": fixture.repo.to_str().unwrap(), "hub": FEATURE,
               "pid": GONE, "psStarted": "Thu Jan  1 00:00:00 1970",
               "startedAt": "20260922T030000Z"}),
    );
    write(
        &state
            .join("hub-titles")
            .join(format!("{FEATURE_SLUG}.json")),
        json!({"url": "https://github.com/acme/widget/issues/957", "title": "Ship the feature"}),
    );

    // The repository's own hub has a saved session and no record.
    write(
        &state.join("sessions").join(format!("{SLUG}.json")),
        json!({"sessionId": "sid-hub", "nwo": "acme/widget"}),
    );
    write(
        &state.join("sessions").join(format!("{SLUG}.alive")),
        json!({"sessionId": "sid-hub", "lastAlive": 1_790_049_000}),
    );

    git_worktree(&fixture, &wid1, "wid-1");
    let pid = std::process::id();
    write(
        &wid1.join(".claude/adjutant-worker.json"),
        json!({"pid": pid, "psStarted": ps_started(pid), "title": "WID-1 Fix widget",
               "task": "WID-1", "startedAt": "20260922T040000Z", "phase": "review",
               "phaseAt": 1_790_051_400,
               "phases": [["plan", 1_790_049_660], ["implement", 1_790_050_200],
                          ["review", 1_790_051_400]],
               "terminal": {"backend": "tmux", "socket": canon.join("tmux-none").to_str().unwrap(),
                            "session": "work", "window": "@3", "pane": "%7"}}),
    );
    write(
        &wid1.join(".claude/adjutant-session.json"),
        json!({"sessionId": "sid-wid-1", "title": "WID-1 Fix widget", "task": "WID-1"}),
    );

    git_worktree(&fixture, &scratch, "scratch");
    write(
        &scratch.join(".claude/adjutant-worker.json"),
        json!({"pid": GONE, "psStarted": "Thu Jan  1 00:00:00 1970", "title": "scratch",
               "startedAt": "20260922T041000Z", "hub": FEATURE}),
    );
    write(
        &scratch.join(".claude/adjutant-session.json"),
        json!({"sessionId": "sid-scratch", "title": "scratch", "hub": FEATURE}),
    );

    write(
        &fixture.repo.join(".claude/adjutant-session.json"),
        json!({"sessionId": "sid-main", "title": "Main checkout worker", "task": "WID-2",
               "savedAt": "20260922T035500Z"}),
    );

    for (id, order, status, worktree, stamp) in [
        (
            "WID-1",
            0,
            "dispatched",
            Some(wid1_path),
            "20260922T035000Z",
        ),
        ("WID-2", 1, "queued", None, "20260922T035100Z"),
        ("WID-3", 2, "done", None, "20260922T034000Z"),
    ] {
        let mut task = json!({"id": id, "kind": "start", "title": format!("Task {id}"),
                              "doneWhen": "pr", "autoStart": true, "order": order,
                              "status": status, "createdAt": stamp, "updatedAt": stamp});
        if let Some(worktree) = worktree {
            task["worktree"] = json!(worktree);
        }
        write(
            &state.join("tasks").join(SLUG).join(format!("{id}.json")),
            task,
        );
    }

    // A task of the parent-task hub, which the repository's board shows apart from its own.
    write(
        &state.join("tasks").join(FEATURE_SLUG).join("WID-9.json"),
        json!({"id": "WID-9", "kind": "start", "title": "Task WID-9", "doneWhen": "pr",
               "autoStart": true, "order": 0, "status": "dispatched",
               "createdAt": "20260922T035200Z", "updatedAt": "20260922T035200Z"}),
    );

    let gates = state.join("gates").join(SLUG);
    write(
        &gates.join("answered/20260922T040500Z-plan.json"),
        json!({"id": "20260922T040500Z-plan", "kind": "plan", "task": "WID-1",
               "worktree": wid1_path, "title": "Plan", "openedAt": "20260922T040500Z",
               "decision": "approve", "answeredAt": "20260922T040800Z",
               "problem": "The widget drops its first click", "goal": "Handle the first click"}),
    );
    for (stamp, task, worktree) in [
        ("20260922T034500Z", "WID-3", fixture.repo.to_str().unwrap()),
        ("20260922T042000Z", "WID-1", wid1_path),
    ] {
        write(
            &gates.join(format!("records/{stamp}-diff-record.json")),
            json!({"id": format!("{stamp}-diff-record"), "kind": "diff", "task": task,
                   "worktree": worktree, "title": "round one", "wait": false,
                   "openedAt": stamp, "diff": "diff --git a/f b/f\n+one\n"}),
        );
    }
    write(
        &gates.join("g-wait.json"),
        json!({"id": "g-wait", "kind": "question", "task": "WID-1", "worktree": wid1_path,
               "title": "Which way?", "focus": "How the click is handled",
               "choices": [{"id": "a", "label": "A", "recommended": true},
                           {"id": "b", "label": "B"}],
               "openedAt": "20260922T044500Z"}),
    );

    let inbox = state.join("inbox").join(SLUG);
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(
        inbox.join("20260922T043000Z-report.md"),
        format!(
            "---\nfrom: wid-1\nworktree: {wid1_path}\nkind: report\nsubject: Done with plan\n\
             at: 20260922T043000Z\n---\n\nThe plan is done.\n"
        ),
    )
    .unwrap();
    std::fs::write(
        inbox.join("20260922T043500Z-question.md"),
        "---\nfrom: wid-1\nkind: question\nsubject: Which way?\nat: 20260922T043500Z\n---\n\n\
         A or B?\n",
    )
    .unwrap();

    let path = path_with(&bin);
    let resident = Resident::start_with(&fixture, &[("PATH", &path)]);
    (fixture, resident)
}

fn read(fixture: &Fixture, resident: &Resident, path: &str, query: &str) -> Value {
    let (status, body) = if query.is_empty() {
        resident.get(path)
    } else {
        get_with_query(resident, path, query)
    };
    assert_eq!(status, 200, "{body}");
    let mut value: Value = serde_json::from_str(&body).unwrap();
    normalize(&mut value, &run(fixture, resident), true);
    value
}

fn assert_same(actual: &Value, expected: &Value) {
    assert!(
        actual == expected,
        "actual:\n{}\nexpected:\n{}",
        serde_json::to_string_pretty(actual).unwrap(),
        serde_json::to_string_pretty(expected).unwrap()
    );
}

fn has_key(value: &Value, wanted: &str) -> bool {
    match value {
        Value::Object(map) => map.contains_key(wanted) || map.values().any(|v| has_key(v, wanted)),
        Value::Array(items) => items.iter().any(|v| has_key(v, wanted)),
        _ => false,
    }
}

fn state_path() -> String {
    format!("/b/{SLUG}/api/state")
}

fn expected_hubs() -> Value {
    json!([
      {
        "children": 0,
        "id": "hub",
        "inbox": [
          {
            "at": "20260922T043500Z",
            "from": "wid-1",
            "kind": "question",
            "name": "20260922T043500Z-question.md",
            "subject": "Which way?"
          },
          {
            "at": "20260922T043000Z",
            "from": "wid-1",
            "kind": "report",
            "name": "20260922T043000Z-report.md",
            "subject": "Done with plan",
            "worktree": "<tmp>/wid-1"
          }
        ],
        "inboxCount": 2,
        "key": null,
        "name": "adjutant-acme-widget-898449509108182c",
        "parent": false,
        "slug": "acme-widget-898449509108182c",
        "state": {
          "present": false,
          "stale": false
        }
      },
      {
        "children": 1,
        "id": "hub-wid-957",
        "inbox": [],
        "inboxCount": 0,
        "key": "wid-957",
        "name": "adjutant-acme-widget-wid-957-5283c95d4f4cc314",
        "parent": true,
        "slug": "acme-widget-wid-957-5283c95d4f4cc314",
        "state": {
          "pid": GONE,
          "present": false,
          "stale": true,
          "startedAt": "20260922T030000Z"
        },
        "title": "Ship the feature"
      }
    ])
}

fn expected_sessions() -> Value {
    json!([
      {
        "agent": "claude",
        "branch": "main",
        "conversation": "sid-hub",
        "id": "hub",
        "kind": "hub",
        "present": false,
        "stale": false,
        "terminal": {
          "backend": "tmux",
          "session": "adjutant-test",
          "socket": "board-state-none"
        },
        "title": "adjutant-acme-widget-898449509108182c",
        "worktree": "<tmp>/widget"
      },
      {
        "agent": "claude",
        "branch": "main",
        "id": "hub-wid-957",
        "key": "wid-957",
        "kind": "hub",
        "pid": GONE,
        "present": false,
        "stale": true,
        "startedAt": "20260922T030000Z",
        "terminal": {
          "backend": "tmux",
          "session": "adjutant-test",
          "socket": "board-state-none"
        },
        "title": "adjutant-acme-widget-wid-957-5283c95d4f4cc314",
        "worktree": "<tmp>/widget"
      },
      {
        "agent": "claude",
        "branch": "scratch",
        "conversation": "sid-scratch",
        "hub": "hub-wid-957",
        "id": "worker-scratch",
        "kind": "worker",
        "pid": GONE,
        "present": false,
        "stale": true,
        "startedAt": "20260922T041000Z",
        "terminal": {
          "backend": "tmux",
          "session": "adjutant-test",
          "socket": "board-state-none"
        },
        "title": "scratch",
        "worktree": "<tmp>/scratch"
      },
      {
        "agent": "claude",
        "branch": "wid-1",
        "conversation": "sid-wid-1",
        "hub": "hub",
        "id": "worker-wid-1",
        "kind": "worker",
        "phase": "review",
        "phaseAt": 1790051400,
        "phases": [
          [
            "plan",
            1790049660
          ],
          [
            "implement",
            1790050200
          ],
          [
            "review",
            1790051400
          ]
        ],
        "pid": "<pid>",
        "present": true,
        "stale": false,
        "startedAt": "20260922T040000Z",
        "task": "WID-1",
        "taskTitle": "Task WID-1",
        "terminal": {
          "backend": "tmux",
          "pane": "%7",
          "session": "work",
          "socket": "<tmp>/tmux-none",
          "window": "@3"
        },
        "title": "WID-1 Fix widget",
        "waiting": {
          "choices": [
            {
              "id": "a",
              "label": "A"
            },
            {
              "id": "b",
              "label": "B"
            }
          ],
          "count": 1,
          "focus": "How the click is handled",
          "hub": "hub",
          "id": "g-wait",
          "kind": "question",
          "openedAt": "20260922T044500Z",
          "options": [
            "answer"
          ],
          "slug": "acme-widget-898449509108182c",
          "title": "Which way?"
        },
        "worktree": "<tmp>/wid-1"
      },
      {
        "agent": "claude",
        "branch": "main",
        "conversation": "sid-main",
        "hub": "hub",
        "id": "worker-main",
        "kind": "worker",
        "present": false,
        "stale": false,
        "task": "WID-2",
        "taskTitle": "Task WID-2",
        "terminal": {
          "backend": "tmux",
          "session": "adjutant-test",
          "socket": "board-state-none"
        },
        "title": "Main checkout worker",
        "worktree": "<tmp>/widget"
      }
    ])
}

fn expected_tasks() -> Value {
    json!([
      {
        "approvedPlan": {
          "answeredAt": "20260922T040800Z",
          "decision": "approve",
          "goal": "Handle the first click",
          "id": "20260922T040500Z-plan",
          "kind": "plan",
          "openedAt": "20260922T040500Z",
          "problem": "The widget drops its first click",
          "rounds": 0,
          "task": "WID-1",
          "title": "Plan",
          "worktree": "<tmp>/wid-1"
        },
        "autoStart": true,
        "createdAt": "20260922T035000Z",
        "doneWhen": "pr",
        "id": "WID-1",
        "kind": "start",
        "order": 0,
        "prTurn": null,
        "records": [
          {
            "answeredByHub": false,
            "diffSize": 24,
            "id": "20260922T042000Z-diff-record",
            "kind": "diff",
            "openedAt": "20260922T042000Z",
            "rounds": 0,
            "task": "WID-1",
            "title": "round one",
            "wait": false,
            "worktree": "<tmp>/wid-1"
          }
        ],
        "status": "dispatched",
        "stopAt": "plan",
        "title": "Task WID-1",
        "updatedAt": "20260922T035000Z",
        "waitsOnPerson": false,
        "worktree": "<tmp>/wid-1"
      },
      {
        "approvedPlan": null,
        "autoStart": true,
        "createdAt": "20260922T035100Z",
        "doneWhen": "pr",
        "id": "WID-2",
        "kind": "start",
        "order": 1,
        "prTurn": null,
        "records": [],
        "status": "queued",
        "stopAt": "plan",
        "title": "Task WID-2",
        "updatedAt": "20260922T035100Z",
        "waitsOnPerson": false
      },
      {
        "autoStart": true,
        "createdAt": "20260922T034000Z",
        "doneWhen": "pr",
        "id": "WID-3",
        "kind": "start",
        "order": 2,
        "status": "done",
        "stopAt": "plan",
        "title": "Task WID-3",
        "updatedAt": "20260922T034000Z",
        "waitsOnPerson": false
      }
    ])
}

fn expected_hub_tasks() -> Value {
    json!([
      {
        "autoStart": true,
        "createdAt": "20260922T035200Z",
        "doneWhen": "pr",
        "id": "WID-9",
        "kind": "start",
        "order": 0,
        "ownerHub": {
          "humanCol": null,
          "key": FEATURE,
          "slug": FEATURE_SLUG
        },
        "prTurn": null,
        "records": [],
        "approvedPlan": null,
        "status": "dispatched",
        "stopAt": "plan",
        "title": "Task WID-9",
        "updatedAt": "20260922T035200Z",
        "waitsOnPerson": false
      }
    ])
}

fn expected_workers() -> Value {
    json!([
      {
        "branch": "scratch",
        "name": "scratch",
        "phase": null,
        "phaseAt": null,
        "hubSlug": FEATURE_SLUG,
        "present": false,
        "stale": true,
        "task": null,
        "title": "scratch",
        "worktree": "<tmp>/scratch"
      },
      {
        "branch": "wid-1",
        "hubSlug": SLUG,
        "name": "wid-1",
        "phase": "review",
        "phaseAt": 1790051400,
        "present": true,
        "stale": false,
        "task": "WID-1",
        "title": "WID-1 Fix widget",
        "worktree": "<tmp>/wid-1"
      }
    ])
}

fn expected_gates() -> Value {
    json!([
      {
        "answeredByHub": false,
        "choices": [
          {
            "id": "a",
            "label": "A",
            "recommended": true
          },
          {
            "id": "b",
            "label": "B",
            "recommended": false
          }
        ],
        "focus": "How the click is handled",
        "humanCol": "question",
        "id": "g-wait",
        "kind": "question",
        "openedAt": "20260922T044500Z",
        "rounds": 0,
        "task": "WID-1",
        "title": "Which way?",
        "worktree": "<tmp>/wid-1"
      }
    ])
}

fn expected_pending() -> Value {
    json!([
      {
        "from": "wid-1",
        "kind": "report",
        "name": "20260922T043000Z-report.md",
        "subject": "Done with plan",
        "worktree": "<tmp>/wid-1"
      },
      {
        "from": "wid-1",
        "kind": "question",
        "name": "20260922T043500Z-question.md",
        "subject": "Which way?",
        "worktree": null
      }
    ])
}

fn expected_state(sessions: Value) -> Value {
    let mut state = json!({
      "boardTerminal": {
        "available": true
      },
      "configPath": "<tmp>/config.json",
      "hub": {
        "pid": null,
        "present": false,
        "stale": false,
        "startedAt": null
      },
      "hubName": "adjutant-acme-widget-898449509108182c",
      "hubResume": {
        "available": true,
        "reason": null
      },
      "hubRunner": "claude -n {name} --session-id {sessionId} --permission-mode auto {prompt}",
      "hubStart": {
        "available": true
      },
      "ideConfigured": true,
      "main": "<tmp>/widget",
      "now": "<now>",
      "prPoll": {
        "active": true,
        "error": null
      },
      "repo": "acme/widget",
      "resident": true,
      "sessionOpen": {
        "available": true,
        "terminal": "terminal.attach"
      },
      "sessionResume": {
        "available": true,
        "reason": null
      },
      "sessionStart": {
        "agent": "claude"
      },
      "stuckAfterMinutes": 120.0,
      "workerSlots": {
        "busy": 1,
        "max": null
      }
    });
    state["hubs"] = expected_hubs();
    state["sessions"] = sessions;
    state["tasks"] = expected_tasks();
    state["hubTasks"] = expected_hub_tasks();
    state["workers"] = expected_workers();
    state["gates"] = expected_gates();
    state["pending"] = expected_pending();
    state
}

fn expected_history() -> Value {
    json!({
      "answered": [
        {
          "answeredAt": "20260922T040800Z",
          "decision": "approve",
          "goal": "Handle the first click",
          "id": "20260922T040500Z-plan",
          "kind": "plan",
          "openedAt": "20260922T040500Z",
          "problem": "The widget drops its first click",
          "rounds": 0,
          "task": "WID-1",
          "title": "Plan",
          "worktree": "<tmp>/wid-1"
        }
      ],
      "records": [
        {
          "diff": "diff --git a/f b/f\n+one\n",
          "id": "20260922T042000Z-diff-record",
          "kind": "diff",
          "openedAt": "20260922T042000Z",
          "rounds": 0,
          "task": "WID-1",
          "title": "round one",
          "wait": false,
          "worktree": "<tmp>/wid-1"
        }
      ]
    })
}

fn expected_boards() -> Value {
    json!([
      {
        "finished": false,
        "gates": [
          {
            "id": "g-wait",
            "kind": "question",
            "openedAt": "20260922T044500Z",
            "task": "WID-1",
            "title": "Which way?",
            "worktree": "<tmp>/wid-1"
          }
        ],
        "hub": null,
        "hubId": "hub",
        "hubLastAlive": 1790049000,
        "hubPresent": false,
        "hubStale": false,
        "hubStartedAt": null,
        "nwo": "acme/widget",
        "queued": 1,
        "slug": "acme-widget-898449509108182c",
        "title": null,
        "url": "http://127.0.0.1:<port>/b/acme-widget-898449509108182c/?token=<token>",
        "waiting": 1,
        "working": 0
      },
      {
        "finished": false,
        "gates": [],
        "hub": "wid-957",
        "hubId": "hub-wid-957",
        "hubLastAlive": null,
        "hubPresent": false,
        "hubStale": true,
        "hubStartedAt": "20260922T030000Z",
        "nwo": "acme/widget",
        "queued": 0,
        "slug": "acme-widget-wid-957-5283c95d4f4cc314",
        "title": "Ship the feature",
        "url": "http://127.0.0.1:<port>/b/acme-widget-wid-957-5283c95d4f4cc314/?token=<token>",
        "waiting": 0,
        "working": 1
      }
    ])
}

#[test]
fn the_board_state_is_pinned_whole() {
    let (fixture, resident) = board();
    let state = read(&fixture, &resident, &state_path(), "");
    assert_same(&state, &expected_state(expected_sessions()));

    let gates = fixture.state.join("gates").join(SLUG);
    assert!(gates.join("g-wait.json").exists());
    assert!(!gates.join("answered/g-wait.json").exists());
}

#[test]
fn a_parent_hubs_board_lists_none_of_the_other_hubs_tasks() {
    let (fixture, resident) = board();
    let state = read(
        &fixture,
        &resident,
        &format!("/b/{FEATURE_SLUG}/api/state"),
        "",
    );
    assert_eq!(state["hubTasks"], json!([]), "{state}");
    let ids: Vec<&str> = state["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["WID-9"]);
}

#[test]
fn the_state_without_sessions_lists_none() {
    let (fixture, resident) = board();
    let state = read(&fixture, &resident, &state_path(), "sessions=0");
    assert_same(&state, &expected_state(json!([])));
}

#[test]
fn the_state_with_lines_has_no_last_line_without_a_pane() {
    let (fixture, resident) = board();
    let state = read(&fixture, &resident, &state_path(), "lines=1");
    assert!(!has_key(&state, "lastLine"), "{state}");
    assert_same(&state, &expected_state(expected_sessions()));
}

#[test]
fn a_live_tasks_history_is_pinned_whole() {
    let (fixture, resident) = board();
    let history = read(
        &fixture,
        &resident,
        &format!("/b/{SLUG}/api/tasks/WID-1/history"),
        "",
    );
    assert_same(&history, &expected_history());
}

#[test]
fn the_board_list_is_pinned_whole() {
    let (fixture, resident) = board();
    let boards = read(&fixture, &resident, "/api/boards", "");
    assert_same(&boards, &expected_boards());
}

#[test]
fn server_status_lists_the_boards_as_the_board_list_does() {
    let (fixture, resident) = board();
    let mut status = fixture.json(&["server", "status", "--json"]);
    normalize(&mut status, &run(&fixture, &resident), true);
    assert_same(
        &status,
        &json!({"running": true, "pid": "<server-pid>", "port": "<port>",
                "url": "http://127.0.0.1:<port>/?token=<token>",
                "boards": expected_boards()}),
    );
    let boards = read(&fixture, &resident, "/api/boards", "");
    assert_same(&status["boards"], &boards);
}
