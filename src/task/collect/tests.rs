use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::infra::gh::GhRun;

fn gh_ok(stdout: impl Into<String>) -> GhRun {
    GhRun {
        ok: true,
        stdout: stdout.into(),
        stderr: String::new(),
    }
}

fn gh_err(stderr: impl Into<String>) -> GhRun {
    GhRun {
        ok: false,
        stdout: String::new(),
        stderr: stderr.into(),
    }
}

struct FakeGh {
    calls: Mutex<Vec<String>>,
    canned: HashMap<String, GhRun>,
    recorded_inputs: Mutex<Vec<Option<Vec<u8>>>>,
}

impl FakeGh {
    fn new(canned: Vec<(&str, GhRun)>) -> Self {
        let mut map = HashMap::new();
        for (k, v) in canned {
            map.insert(k.to_string(), v);
        }
        Self {
            calls: Mutex::new(Vec::new()),
            canned: map,
            recorded_inputs: Mutex::new(Vec::new()),
        }
    }

    fn runner(
        &self,
    ) -> impl Fn(&[&str], Option<&[u8]>, Instant) -> Result<GhRun, String> + Sync + '_ {
        move |args: &[&str], input: Option<&[u8]>, _dl: Instant| {
            let key = args.join(" ");
            self.calls.lock().unwrap().push(key.clone());
            self.recorded_inputs
                .lock()
                .unwrap()
                .push(input.map(|b| b.to_vec()));
            match self.canned.get(&key) {
                Some(run) => Ok(GhRun {
                    ok: run.ok,
                    stdout: run.stdout.clone(),
                    stderr: run.stderr.clone(),
                }),
                None => panic!("unexpected gh call: {key}"),
            }
        }
    }
}

fn fake_worktrees(proctor: bool, wts: Vec<(&str, Option<&str>)>) -> impl Fn() -> WorktreeListing {
    move || WorktreeListing {
        proctor,
        worktrees: Ok(wts
            .iter()
            .map(|(p, b)| Worktree {
                path: p.to_string(),
                branch: b.map(str::to_string),
            })
            .collect()),
    }
}

fn fake_worktrees_fail(proctor: bool, err: &str) -> impl Fn() -> WorktreeListing {
    let err_msg = err.to_string();
    move || WorktreeListing {
        proctor,
        worktrees: Err(err_msg.clone()),
    }
}

fn sample_keys() -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    map.insert(
        "example/team-app".to_string(),
        Value::String("ALPHA".to_string()),
    );
    map.insert(
        "example/team".to_string(),
        Value::String("GAMMA".to_string()),
    );
    map
}

// 1. github source: assigned + unassigned rows, labels -> status (bug, wip), no labels -> -,
// item -, assignee @me / -, key from issueKeys; exact D1/D2 argument vectors.
#[test]
fn test_01_dashboard_github_source() {
    let d1 = "issue list -R example/team-app --assignee @me --state open --json number,title,labels,url --limit 50";
    let d2 = "issue list -R example/team-app --search no:assignee --state open --json number,title,labels,url --limit 50";
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";

    let fake = FakeGh::new(vec![
        (
            d1,
            gh_ok(
                r#"[{"number":1,"title":"Bug report","labels":[{"name":"bug"},{"name":"wip"}],"url":"https://example.com/1"}]"#,
            ),
        ),
        (
            d2,
            gh_ok(
                r#"[{"number":2,"title":"Feature request","labels":[],"url":"https://example.com/2"}]"#,
            ),
        ),
        (d6, gh_ok("[]")),
    ]);

    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(false, vec![])).unwrap();
    assert_eq!(col.mode, "dashboard");
    assert_eq!(col.rows.len(), 2);

    match &col.rows[0] {
        Row::Task(t) => {
            assert_eq!(t.kind, "task");
            assert_eq!(t.identifier, "example/team-app#1");
            assert_eq!(t.key, "ALPHA-1");
            assert_eq!(t.item, "-");
            assert_eq!(t.status, "bug, wip");
            assert_eq!(t.assignee, "@me");
        }
        _ => panic!("expected task row"),
    }

    match &col.rows[1] {
        Row::Task(t) => {
            assert_eq!(t.kind, "task");
            assert_eq!(t.identifier, "example/team-app#2");
            assert_eq!(t.key, "ALPHA-2");
            assert_eq!(t.item, "-");
            assert_eq!(t.status, "-");
            assert_eq!(t.assignee, "-");
        }
        _ => panic!("expected task row"),
    }

    let calls = fake.calls.lock().unwrap();
    assert_eq!(calls[0], d1);
    assert_eq!(calls[1], d2);
    assert_eq!(calls[2], d6);
}

// 2. github-project: D3/D4 then exactly one nodes(ids:) call whose stdin holds the ids as
// variables.ids and whose query text holds no id; status/item from the item of the
// source's projectNumber; an issue with items only on another board -> no-item / N/A;
// an item without Status -> N/A.
#[test]
fn test_02_dashboard_github_project_source() {
    let d3 = "search issues --assignee @me --state open --project example/1 --json id,number,title,url,repository --limit 50";
    let d4 = "search issues no:assignee --state open --project example/1 --json id,number,title,url,repository --limit 50";
    let nodes_call = "api graphql -H Content-Type: application/json --input -";
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";

    let nodes_body = serde_json::json!({
        "data": {
            "nodes": [
                {
                    "projectItems": {
                        "nodes": [
                            {
                                "id": "item-1",
                                "project": { "number": 1, "owner": { "login": "example" } },
                                "fieldValueByName": { "name": "In Progress" }
                            }
                        ]
                    }
                },
                {
                    "projectItems": {
                        "nodes": [
                            {
                                "id": "item-other",
                                "project": { "number": 99, "owner": { "login": "example" } },
                                "fieldValueByName": { "name": "Done" }
                            }
                        ]
                    }
                },
                {
                    "projectItems": {
                        "nodes": [
                            {
                                "id": "item-3",
                                "project": { "number": 1, "owner": { "login": "example" } },
                                "fieldValueByName": null
                            }
                        ]
                    }
                }
            ]
        }
    });

    let fake = FakeGh::new(vec![
        (
            d3,
            gh_ok(
                r#"[{"id":"node-1","number":1,"title":"T1","url":"https://example.com/1","repository":{"nameWithOwner":"example/team-app"}}]"#,
            ),
        ),
        (
            d4,
            gh_ok(
                r#"[
                {"id":"node-2","number":2,"title":"T2","url":"https://example.com/2","repository":{"nameWithOwner":"example/team-app"}},
                {"id":"node-3","number":3,"title":"T3","url":"https://example.com/3","repository":{"nameWithOwner":"example/team-app"}}
            ]"#,
            ),
        ),
        (
            nodes_call,
            gh_ok(serde_json::to_string(&nodes_body).unwrap()),
        ),
        (d6, gh_ok("[]")),
    ]);

    let sources = vec![serde_json::json!({
        "type": "github-project",
        "projectOwner": "example",
        "projectNumber": 1
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(false, vec![])).unwrap();
    assert_eq!(col.rows.len(), 3);

    // Verify stdin of nodes call: holds variables.ids and query text holds no id
    let recorded = fake.recorded_inputs.lock().unwrap();
    let nodes_input = recorded[2].as_ref().expect("nodes call has input");
    let input_val: Value = serde_json::from_slice(nodes_input).unwrap();
    let ids = input_val
        .pointer("/variables/ids")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], "node-1");
    let query_str = input_val["query"].as_str().unwrap();
    assert!(!query_str.contains("node-1"));

    match &col.rows[0] {
        Row::Task(t) => {
            assert_eq!(t.item, "item-1");
            assert_eq!(t.status, "In Progress");
        }
        _ => panic!(),
    }

    // issue with items only on another board -> no-item / N/A
    match &col.rows[1] {
        Row::Task(t) => {
            assert_eq!(t.item, "no-item");
            assert_eq!(t.status, "N/A");
        }
        _ => panic!(),
    }

    // item without Status -> N/A
    match &col.rows[2] {
        Row::Task(t) => {
            assert_eq!(t.item, "item-3");
            assert_eq!(t.status, "N/A");
        }
        _ => panic!(),
    }
}

// 3. nodes(ids:) fails (non-zero, no data) -> ? in status and item on that source's rows,
// one Failed with lookup nodes(ids:); a null node -> only that row ?.
#[test]
fn test_03_dashboard_nodes_failure_and_null_node() {
    let d3 = "search issues --assignee @me --state open --project example/1 --json id,number,title,url,repository --limit 50";
    let d4 = "search issues no:assignee --state open --project example/1 --json id,number,title,url,repository --limit 50";
    let nodes_call = "api graphql -H Content-Type: application/json --input -";
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";

    // Subtest A: nodes fails
    let fake_a = FakeGh::new(vec![
        (
            d3,
            gh_ok(
                r#"[{"id":"node-1","number":1,"title":"T1","url":"https://example.com/1","repository":{"nameWithOwner":"example/team-app"}}]"#,
            ),
        ),
        (d4, gh_ok("[]")),
        (nodes_call, gh_err("GraphQL network timeout")),
        (d6, gh_ok("[]")),
    ]);
    let sources = vec![serde_json::json!({
        "type": "github-project",
        "projectOwner": "example",
        "projectNumber": 1
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let col_a = collect_with(&input, &fake_a.runner(), &fake_worktrees(false, vec![])).unwrap();
    match &col_a.rows[0] {
        Row::Task(t) => {
            assert_eq!(t.item, "?");
            assert_eq!(t.status, "?");
        }
        _ => panic!(),
    }
    assert_eq!(col_a.failed.len(), 1);
    assert_eq!(col_a.failed[0].lookup, "nodes(ids:)");

    // Subtest B: null node in data.nodes -> only that row ?
    let nodes_body_b = serde_json::json!({
        "data": {
            "nodes": [
                {
                    "projectItems": {
                        "nodes": [
                            {
                                "id": "item-1",
                                "project": { "number": 1, "owner": { "login": "example" } },
                                "fieldValueByName": { "name": "Todo" }
                            }
                        ]
                    }
                },
                null
            ]
        }
    });
    let fake_b = FakeGh::new(vec![
        (
            d3,
            gh_ok(
                r#"[
                {"id":"node-1","number":1,"title":"T1","url":"https://example.com/1","repository":{"nameWithOwner":"example/team-app"}},
                {"id":"node-2","number":2,"title":"T2","url":"https://example.com/2","repository":{"nameWithOwner":"example/team-app"}}
            ]"#,
            ),
        ),
        (d4, gh_ok("[]")),
        (
            nodes_call,
            gh_ok(serde_json::to_string(&nodes_body_b).unwrap()),
        ),
        (d6, gh_ok("[]")),
    ]);
    let col_b = collect_with(&input, &fake_b.runner(), &fake_worktrees(false, vec![])).unwrap();
    match &col_b.rows[0] {
        Row::Task(t) => {
            assert_eq!(t.item, "item-1");
            assert_eq!(t.status, "Todo");
        }
        _ => panic!(),
    }
    match &col_b.rows[1] {
        Row::Task(t) => {
            assert_eq!(t.item, "?");
            assert_eq!(t.status, "?");
        }
        _ => panic!(),
    }
}

// 4. One list fails (non-zero exit with stderr) -> no rows from it, Failed names the source and
// assigned list; the other list and other sources still produce rows.
#[test]
fn test_04_dashboard_one_list_fails() {
    let d1 = "issue list -R example/team-app --assignee @me --state open --json number,title,labels,url --limit 50";
    let d2 = "issue list -R example/team-app --search no:assignee --state open --json number,title,labels,url --limit 50";
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";

    let fake = FakeGh::new(vec![
        (d1, gh_err("HTTP 502 bad gateway")),
        (
            d2,
            gh_ok(r#"[{"number":2,"title":"T2","labels":[],"url":"https://example.com/2"}]"#),
        ),
        (d6, gh_ok("[]")),
    ]);

    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(false, vec![])).unwrap();
    assert_eq!(col.rows.len(), 1);
    assert_eq!(col.failed.len(), 1);
    assert_eq!(col.failed[0].what, "github example/team-app");
    assert_eq!(col.failed[0].lookup, "assigned list");
    assert!(col.failed[0].error.contains("HTTP 502 bad gateway"));
}

// 5. Unparsable stdout from an ok call -> treated as a failure, not as none.
#[test]
fn test_05_dashboard_unparsable_stdout_is_failure() {
    let d1 = "issue list -R example/team-app --assignee @me --state open --json number,title,labels,url --limit 50";
    let d2 = "issue list -R example/team-app --search no:assignee --state open --json number,title,labels,url --limit 50";
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";

    let fake = FakeGh::new(vec![
        (d1, gh_ok("not-json-garbage")),
        (d2, gh_ok("[]")),
        (d6, gh_ok("[]")),
    ]);

    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(false, vec![])).unwrap();
    assert_eq!(col.rows.len(), 0);
    assert_eq!(col.failed.len(), 1);
    assert_eq!(col.failed[0].lookup, "assigned list");
}

// 6. Every list of every source fails -> Ok, no task rows, every failure in failed, PR and
// worktree rows still present.
#[test]
fn test_06_dashboard_all_lists_fail_still_ok() {
    let d1 = "issue list -R example/team-app --assignee @me --state open --json number,title,labels,url --limit 50";
    let d2 = "issue list -R example/team-app --search no:assignee --state open --json number,title,labels,url --limit 50";
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";

    let fake = FakeGh::new(vec![
        (d1, gh_err("err1")),
        (d2, gh_err("err2")),
        (
            d6,
            gh_ok(
                r#"[{"number":10,"title":"PR1","url":"https://example.com/pr/10","isDraft":false,"statusCheckRollup":[]}]"#,
            ),
        ),
    ]);

    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let lister = fake_worktrees(false, vec![("/path/wt", Some("feat"))]);
    let col = collect_with(&input, &fake.runner(), &lister).unwrap();
    assert_eq!(col.rows.len(), 2); // 1 PR + 1 Worktree
    assert_eq!(col.failed.len(), 2);
}

// 7. Dedupe: the same issue on two boards (and with different identifier case) -> one row at
// the first position, status/item from the source with projectFields even when it is the
// second source; with neither having projectFields, the first.
#[test]
fn test_07_dashboard_dedupe_project_fields_precedence() {
    let d3_s1 = "search issues --assignee @me --state open --project example/1 --json id,number,title,url,repository --limit 50";
    let d4_s1 = "search issues no:assignee --state open --project example/1 --json id,number,title,url,repository --limit 50";
    let d3_s2 = "search issues --assignee @me --state open --project example/2 --json id,number,title,url,repository --limit 50";
    let d4_s2 = "search issues no:assignee --state open --project example/2 --json id,number,title,url,repository --limit 50";
    let nodes_call = "api graphql -H Content-Type: application/json --input -";
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";

    let nodes_body_1 = serde_json::json!({
        "data": {
            "nodes": [
                {
                    "projectItems": {
                        "nodes": [{ "id": "item-board-1", "project": { "number": 1, "owner": { "login": "example" } }, "fieldValueByName": { "name": "Status 1" } }]
                    }
                }
            ]
        }
    });

    let nodes_body_2 = serde_json::json!({
        "data": {
            "nodes": [
                {
                    "projectItems": {
                        "nodes": [{ "id": "item-board-2", "project": { "number": 2, "owner": { "login": "example" } }, "fieldValueByName": { "name": "Status 2" } }]
                    }
                }
            ]
        }
    });

    let fake = FakeGh::new(vec![
        // Board 1: no projectFields
        (
            d3_s1,
            gh_ok(
                r#"[{"id":"node-1","number":5,"title":"T5","url":"https://example.com/5","repository":{"nameWithOwner":"example/team-app"}}]"#,
            ),
        ),
        (d4_s1, gh_ok("[]")),
        (
            d3_s2,
            gh_ok(
                r#"[{"id":"node-1","number":5,"title":"T5","url":"https://example.com/5","repository":{"nameWithOwner":"EXAMPLE/TEAM-APP"}}]"#,
            ),
        ),
        (d4_s2, gh_ok("[]")),
        // First nodes call for Board 1, second for Board 2
        (
            nodes_call,
            gh_ok(serde_json::to_string(&nodes_body_1).unwrap()),
        ),
        (d6, gh_ok("[]")),
    ]);

    // Override fake to return board 2 response on second nodes call
    let sources = vec![
        serde_json::json!({
            "type": "github-project",
            "projectOwner": "example",
            "projectNumber": 1
        }),
        serde_json::json!({
            "type": "github-project",
            "projectOwner": "example",
            "projectNumber": 2,
            "projectFields": { "Status": "status-id" }
        }),
    ];

    let calls_count = AtomicUsize::new(0);
    let runner = |args: &[&str], input: Option<&[u8]>, dl: Instant| -> Result<GhRun, String> {
        let key = args.join(" ");
        if key == nodes_call {
            let count = calls_count.fetch_add(1, Ordering::Relaxed) + 1;
            if count == 1 {
                Ok(gh_ok(serde_json::to_string(&nodes_body_1).unwrap()))
            } else {
                Ok(gh_ok(serde_json::to_string(&nodes_body_2).unwrap()))
            }
        } else {
            fake.runner()(args, input, dl)
        }
    };

    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &runner, &fake_worktrees(false, vec![])).unwrap();
    assert_eq!(col.rows.len(), 1);
    match &col.rows[0] {
        Row::Task(t) => {
            // Second source has projectFields, so its item and status win!
            assert_eq!(t.item, "item-board-2");
            assert_eq!(t.status, "Status 2");
        }
        _ => panic!(),
    }
}

// 8. issueKeys lookup: case-insensitive repo match works; a repo absent -> leftOut
// [{repo, count}] counted once per issue across sources, no row.
#[test]
fn test_08_dashboard_issue_keys_and_left_out() {
    let d1 = "issue list -R example/team-app --assignee @me --state open --json number,title,labels,url --limit 50";
    let d2 = "issue list -R example/team-app --search no:assignee --state open --json number,title,labels,url --limit 50";
    let d3 = "search issues --assignee @me --state open --project example/1 --json id,number,title,url,repository --limit 50";
    let d4 = "search issues no:assignee --state open --project example/1 --json id,number,title,url,repository --limit 50";
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";

    let fake = FakeGh::new(vec![
        (d1, gh_ok("[]")),
        (d2, gh_ok("[]")),
        (
            d3,
            gh_ok(
                r#"[
                {"id":"n1","number":1,"title":"T1","url":"https://example.com/1","repository":{"nameWithOwner":"example/other"}},
                {"id":"n2","number":2,"title":"T2","url":"https://example.com/2","repository":{"nameWithOwner":"EXAMPLE/OTHER"}}
            ]"#,
            ),
        ),
        (
            d4,
            gh_ok(
                r#"[
                {"id":"n1","number":1,"title":"T1","url":"https://example.com/1","repository":{"nameWithOwner":"example/other"}}
            ]"#,
            ),
        ),
        (d6, gh_ok("[]")),
    ]);

    let sources = vec![
        serde_json::json!({ "type": "github", "issueRepo": "example/team-app" }),
        serde_json::json!({ "type": "github-project", "projectOwner": "example", "projectNumber": 1 }),
    ];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(false, vec![])).unwrap();
    assert_eq!(col.rows.len(), 0);
    assert_eq!(col.left_out.len(), 1);
    assert_eq!(col.left_out[0].repo, "example/other");
    assert_eq!(col.left_out[0].count, 2); // issue 1 and 2, counted once each across lists
}

// 9. jira / linear sources -> agentSources holds them unchanged, no gh call for them.
#[test]
fn test_09_dashboard_agent_sources() {
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";
    let fake = FakeGh::new(vec![(d6, gh_ok("[]"))]);

    let sources = vec![
        serde_json::json!({ "type": "jira", "jira": { "project": "ABC" } }),
        serde_json::json!({ "type": "linear", "linear": { "team": "XYZ" } }),
    ];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(false, vec![])).unwrap();
    assert_eq!(col.agent_sources.as_ref().unwrap().len(), 2);
    assert_eq!(col.agent_sources.as_ref().unwrap()[0]["type"], "jira");
    assert_eq!(col.agent_sources.as_ref().unwrap()[1]["type"], "linear");
    // Only d6 was called
    assert_eq!(fake.calls.lock().unwrap().len(), 1);
}

// 10. Own PRs: draft and ci (failing / pending / passing / -); D6 failure -> no pr
// rows and Failed { what: "own PRs" }.
#[test]
fn test_10_dashboard_own_prs_and_ci() {
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";

    let completed = "COMPLETED";
    let failure_conclusion = "FAILURE";
    let in_progress = "IN_PROGRESS";
    let success_conclusion = "SUCCESS";

    let pr_json = format!(
        r#"[
            {{"number":1,"title":"PR1","url":"https://example.com/1","isDraft":true,"statusCheckRollup":[
                {{"__typename":"CheckRun","status":"{completed}","conclusion":"{failure_conclusion}"}}
            ]}},
            {{"number":2,"title":"PR2","url":"https://example.com/2","isDraft":false,"statusCheckRollup":[
                {{"__typename":"CheckRun","status":"{in_progress}","conclusion":null}}
            ]}},
            {{"number":3,"title":"PR3","url":"https://example.com/3","isDraft":false,"statusCheckRollup":[
                {{"__typename":"CheckRun","status":"{completed}","conclusion":"{success_conclusion}"}}
            ]}},
            {{"number":4,"title":"PR4","url":"https://example.com/4","isDraft":false,"statusCheckRollup":[]}},
            {{"number":5,"title":"PR5","url":"https://example.com/5","isDraft":false,"statusCheckRollup":[
                {{"something":"unknown"}}
            ]}}
        ]"#
    );

    let fake = FakeGh::new(vec![(d6, gh_ok(pr_json))]);
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &[],
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(false, vec![])).unwrap();
    assert_eq!(col.rows.len(), 5);
    if let Row::Pr(pr) = &col.rows[0] {
        assert_eq!(pr.draft, "yes");
        assert_eq!(pr.ci, "failing");
    } else {
        panic!();
    }
    if let Row::Pr(pr) = &col.rows[1] {
        assert_eq!(pr.draft, "no");
        assert_eq!(pr.ci, "pending");
    } else {
        panic!();
    }
    if let Row::Pr(pr) = &col.rows[2] {
        assert_eq!(pr.draft, "no");
        assert_eq!(pr.ci, "passing");
    } else {
        panic!();
    }
    if let Row::Pr(pr) = &col.rows[3] {
        assert_eq!(pr.draft, "no");
        assert_eq!(pr.ci, "-");
    } else {
        panic!();
    }
    if let Row::Pr(pr) = &col.rows[4] {
        assert_eq!(pr.draft, "no");
        assert_eq!(pr.ci, "?");
    } else {
        panic!();
    }

    // D6 failure
    let fake_fail = FakeGh::new(vec![(d6, gh_err("PR list error"))]);
    let col_fail =
        collect_with(&input, &fake_fail.runner(), &fake_worktrees(false, vec![])).unwrap();
    assert_eq!(col_fail.rows.len(), 0);
    assert_eq!(col_fail.failed.len(), 1);
    assert_eq!(col_fail.failed[0].what, "own PRs");
}

// 11. Worktree listing failure -> no worktree rows, Failed { what: "worktrees" }.
#[test]
fn test_11_dashboard_worktree_failure() {
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";
    let fake = FakeGh::new(vec![(d6, gh_ok("[]"))]);
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &[],
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(
        &input,
        &fake.runner(),
        &fake_worktrees_fail(false, "git error"),
    )
    .unwrap();
    assert_eq!(col.rows.len(), 0);
    assert_eq!(col.failed.len(), 1);
    assert_eq!(col.failed[0].what, "worktrees");
    assert_eq!(col.failed[0].lookup, "worktree listing");
}

// 12. A title holding |, " and a newline serialises as one JSON line and round-trips; key
// order of each row kind is exactly as specified.
#[test]
fn test_12_row_serialisation_and_key_order() {
    let special_title = "Title with | and \"quotes\" and \n newline";
    let task = TaskRow {
        kind: "task".to_string(),
        identifier: "example/team-app#1".to_string(),
        key: "ALPHA-1".to_string(),
        item: "item-1".to_string(),
        status: "open".to_string(),
        title: special_title.to_string(),
        url: "https://example.com/1".to_string(),
        assignee: "@me".to_string(),
    };
    let json_line = serde_json::to_string(&task).unwrap();
    assert!(!json_line.contains('\n'));
    let round_trip: Value = serde_json::from_str(&json_line).unwrap();
    assert_eq!(round_trip["title"], special_title);

    // Verify key order
    let expected_keys = [
        "kind",
        "identifier",
        "key",
        "item",
        "status",
        "title",
        "url",
        "assignee",
    ];
    let mut pos = 0;
    for k in expected_keys {
        let needle = format!("\"{k}\":");
        let found = json_line[pos..].find(&needle).expect(k);
        pos += found + needle.len();
    }
}

// 13. A source missing issueRepo -> Failed { lookup: "config" }, no gh call.
#[test]
fn test_13_dashboard_missing_issue_repo() {
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";
    let fake = FakeGh::new(vec![(d6, gh_ok("[]"))]);
    let sources = vec![serde_json::json!({
        "type": "github"
        // missing issueRepo
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(false, vec![])).unwrap();
    assert_eq!(col.failed.len(), 1);
    assert_eq!(col.failed[0].lookup, "config");
    assert_eq!(col.failed[0].error, "missing issueRepo");
}

// 14. Happy path, github source: parent row first (title/url from P1), subtasks in returned order,
// assignee joined with , / -, closed child status closed, labels otherwise; exact
// P1/P2/P3/P5 vectors; me set.
#[test]
fn test_14_parent_happy_path_github_source() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let p5_1 = "pr list -R example/android-app --head alice/ALPHA-11 --state all --limit 50 --json number,title,url,state,isDraft";
    let p5_2 = "pr list -R example/android-app --head alice/ALPHA-12 --state all --limit 50 --json number,title,url,state,isDraft";

    let open_state = "OPEN";
    let closed_state = "CLOSED";

    let p3_lines = format!(
        r#"{{"number":11,"title":"Child 1","state":"{closed_state}","html_url":"https://example.com/11","node_id":"n11","repo":"example/team-app","labels":["bug"],"assignees":["alice","bob"]}}
{{"number":12,"title":"Child 2","state":"{open_state}","html_url":"https://example.com/12","node_id":"n12","repo":"example/team-app","labels":["enhancement"],"assignees":[]}}"#
    );

    let pr_state_open = "OPEN";
    let fake = FakeGh::new(vec![
        (
            p1,
            gh_ok(
                r#"{"title":"Parent Issue","html_url":"https://example.com/10","node_id":"np10"}"#,
            ),
        ),
        (p2, gh_ok("alice\n")),
        (&p3, gh_ok(p3_lines)),
        (
            p5_1,
            gh_ok(format!(
                r#"[{{"number":101,"title":"PR101","url":"https://example.com/pr/101","state":"{pr_state_open}","isDraft":false}}]"#
            )),
        ),
        (p5_2, gh_ok("[]")),
        (
            "pr list -R example/android-app --search ALPHA-12 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName",
            gh_ok("[]"),
        ),
    ]);

    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![])).unwrap();
    assert_eq!(col.mode, "parent");
    assert_eq!(col.me.as_deref(), Some("alice"));
    assert_eq!(col.rows.len(), 3); // parent + 2 subtasks

    match &col.rows[0] {
        Row::Parent(p) => {
            assert_eq!(p.key, "ALPHA-10");
            assert_eq!(p.title, "Parent Issue");
            assert_eq!(p.url, "https://example.com/10");
        }
        _ => panic!(),
    }

    match &col.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.key, "ALPHA-11");
            assert_eq!(s.assignee, "alice,bob");
            assert_eq!(s.status, "closed");
            assert_eq!(s.pr, "101");
            assert_eq!(s.pr_state, "OPEN");
        }
        _ => panic!(),
    }

    match &col.rows[2] {
        Row::Subtask(s) => {
            assert_eq!(s.key, "ALPHA-12");
            assert_eq!(s.assignee, "-");
            assert_eq!(s.status, "enhancement");
            assert_eq!(s.pr, "-");
            assert_eq!(s.pr_state, "-");
        }
        _ => panic!(),
    }

    let calls = fake.calls.lock().unwrap();
    assert_eq!(calls[0], p1);
    assert_eq!(calls[1], p2);
    assert_eq!(calls[2], p3);
    let mut pr_calls = calls[3..].to_vec();
    pr_calls.sort();
    let mut expected_pr_calls = vec![
        p5_1.to_string(),
        p5_2.to_string(),
        "pr list -R example/android-app --search ALPHA-12 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName".to_string(),
    ];
    expected_pr_calls.sort();
    assert_eq!(pr_calls, expected_pr_calls);
}

// 15. github-project chosen: nodes(ids:) for the children; nodes failure -> ? in
// status/item, except a closed child keeps closed.
#[test]
fn test_15_parent_github_project_nodes_failure() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let nodes_call = "api graphql -H Content-Type: application/json --input -";
    let p5_1 = "pr list -R example/android-app --head alice/ALPHA-11 --state all --limit 50 --json number,title,url,state,isDraft";
    let p5_2 = "pr list -R example/android-app --head alice/ALPHA-12 --state all --limit 50 --json number,title,url,state,isDraft";

    let open_state = "OPEN";
    let closed_state = "CLOSED";

    let p3_lines = format!(
        r#"{{"number":11,"title":"Child 1","state":"{closed_state}","html_url":"https://example.com/11","node_id":"n11","repo":"example/team-app","labels":[],"assignees":[]}}
{{"number":12,"title":"Child 2","state":"{open_state}","html_url":"https://example.com/12","node_id":"n12","repo":"example/team-app","labels":[],"assignees":[]}}"#
    );

    let fake = FakeGh::new(vec![
        (
            p1,
            gh_ok(r#"{"title":"Parent","html_url":"https://example.com/10","node_id":"np10"}"#),
        ),
        (p2, gh_ok("alice")),
        (&p3, gh_ok(p3_lines)),
        (nodes_call, gh_err("nodes error")),
        (p5_1, gh_ok("[]")),
        (
            "pr list -R example/android-app --search ALPHA-11 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName",
            gh_ok("[]"),
        ),
        (p5_2, gh_ok("[]")),
        (
            "pr list -R example/android-app --search ALPHA-12 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName",
            gh_ok("[]"),
        ),
    ]);

    let sources = vec![serde_json::json!({
        "type": "github-project",
        "projectOwner": "example",
        "projectNumber": 1
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![])).unwrap();
    match &col.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.key, "ALPHA-11");
            assert_eq!(s.status, "closed");
            assert_eq!(s.item, "?");
        }
        _ => panic!(),
    }
    match &col.rows[2] {
        Row::Subtask(s) => {
            assert_eq!(s.key, "ALPHA-12");
            assert_eq!(s.status, "?");
            assert_eq!(s.item, "?");
        }
        _ => panic!(),
    }
}

// 16. Fatal: P1 fails / P2 fails / P3 non-zero / P3 a line that does not parse -> Err each,
// and (for P3) the error names the sub-issues.
#[test]
fn test_16_parent_fatal_errors() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );

    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    // P1 fails
    let fake_p1 = FakeGh::new(vec![(p1, gh_err("P1 HTTP 404"))]);
    let err = collect_with(&input, &fake_p1.runner(), &fake_worktrees(false, vec![])).unwrap_err();
    assert!(err.contains("cannot read the parent ALPHA-10"));

    // P2 fails
    let fake_p2 = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_err("P2 unauth")),
    ]);
    let err = collect_with(&input, &fake_p2.runner(), &fake_worktrees(false, vec![])).unwrap_err();
    assert!(err.contains("cannot tell who you are"));

    // P3 non-zero
    let fake_p3 = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (&p3, gh_err("P3 failed")),
    ]);
    let err = collect_with(&input, &fake_p3.runner(), &fake_worktrees(false, vec![])).unwrap_err();
    assert!(err.contains("cannot read the sub-issues of ALPHA-10"));

    // P3 parse fail
    let fake_p3_parse = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (&p3, gh_ok("not-json-line\n")),
    ]);
    let err = collect_with(
        &input,
        &fake_p3_parse.runner(),
        &fake_worktrees(false, vec![]),
    )
    .unwrap_err();
    assert!(err.contains("cannot read the sub-issues of ALPHA-10"));
}

// 17. Child from a repo without a key -> leftOut, no row, no P5 for it.
#[test]
fn test_17_parent_child_without_key() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );

    let open_state = "OPEN";
    let p3_lines = format!(
        r#"{{"number":99,"title":"Other","state":"{open_state}","html_url":"https://example.com/99","node_id":"n99","repo":"example/other","labels":[],"assignees":[]}}"#
    );

    let fake = FakeGh::new(vec![
        (
            p1,
            gh_ok(r#"{"title":"Parent","html_url":"https://example.com/10","node_id":"nid"}"#),
        ),
        (p2, gh_ok("alice")),
        (&p3, gh_ok(p3_lines)),
    ]);

    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![])).unwrap();
    // Only parent row, no child row
    assert_eq!(col.rows.len(), 1);
    assert_eq!(col.left_out.len(), 1);
    assert_eq!(col.left_out[0].repo, "example/other");
    assert_eq!(col.left_out[0].count, 1);
}

// 18. Branch: proctor present -> {user}/ALPHA-7 is primary; absent -> {user}/alpha-7; a
// worktree on the other spelling still matches; ALPHA-1 does not match a worktree on
// alice/ALPHA-10; a source branchPattern (feature/{name}) is used.
#[test]
fn test_18_parent_branch_resolution_and_matching() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let open_state = "OPEN";

    // Case A: proctor present -> alice/ALPHA-7 is primary, matches alice/alpha-7 worktree
    let p3_lines = format!(
        r#"{{"number":7,"title":"Child 7","state":"{open_state}","html_url":"https://example.com/7","node_id":"n7","repo":"example/team-app","labels":[],"assignees":[]}}"#
    );
    let p5 = "pr list -R example/android-app --head alice/alpha-7 --state all --limit 50 --json number,title,url,state,isDraft";
    let fake_a = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (&p3, gh_ok(p3_lines)),
        (p5, gh_ok("[]")),
        (
            "pr list -R example/android-app --search ALPHA-7 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName",
            gh_ok("[]"),
        ),
    ]);
    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let lister_a = fake_worktrees(true, vec![("/path/wt", Some("alice/alpha-7"))]);
    let col_a = collect_with(&input, &fake_a.runner(), &lister_a).unwrap();
    match &col_a.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.branch, "alice/alpha-7");
            assert_eq!(s.worktree, "/path/wt");
        }
        _ => panic!(),
    }

    // Case B: ALPHA-1 does not match alice/ALPHA-10
    let p3_lines_b = format!(
        r#"{{"number":1,"title":"Child 1","state":"{open_state}","html_url":"https://example.com/1","node_id":"n1","repo":"example/team-app","labels":[],"assignees":[]}}"#
    );
    let p5_b = "pr list -R example/android-app --head alice/ALPHA-1 --state all --limit 50 --json number,title,url,state,isDraft";
    let fake_b = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (&p3, gh_ok(p3_lines_b)),
        (p5_b, gh_ok("[]")),
        (
            "pr list -R example/android-app --search ALPHA-1 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName",
            gh_ok("[]"),
        ),
    ]);
    let lister_b = fake_worktrees(true, vec![("/path/wt10", Some("alice/ALPHA-10"))]);
    let col_b = collect_with(&input, &fake_b.runner(), &lister_b).unwrap();
    match &col_b.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.branch, "alice/ALPHA-1");
            assert_eq!(s.worktree, "-");
        }
        _ => panic!(),
    }

    // Case C: source branchPattern is used
    let sources_c = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app",
        "branchPattern": "feature/{name}"
    })];
    let p5_c = "pr list -R example/android-app --head feature/ALPHA-7 --state all --limit 50 --json number,title,url,state,isDraft";
    let fake_c = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":7,"title":"Child 7","state":"{open_state}","html_url":"https://example.com/7","node_id":"n7","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5_c, gh_ok("[]")),
        (
            "pr list -R example/android-app --search ALPHA-7 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName",
            gh_ok("[]"),
        ),
    ]);
    let input_c = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources_c,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let col_c = collect_with(&input_c, &fake_c.runner(), &fake_worktrees(true, vec![])).unwrap();
    match &col_c.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.branch, "feature/ALPHA-7");
        }
        _ => panic!(),
    }
}

// 19. Worktree listing failure -> worktree ? on every subtask row, one Failed.
#[test]
fn test_19_parent_worktree_listing_failure() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let open_state = "OPEN";
    let p5 = "pr list -R example/android-app --head alice/ALPHA-1 --state all --limit 50 --json number,title,url,state,isDraft";

    let fake = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":1,"title":"C1","state":"{open_state}","html_url":"u1","node_id":"n1","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5, gh_ok("[]")),
        (
            "pr list -R example/android-app --search ALPHA-1 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName",
            gh_ok("[]"),
        ),
    ]);
    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(
        &input,
        &fake.runner(),
        &fake_worktrees_fail(true, "proctor boom"),
    )
    .unwrap();
    match &col.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.worktree, "?");
        }
        _ => panic!(),
    }
    assert_eq!(col.failed.len(), 1);
    assert_eq!(col.failed[0].what, "worktrees");
}

// 20. unmatchedWorktrees lists the worktree that matched no child.
#[test]
fn test_20_parent_unmatched_worktrees() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let open_state = "OPEN";
    let p5 = "pr list -R example/android-app --head alice/ALPHA-1 --state all --limit 50 --json number,title,url,state,isDraft";

    let fake = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":1,"title":"C1","state":"{open_state}","html_url":"u1","node_id":"n1","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5, gh_ok("[]")),
        (
            "pr list -R example/android-app --search ALPHA-1 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName",
            gh_ok("[]"),
        ),
    ]);
    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let lister = fake_worktrees(
        true,
        vec![
            ("/path/wt1", Some("alice/ALPHA-1")),
            ("/path/unmatched", Some("alice/other-branch")),
        ],
    );
    let col = collect_with(&input, &fake.runner(), &lister).unwrap();
    let unmatched = col.unmatched_worktrees.as_ref().unwrap();
    assert_eq!(unmatched.len(), 1);
    assert_eq!(unmatched[0].path, "/path/unmatched");
    assert_eq!(unmatched[0].branch, "alice/other-branch");
}

// 21. PR choice: OPEN beats a newer MERGED; among CLOSED the highest number; prState as printed.
#[test]
fn test_21_parent_pr_choice_ranking() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let open_state = "OPEN";
    let p5 = "pr list -R example/android-app --head alice/ALPHA-1 --state all --limit 50 --json number,title,url,state,isDraft";

    let open_pr = "OPEN";
    let merged_pr = "MERGED";
    let closed_pr = "CLOSED";

    // OPEN (number 10) beats newer MERGED (number 20)
    let fake_prs = format!(
        r#"[
            {{"number":20,"state":"{merged_pr}","title":"M","url":"uM","isDraft":false}},
            {{"number":10,"state":"{open_pr}","title":"O","url":"uO","isDraft":false}}
        ]"#
    );

    let fake = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":1,"title":"C1","state":"{open_state}","html_url":"u1","node_id":"n1","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5, gh_ok(fake_prs)),
    ]);
    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![])).unwrap();
    match &col.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.pr, "10");
            assert_eq!(s.pr_state, "OPEN");
        }
        _ => panic!(),
    }

    // Among CLOSED, highest number
    let fake_closed = format!(
        r#"[
            {{"number":30,"state":"{closed_pr}","title":"C30","url":"uC30","isDraft":false}},
            {{"number":50,"state":"{closed_pr}","title":"C50","url":"uC50","isDraft":false}}
        ]"#
    );
    let fake_c = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":1,"title":"C1","state":"{open_state}","html_url":"u1","node_id":"n1","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5, gh_ok(fake_closed)),
    ]);
    let col_c = collect_with(&input, &fake_c.runner(), &fake_worktrees(true, vec![])).unwrap();
    match &col_c.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.pr, "50");
            assert_eq!(s.pr_state, "CLOSED");
        }
        _ => panic!(),
    }
}

// 22. P5 fails -> ?/? and no P6 call (the fake panics if called); P5 [] and P6 fails -> ?/?;
// both empty -> -/-; P5 [] and P6 finds one -> that PR.
#[test]
fn test_22_parent_p5_and_p6_fallback() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let open_state = "OPEN";
    let p5 = "pr list -R example/android-app --head alice/ALPHA-1 --state all --limit 50 --json number,title,url,state,isDraft";
    let p6 = "pr list -R example/android-app --search ALPHA-1 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName";

    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    // Subtest A: P5 fails -> ?/? and NO P6 call (panic if called since P6 is not in fake)
    let fake_a = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":1,"title":"C1","state":"{open_state}","html_url":"u1","node_id":"n1","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5, gh_err("p5 fail")),
    ]);
    let col_a = collect_with(&input, &fake_a.runner(), &fake_worktrees(true, vec![])).unwrap();
    match &col_a.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.pr, "?");
            assert_eq!(s.pr_state, "?");
        }
        _ => panic!(),
    }

    // Subtest B: P5 [] and P6 fails -> ?/?
    let fake_b = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":1,"title":"C1","state":"{open_state}","html_url":"u1","node_id":"n1","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5, gh_ok("[]")),
        (p6, gh_err("p6 fail")),
    ]);
    let col_b = collect_with(&input, &fake_b.runner(), &fake_worktrees(true, vec![])).unwrap();
    match &col_b.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.pr, "?");
            assert_eq!(s.pr_state, "?");
        }
        _ => panic!(),
    }

    // Subtest C: P5 [] and P6 finds one -> that PR
    let pr_state_open = "OPEN";
    let fake_c = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":1,"title":"C1","state":"{open_state}","html_url":"u1","node_id":"n1","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5, gh_ok("[]")),
        (
            p6,
            gh_ok(format!(
                r#"[{{"number":77,"state":"{pr_state_open}","title":"T77 (ALPHA-1)","url":"u77","isDraft":false,"body":null,"headRefName":"alice/ALPHA-1"}}]"#
            )),
        ),
    ]);
    let col_c = collect_with(&input, &fake_c.runner(), &fake_worktrees(true, vec![])).unwrap();
    match &col_c.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.pr, "77");
            assert_eq!(s.pr_state, "OPEN");
        }
        _ => panic!(),
    }
}

// 23. Source choice errors: key not KEY-number; no match; two issueKeys repos WID/wid;
// github + github-project in one entry; two github-project both with projectFields;
// a jira key (ABC-819 with a jira source project: "ABC") -> Err mentioning the
// collection agent. Two github-project, one with projectFields -> that one (no error).
#[test]
fn test_23_parent_source_choice_errors() {
    let dummy_gh = FakeGh::new(vec![]);
    let keys = sample_keys();

    // Key not KEY-number
    let input_bad_key = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &[],
        parent: Some("badkey"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let err = collect_with(
        &input_bad_key,
        &dummy_gh.runner(),
        &fake_worktrees(false, vec![]),
    )
    .unwrap_err();
    assert!(err.contains("is not a KEY-number task key"));

    // No match
    let input_no_match = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &[],
        parent: Some("BETA-123"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let err = collect_with(
        &input_no_match,
        &dummy_gh.runner(),
        &fake_worktrees(false, vec![]),
    )
    .unwrap_err();
    assert!(err.contains("matches no task source"));

    // Two issueKeys repos WID/wid
    let mut keys_two = serde_json::Map::new();
    keys_two.insert(
        "example/repo-a".to_string(),
        Value::String("WID".to_string()),
    );
    keys_two.insert(
        "example/repo-b".to_string(),
        Value::String("wid".to_string()),
    );
    let input_two_keys = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys_two,
        sources: &[],
        parent: Some("WID-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let err = collect_with(
        &input_two_keys,
        &dummy_gh.runner(),
        &fake_worktrees(false, vec![]),
    )
    .unwrap_err();
    assert!(err.contains("matches several repositories in issueKeys"));

    // github + github-project in one entry
    let sources_gh_and_project = vec![
        serde_json::json!({
            "type": "github",
            "issueRepo": "example/team-app"
        }),
        serde_json::json!({
            "type": "github-project",
            "projectOwner": "example",
            "projectNumber": 1
        }),
    ];
    let input_mix = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources_gh_and_project,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let err = collect_with(
        &input_mix,
        &dummy_gh.runner(),
        &fake_worktrees(false, vec![]),
    )
    .unwrap_err();
    assert!(err.contains("matches several task sources"));

    // Two github-project both with projectFields
    let sources_two_pf = vec![
        serde_json::json!({
            "type": "github-project",
            "projectOwner": "example",
            "projectNumber": 1,
            "projectFields": { "Status": "s1" }
        }),
        serde_json::json!({
            "type": "github-project",
            "projectOwner": "example",
            "projectNumber": 2,
            "projectFields": { "Status": "s2" }
        }),
    ];
    let input_two_pf = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources_two_pf,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let err = collect_with(
        &input_two_pf,
        &dummy_gh.runner(),
        &fake_worktrees(false, vec![]),
    )
    .unwrap_err();
    assert!(err.contains("matches several task sources"));

    // Jira key -> Err mentioning collection agent
    let sources_jira = vec![serde_json::json!({
        "type": "jira",
        "jira": { "project": "ABC" }
    })];
    let input_jira = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources_jira,
        parent: Some("ABC-819"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let err = collect_with(
        &input_jira,
        &dummy_gh.runner(),
        &fake_worktrees(false, vec![]),
    )
    .unwrap_err();
    assert!(err.contains("collect it with the collection agent"));

    // Two github-project, one with projectFields -> that one (no error selecting source)
    let sources_one_pf = vec![
        serde_json::json!({
            "type": "github-project",
            "projectOwner": "example",
            "projectNumber": 1
        }),
        serde_json::json!({
            "type": "github-project",
            "projectOwner": "example",
            "projectNumber": 2,
            "projectFields": { "Status": "s2" }
        }),
    ];
    let input_one_pf = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources_one_pf,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let (chosen, repo, key_spelled, n) = choose_source(&input_one_pf, "ALPHA-10").unwrap();
    assert_eq!(chosen["projectNumber"], 2);
    assert_eq!(repo, "example/team-app");
    assert_eq!(key_spelled, "ALPHA");
    assert_eq!(n, 10);
}

// 24. No sub-issues -> Ok with just the parent row.
#[test]
fn test_24_parent_no_sub_issues() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );

    let fake = FakeGh::new(vec![
        (
            p1,
            gh_ok(
                r#"{"title":"Empty Parent","html_url":"https://example.com/10","node_id":"nid"}"#,
            ),
        ),
        (p2, gh_ok("alice")),
        (&p3, gh_ok("")),
    ]);

    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![])).unwrap();
    assert_eq!(col.rows.len(), 1);
    match &col.rows[0] {
        Row::Parent(p) => {
            assert_eq!(p.title, "Empty Parent");
        }
        _ => panic!(),
    }
}

// 25. parse_proctor_worktrees: drops isMain: true (even when it is not the first), strips
// refs/heads/, empty branch -> None, garbage -> Err.
#[test]
fn test_25_parse_proctor_worktrees() {
    let json_sample = serde_json::json!([
        {
            "repository": "repo1",
            "worktrees": [
                { "path": "/path/wt1", "branch": "refs/heads/feature-1", "isMain": false },
                { "path": "/path/main", "branch": "refs/heads/main", "isMain": true },
                { "path": "/path/wt2", "branch": "", "isMain": false },
                { "path": "/path/wt3", "branch": "direct-branch", "isMain": false }
            ]
        }
    ]);
    let wts = parse_proctor_worktrees("/path/main", &json_sample.to_string()).unwrap();
    assert_eq!(wts.len(), 3);
    assert_eq!(wts[0].path, "/path/wt1");
    assert_eq!(wts[0].branch.as_deref(), Some("feature-1"));

    assert_eq!(wts[1].path, "/path/wt2");
    assert_eq!(wts[1].branch, None);

    assert_eq!(wts[2].path, "/path/wt3");
    assert_eq!(wts[2].branch.as_deref(), Some("direct-branch"));

    // Garbage -> Err
    assert!(parse_proctor_worktrees("/path/main", "not-json").is_err());
    assert!(parse_proctor_worktrees("/path/main", r#"{"not":"array"}"#).is_err());
    assert!(
        parse_proctor_worktrees("/path/main", r#"[{"worktrees":[{"no_path":true}]}]"#).is_err()
    );
}

// Parent search fallback key token matching:
// (a) search returns a PR that does not mention the key -> pr "-"
// (b) search returns one naming the key in the body ("Closes ALPHA-7" style) -> that PR
// (c) ALPHA-1 does not match a PR mentioning only ALPHA-10
#[test]
fn test_parent_search_pr_token_matching_unrelated_ignored() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let open_state = "OPEN";
    let merged_state = "MERGED";
    let p5 = "pr list -R example/android-app --head alice/ALPHA-1 --state all --limit 50 --json number,title,url,state,isDraft";
    let p6 = "pr list -R example/android-app --search ALPHA-1 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName";

    // (a) Search returned an unrelated PR (e.g. #498 on branch syarihu/ALPHA-496 that never mentions ALPHA-1)
    let fake_search = format!(
        r#"[{{"number":498,"state":"{merged_state}","title":"Fix unrelated component","url":"https://example.com/pr/498","isDraft":false,"body":"Fixes issue 496","headRefName":"alice/ALPHA-496"}}]"#
    );

    let fake = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":1,"title":"C1","state":"{open_state}","html_url":"u1","node_id":"n1","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5, gh_ok("[]")),
        (p6, gh_ok(fake_search)),
    ]);
    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![])).unwrap();
    match &col.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.pr, "-");
            assert_eq!(s.pr_state, "-");
        }
        _ => panic!(),
    }
}

#[test]
fn test_parent_search_pr_token_matching_body_match() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let open_state = "OPEN";
    let p5 = "pr list -R example/android-app --head alice/ALPHA-7 --state all --limit 50 --json number,title,url,state,isDraft";
    let p6 = "pr list -R example/android-app --search ALPHA-7 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName";

    // (b) Search returns one naming the key in the body ("Closes ALPHA-7" style)
    let fake_search = format!(
        r#"[{{"number":70,"state":"{open_state}","title":"Implement feature","url":"https://example.com/pr/70","isDraft":false,"body":"Closes ALPHA-7\nSome details","headRefName":"alice/random-branch"}}]"#
    );

    let fake = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":7,"title":"C7","state":"{open_state}","html_url":"u7","node_id":"n7","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5, gh_ok("[]")),
        (p6, gh_ok(fake_search)),
    ]);
    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![])).unwrap();
    match &col.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.pr, "70");
            assert_eq!(s.pr_state, "OPEN");
        }
        _ => panic!(),
    }
}

#[test]
fn test_parent_search_pr_token_matching_digit_boundary() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let open_state = "OPEN";
    let p5 = "pr list -R example/android-app --head alice/ALPHA-1 --state all --limit 50 --json number,title,url,state,isDraft";
    let p6 = "pr list -R example/android-app --search ALPHA-1 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName";

    // (c) ALPHA-1 does not match a PR mentioning only ALPHA-10
    let fake_search = format!(
        r#"[{{"number":15,"state":"{open_state}","title":"ALPHA-10 title","url":"https://example.com/pr/15","isDraft":false,"body":"Mentions ALPHA-10 only","headRefName":"alice/ALPHA-10"}}]"#
    );

    let fake = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":1,"title":"C1","state":"{open_state}","html_url":"u1","node_id":"n1","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5, gh_ok("[]")),
        (p6, gh_ok(fake_search)),
    ]);
    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![])).unwrap();
    match &col.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.pr, "-");
            assert_eq!(s.pr_state, "-");
        }
        _ => panic!(),
    }
}

#[test]
fn test_parent_github_project_source_without_project_number_returns_err() {
    let fake = FakeGh::new(vec![]);
    let sources = vec![serde_json::json!({
        "type": "github-project",
        "projectOwner": "example"
        // no projectNumber
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let res = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![]));
    assert!(res.is_err());
    assert_eq!(
        res.unwrap_err(),
        "ALPHA-10's task source (github-project) has no projectNumber in the config"
    );
    assert!(fake.calls.lock().unwrap().is_empty());
}

#[test]
fn test_ci_of_unrecognised_item_and_precedence() {
    let completed = "COMPLETED";
    let failure = "FAILURE";
    let success = "SUCCESS";
    let in_progress = "IN_PROGRESS";

    // 1. Unrecognised item alone -> "?"
    let unrecognised = serde_json::json!([
        { "something": "unknown" }
    ]);
    assert_eq!(ci_of(Some(&unrecognised)), "?");

    // 2. Unrecognised item with none of __typename / status / conclusion / state that this code recognises
    let unrecognised_type = serde_json::json!([
        { "__typename": "UnknownContext", "status": completed }
    ]);
    assert_eq!(ci_of(Some(&unrecognised_type)), "?");

    // 3. Failing still wins over unknown
    let failing_and_unknown = serde_json::json!([
        { "something": "unknown" },
        { "__typename": "CheckRun", "status": completed, "conclusion": failure }
    ]);
    assert_eq!(ci_of(Some(&failing_and_unknown)), "failing");

    // 4. Unknown wins over pending
    let pending_and_unknown = serde_json::json!([
        { "something": "unknown" },
        { "__typename": "CheckRun", "status": in_progress, "conclusion": null }
    ]);
    assert_eq!(ci_of(Some(&pending_and_unknown)), "?");

    // 5. Unknown wins over passing
    let passing_and_unknown = serde_json::json!([
        { "something": "unknown" },
        { "__typename": "CheckRun", "status": completed, "conclusion": success }
    ]);
    assert_eq!(ci_of(Some(&passing_and_unknown)), "?");
}

// Item 6: Parent branch resolution when proctor is absent -> alice/alpha-7 primary, worktree "-"
#[test]
fn test_parent_branch_and_worktree_without_proctor() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let open_state = "OPEN";
    let p3_lines = format!(
        r#"{{"number":7,"title":"Child 7","state":"{open_state}","html_url":"https://example.com/7","node_id":"n7","repo":"example/team-app","labels":[],"assignees":[]}}"#
    );
    let p5 = "pr list -R example/android-app --head alice/alpha-7 --state all --limit 50 --json number,title,url,state,isDraft";
    let p6 = "pr list -R example/android-app --search ALPHA-7 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName";

    let fake = FakeGh::new(vec![
        (
            p1,
            gh_ok(r#"{"title":"Parent","html_url":"u","node_id":"nid"}"#),
        ),
        (p2, gh_ok("alice")),
        (&p3, gh_ok(p3_lines)),
        (p5, gh_ok("[]")),
        (p6, gh_ok("[]")),
    ]);
    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(false, vec![])).unwrap();
    match &col.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.branch, "alice/alpha-7");
            assert_eq!(s.worktree, "-");
        }
        _ => panic!("expected subtask row"),
    }
}

// Item 1: Proctor timeout handling
#[test]
fn test_proctor_worktree_ls_timeout_recorded_as_failed() {
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &[],
        parent: None,
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let d6 = "pr list -R example/android-app --author @me --state open --json number,title,url,isDraft,statusCheckRollup --limit 50";
    let fake = FakeGh::new(vec![(d6, gh_ok("[]"))]);
    let col = collect_with(
        &input,
        &fake.runner(),
        &fake_worktrees_fail(true, "proctor worktree ls --json did not finish in time"),
    )
    .unwrap();
    assert_eq!(col.failed.len(), 1);
    assert_eq!(col.failed[0].what, "worktrees");
    assert_eq!(col.failed[0].lookup, "worktree listing");
    assert_eq!(
        col.failed[0].error,
        "proctor worktree ls --json did not finish in time"
    );
}

// Item 2: parse_proctor_worktrees matching groups
#[test]
fn test_parse_proctor_worktrees_group_matching() {
    let json_multiple = serde_json::json!([
        {
            "repo": "/repo/one",
            "worktrees": [
                { "path": "/repo/one/wt", "branch": "refs/heads/branch-1", "isMain": false },
                { "path": "/repo/one/main", "branch": "refs/heads/main", "isMain": true }
            ]
        },
        {
            "repo": "/repo/two",
            "worktrees": [
                { "path": "/repo/two/wt", "branch": "refs/heads/branch-2", "isMain": false }
            ]
        }
    ]);

    // Matching repo 1
    let wts_one = parse_proctor_worktrees("/repo/one", &json_multiple.to_string()).unwrap();
    assert_eq!(wts_one.len(), 1);
    assert_eq!(wts_one[0].path, "/repo/one/wt");
    assert_eq!(wts_one[0].branch.as_deref(), Some("branch-1"));

    // Matching repo 2
    let wts_two = parse_proctor_worktrees("/repo/two", &json_multiple.to_string()).unwrap();
    assert_eq!(wts_two.len(), 1);
    assert_eq!(wts_two[0].path, "/repo/two/wt");
    assert_eq!(wts_two[0].branch.as_deref(), Some("branch-2"));

    // No matching group
    let err = parse_proctor_worktrees("/repo/three", &json_multiple.to_string()).unwrap_err();
    assert_eq!(err, "proctor's answer has no group for /repo/three");

    // Empty array
    let err_empty = parse_proctor_worktrees("/repo/any", "[]").unwrap_err();
    assert_eq!(err_empty, "proctor's answer has no group for /repo/any");
}

// Item 3a & 3b: Node project item requires matching both number and owner
#[test]
fn test_project_items_requires_matching_owner_and_number() {
    let node_sample = serde_json::json!({
        "projectItems": {
            "pageInfo": { "hasNextPage": false },
            "nodes": [
                {
                    "id": "item-diff-owner",
                    "project": { "number": 1, "owner": { "login": "wrong-owner" } },
                    "fieldValueByName": { "name": "Todo" }
                },
                {
                    "id": "item-match",
                    "project": { "number": 1, "owner": { "login": "example" } },
                    "fieldValueByName": { "name": "In Progress" }
                }
            ]
        }
    });

    let (item_id, status, err) =
        extract_project_item_and_status(&node_sample, &serde_json::json!(1), "example", None);
    assert_eq!(item_id, "item-match");
    assert_eq!(status, "In Progress");
    assert!(err.is_none());

    // When owner does not match any item
    let (item_id, status, err) =
        extract_project_item_and_status(&node_sample, &serde_json::json!(1), "other", None);
    assert_eq!(item_id, "no-item");
    assert_eq!(status, "N/A");
    assert!(err.is_none());
}

// Item 3b: Parent mode requires non-empty projectOwner in config
#[test]
fn test_parent_github_project_source_without_project_owner_returns_err() {
    let fake = FakeGh::new(vec![]);
    let sources = vec![serde_json::json!({
        "type": "github-project",
        "projectNumber": 1
        // missing projectOwner
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let res = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![]));
    assert!(res.is_err());
    assert_eq!(
        res.unwrap_err(),
        "ALPHA-10's task source (github-project) has no projectOwner in the config"
    );
    assert!(fake.calls.lock().unwrap().is_empty());
}

// Item 3c: projectItems hasNextPage returns ? and Failed
#[test]
fn test_project_items_has_next_page_returns_failed() {
    let node_sample = serde_json::json!({
        "projectItems": {
            "pageInfo": { "hasNextPage": true },
            "nodes": [
                {
                    "id": "item-other",
                    "project": { "number": 99, "owner": { "login": "example" } },
                    "fieldValueByName": { "name": "Done" }
                }
            ]
        }
    });

    let (item_id, status, err) =
        extract_project_item_and_status(&node_sample, &serde_json::json!(1), "example", None);
    assert_eq!(item_id, "?");
    assert_eq!(status, "?");
    assert_eq!(err.as_deref(), Some("projectItems has more than 50 items"));
}

// Item 3d: projectItems null with GraphQL errors returns ? and Failed
#[test]
fn test_project_items_null_with_graphql_error_returns_failed() {
    let node_sample = serde_json::json!({
        "projectItems": null
    });

    let (item_id, status, err) = extract_project_item_and_status(
        &node_sample,
        &serde_json::json!(1),
        "example",
        Some("Could not resolve to a node"),
    );
    assert_eq!(item_id, "?");
    assert_eq!(status, "?");
    assert_eq!(err.as_deref(), Some("Could not resolve to a node"));
}

// Item 4: Parent mode validates projectNumber as number or digit string
#[test]
fn test_parent_github_project_source_invalid_project_number_returns_err() {
    let fake = FakeGh::new(vec![]);
    let keys = sample_keys();

    // String with non-digits
    let sources_non_digits = vec![serde_json::json!({
        "type": "github-project",
        "projectOwner": "example",
        "projectNumber": "abc"
    })];
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources_non_digits,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let err = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![])).unwrap_err();
    assert_eq!(
        err,
        "ALPHA-10's task source (github-project) has invalid projectNumber in the config"
    );

    // Empty string
    let sources_empty = vec![serde_json::json!({
        "type": "github-project",
        "projectOwner": "example",
        "projectNumber": ""
    })];
    let input_empty = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources_empty,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let err_empty =
        collect_with(&input_empty, &fake.runner(), &fake_worktrees(true, vec![])).unwrap_err();
    assert_eq!(
        err_empty,
        "ALPHA-10's task source (github-project) has invalid projectNumber in the config"
    );

    // Boolean
    let sources_bool = vec![serde_json::json!({
        "type": "github-project",
        "projectOwner": "example",
        "projectNumber": true
    })];
    let input_bool = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources_bool,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let err_bool =
        collect_with(&input_bool, &fake.runner(), &fake_worktrees(true, vec![])).unwrap_err();
    assert_eq!(
        err_bool,
        "ALPHA-10's task source (github-project) has invalid projectNumber in the config"
    );
    assert!(fake.calls.lock().unwrap().is_empty());
}

// Item 5: P6 search returning 100 results with none matching records Failed and ?/?
#[test]
fn test_parent_pr_search_fallback_100_results_unmatched_records_failed() {
    let p1 = "api repos/example/team-app/issues/10 --jq {title, html_url, node_id}";
    let p2 = "api user --jq .login";
    let p3 = format!(
        "api --paginate repos/example/team-app/issues/10/sub_issues --jq {}",
        SUB_ISSUES_JQ
    );
    let open_state = "OPEN";
    let p5 = "pr list -R example/android-app --head alice/ALPHA-1 --state all --limit 50 --json number,title,url,state,isDraft";
    let p6 = "pr list -R example/android-app --search ALPHA-1 --state all --limit 100 --json number,title,url,state,isDraft,body,headRefName";

    let pr_state_open = "OPEN";
    let mut fake_prs = Vec::new();
    for i in 1..=100 {
        fake_prs.push(serde_json::json!({
            "number": i + 1000,
            "state": pr_state_open,
            "title": format!("Unrelated PR {i}"),
            "url": format!("https://example.com/{i}"),
            "isDraft": false,
            "body": "No match here",
            "headRefName": "main"
        }));
    }

    let fake = FakeGh::new(vec![
        (p1, gh_ok(r#"{"title":"P","html_url":"u","node_id":"nid"}"#)),
        (p2, gh_ok("alice")),
        (
            &p3,
            gh_ok(format!(
                r#"{{"number":1,"title":"C1","state":"{open_state}","html_url":"u1","node_id":"n1","repo":"example/team-app","labels":[],"assignees":[]}}"#
            )),
        ),
        (p5, gh_ok("[]")),
        (p6, gh_ok(serde_json::to_string(&fake_prs).unwrap())),
    ]);
    let sources = vec![serde_json::json!({
        "type": "github",
        "issueRepo": "example/team-app"
    })];
    let keys = sample_keys();
    let input = CollectInput {
        code_repo: "example/android-app",
        issue_keys: &keys,
        sources: &sources,
        parent: Some("ALPHA-10"),
        taken_at: "2026-11-07T00:00:00Z".to_string(),
    };

    let col = collect_with(&input, &fake.runner(), &fake_worktrees(true, vec![])).unwrap();
    match &col.rows[1] {
        Row::Subtask(s) => {
            assert_eq!(s.pr, "?");
            assert_eq!(s.pr_state, "?");
        }
        _ => panic!(),
    }
    assert_eq!(col.failed.len(), 1);
    assert_eq!(col.failed[0].what, "ALPHA-1");
    assert_eq!(col.failed[0].lookup, "pr search");
    assert_eq!(
        col.failed[0].error,
        "the PR search for ALPHA-1 returned 100 results and none named it"
    );
}
