use std::time::SystemTime;

use serde_json::json;

use super::store::{archive, claim_id, dir_of, exists, list_modified_since, load, path_of, save};
use super::*;

use crate::registry::Context;

/// A hub's context in a sandboxed state directory. Hold the sandbox for the whole test.
fn hub() -> (crate::testing::Sandbox, Context) {
    let sandbox = crate::testing::Sandbox::empty();
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = crate::registry::context_at(repo, sandbox.state()).unwrap();
    (sandbox, ctx)
}

fn ids(gates: Vec<Gate>) -> Vec<String> {
    gates.into_iter().map(|g| g.id).collect()
}

fn gate(kind: Kind) -> Gate {
    Gate {
        id: "20260922T041233Z-plan".to_string(),
        kind,
        worktree: "/tmp/wt".to_string(),
        opened_by: Opener::Worker,
        task: Some("20260922T041000Z-cache".to_string()),
        title: "Design review: caching search results".to_string(),
        facts: vec!["6 files to touch".to_string()],
        focus: Some("Please decide how the TTL is held".to_string()),
        decided: Some("An LRU of 64 entries".to_string()),
        unsure: None,
        body: None,
        run: None,
        diff: None,
        choices: vec![Choice {
            id: "const".to_string(),
            label: "Option A — a constant".to_string(),
            why: "there is no remote config".to_string(),
            points: vec!["small diff".to_string()],
            recommended: true,
        }],
        options: vec!["approve".to_string(), "changes".to_string()],
        rounds: 0,
        problem: None,
        goal: None,
        review_rounds: Vec::new(),
        findings: Vec::new(),
        commands: Vec::new(),
        manual: Vec::new(),
        stopped_by: Vec::new(),
        wait: true,
        opened_at: "20260922T041233Z".to_string(),
        decision: None,
        choice: None,
        comment: None,
        answered_at: None,
        answers: Vec::new(),
        extra: serde_json::Map::new(),
    }
}

#[test]
fn a_key_this_binary_does_not_know_survives_a_load_save_and_archive() {
    let (_sandbox, ctx) = hub();
    let gate = gate(Kind::Plan);
    let open = dir_of(&ctx, Shelf::Open);
    std::fs::create_dir_all(&open).unwrap();
    let mut raw = serde_json::to_value(&gate).unwrap();
    raw["futureField"] = serde_json::json!("x");
    std::fs::write(path_of(&open, &gate.id), raw.to_string()).unwrap();

    let loaded = load(&ctx.state, &ctx.repo.slug, Shelf::Open, &gate.id).unwrap();
    save(&ctx, Shelf::Open, &loaded).unwrap();
    let text = std::fs::read_to_string(path_of(&open, &gate.id)).unwrap();
    assert!(text.contains("\"futureField\": \"x\""), "{text}");

    let archived = archive(&ctx, &loaded).unwrap();
    let text = std::fs::read_to_string(archived).unwrap();
    assert!(text.contains("\"futureField\": \"x\""), "{text}");
}

#[test]
fn only_a_decision_made_on_the_board_counts_as_answered_there() {
    let mut g = gate(Kind::Plan);
    assert!(!g.answered_on_board());
    for (decision, on_board) in [(CLOSED, false), (TERMINAL, false), ("approve", true)] {
        g.decision = Some(decision.to_string());
        assert_eq!(g.answered_on_board(), on_board, "{decision}");
    }
}

const STARTED: &str = "20260922T040000Z";

#[test]
fn a_later_phase_shows_the_worker_moved_on() {
    let g = gate(Kind::Question);
    let got = resumed_at(&g, Some(STARTED), Some("20260922T041300Z"), None);
    assert_eq!(got, Some(("20260922T041300Z".to_string(), Signal::Phase)));
    // The same second is not later, and neither is earlier.
    assert_eq!(
        resumed_at(&g, Some(STARTED), Some("20260922T041233Z"), None),
        None
    );
    assert_eq!(
        resumed_at(&g, Some(STARTED), Some("20260922T041000Z"), None),
        None
    );
    assert_eq!(resumed_at(&g, Some(STARTED), None, None), None);
}

#[test]
fn a_later_gate_shows_the_worker_moved_on() {
    let g = gate(Kind::Question);
    let got = resumed_at(&g, Some(STARTED), None, Some("20260922T042000Z"));
    assert_eq!(got, Some(("20260922T042000Z".to_string(), Signal::Gate)));
}

#[test]
fn with_both_signals_the_earlier_one_is_the_time() {
    let g = gate(Kind::Question);
    let got = resumed_at(
        &g,
        Some(STARTED),
        Some("20260922T043000Z"),
        Some("20260922T042000Z"),
    );
    assert_eq!(got, Some(("20260922T042000Z".to_string(), Signal::Gate)));
    let got = resumed_at(
        &g,
        Some(STARTED),
        Some("20260922T041500Z"),
        Some("20260922T042000Z"),
    );
    assert_eq!(got, Some(("20260922T041500Z".to_string(), Signal::Phase)));
}

#[test]
fn a_worker_that_is_not_the_one_that_opened_the_gate_is_not_believed() {
    let g = gate(Kind::Question);
    let later = Some("20260922T050000Z");
    assert_eq!(resumed_at(&g, Some("20260922T041234Z"), later, later), None);
    assert_eq!(resumed_at(&g, None, later, later), None);
    // Started in the same second as the gate: it may be a replacement, so it is not believed.
    assert_eq!(resumed_at(&g, Some("20260922T041233Z"), later, later), None);
    assert!(resumed_at(&g, Some("20260922T041232Z"), later, None).is_some());
}

#[test]
fn a_record_and_the_hub_s_gates_are_never_resumed() {
    let later = Some("20260922T050000Z");
    let mut record = gate(Kind::Verify);
    record.wait = false;
    assert_eq!(resumed_at(&record, Some(STARTED), later, later), None);
    let dispatch = gate(Kind::Dispatch);
    assert_eq!(resumed_at(&dispatch, Some(STARTED), later, later), None);
    let mut plan = gate(Kind::Plan);
    plan.opened_by = Opener::Hub;
    assert_eq!(resumed_at(&plan, Some(STARTED), later, later), None);
}

#[test]
fn listing_by_modification_time_skips_older_files() {
    let (_sandbox, ctx) = hub();
    let (mut old, mut new) = (gate(Kind::Plan), gate(Kind::Plan));
    old.id = "old".to_string();
    new.id = "new".to_string();
    save(&ctx, Shelf::Open, &old).unwrap();
    save(&ctx, Shelf::Open, &new).unwrap();
    let open = dir_of(&ctx, Shelf::Open);
    let now = SystemTime::now();
    let then = now - std::time::Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(path_of(&open, "old"))
        .unwrap()
        .set_modified(then)
        .unwrap();
    let since = now - std::time::Duration::from_secs(60);
    assert_eq!(ids(list_modified_since(&open, since)), ["new"]);
}

#[test]
fn a_saved_gate_reads_back_the_same() {
    let (_sandbox, ctx) = hub();
    let gate = gate(Kind::Plan);
    save(&ctx, Shelf::Open, &gate).unwrap();
    assert_eq!(
        load(&ctx.state, &ctx.repo.slug, Shelf::Open, &gate.id).unwrap(),
        gate
    );
}

/// Written again over itself, a gate leaves nothing staged behind and still reads back.
#[test]
fn saving_over_a_gate_replaces_it_and_leaves_nothing_behind() {
    let (_sandbox, ctx) = hub();
    let mut gate = gate(Kind::Diff);
    save(&ctx, Shelf::Open, &gate).unwrap();
    gate.title = "Rewritten".to_string();
    save(&ctx, Shelf::Open, &gate).unwrap();
    assert_eq!(
        load(&ctx.state, &ctx.repo.slug, Shelf::Open, &gate.id).unwrap(),
        gate
    );
    let names: Vec<String> = std::fs::read_dir(dir_of(&ctx, Shelf::Open))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, [format!("{}.json", gate.id)]);
}

/// Two pieces of work finishing in the same second is ordinary, and the loser of a
/// check-then-write would overwrite a gate somebody is reading.
#[test]
fn ids_claimed_in_the_same_second_do_not_collide() {
    let (_sandbox, ctx) = hub();
    let ids: Vec<String> = (0..3)
        .map(|_| claim_id(&ctx, Shelf::Open, "20260922T041233Z", Kind::Diff).unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "20260922T041233Z-diff",
            "20260922T041233Z-diff-2",
            "20260922T041233Z-diff-3"
        ]
    );
}

#[test]
fn listing_is_oldest_first_so_the_queue_is_worked_in_order() {
    let (_sandbox, ctx) = hub();
    for (id, at) in [
        ("c", "20260922T03"),
        ("a", "20260922T01"),
        ("b", "20260922T02"),
    ] {
        let mut gate = gate(Kind::Plan);
        gate.id = id.to_string();
        gate.opened_at = at.to_string();
        save(&ctx, Shelf::Open, &gate).unwrap();
    }
    assert_eq!(
        ids(list(&ctx.state, &ctx.repo.slug, Shelf::Open)),
        ["a", "b", "c"]
    );
}

#[test]
fn listing_by_kind_reads_only_that_kind() {
    let (_sandbox, ctx) = hub();
    for (id, kind) in [
        ("20260922T01Z-plan", Kind::Plan),
        ("20260922T02Z-plan-2", Kind::Plan),
        ("20260922T03Z-diff", Kind::Diff),
    ] {
        let mut gate = gate(kind);
        gate.id = id.to_string();
        save(&ctx, Shelf::Open, &gate).unwrap();
    }
    let ids = ids(list_of_kind(
        &ctx.state,
        &ctx.repo.slug,
        Shelf::Open,
        Kind::Plan,
    ));
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert!(ids.iter().all(|id| id.contains("-plan")), "{ids:?}");
}

/// The `answered` subdirectory lives inside the gate directory, so it must not read as
/// a gate itself.
#[test]
fn the_archive_is_not_listed_as_an_open_gate() {
    let (_sandbox, ctx) = hub();
    let gate = gate(Kind::Plan);
    save(&ctx, Shelf::Open, &gate).unwrap();
    archive(&ctx, &gate).unwrap();
    assert!(list(&ctx.state, &ctx.repo.slug, Shelf::Open).is_empty());
    assert!(exists(
        &ctx.state,
        &ctx.repo.slug,
        Shelf::Answered,
        &gate.id
    ));
    assert!(!exists(&ctx.state, &ctx.repo.slug, Shelf::Open, &gate.id));
}

/// The board reads `answered/` on every poll, so the archived gate is staged and renamed
/// rather than written in place, and nothing of the staging is left behind.
#[test]
fn archiving_leaves_only_the_answered_file() {
    let (_sandbox, ctx) = hub();
    let gate = gate(Kind::Plan);
    save(&ctx, Shelf::Open, &gate).unwrap();
    archive(&ctx, &gate).unwrap();
    let names: Vec<String> = std::fs::read_dir(dir_of(&ctx, Shelf::Answered))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, [format!("{}.json", gate.id)]);
    assert!(!path_of(&dir_of(&ctx, Shelf::Open), &gate.id).exists());
}

/// A write in flight is a dotfile beside the gates, and it is not a gate yet.
#[test]
fn a_staged_file_is_not_listed() {
    let (_sandbox, ctx) = hub();
    let open = dir_of(&ctx, Shelf::Open);
    std::fs::create_dir_all(&open).unwrap();
    let json = serde_json::to_string(&gate(Kind::Plan)).unwrap();
    crate::infra::fs::stage(&open, &json).unwrap();
    assert!(list(&ctx.state, &ctx.repo.slug, Shelf::Open).is_empty());
}

#[test]
fn a_gate_is_found_open_first_then_as_a_record() {
    let (_sandbox, ctx) = hub();
    let (root, slug) = (&ctx.state, &ctx.repo.slug);
    let mut open = gate(Kind::Diff);
    open.id = "same".to_string();
    let mut record = open.clone();
    record.title = "the record".to_string();
    record.wait = false;
    save(&ctx, Shelf::Record, &record).unwrap();
    assert_eq!(get(root, slug, "same").unwrap(), record);
    save(&ctx, Shelf::Open, &open).unwrap();
    assert_eq!(get(root, slug, "same").unwrap(), open);
    assert_eq!(
        get(root, slug, "missing").unwrap_err(),
        "no open gate or record: missing"
    );
}

#[test]
fn a_broken_file_is_skipped_rather_than_blanking_the_queue() {
    let (_sandbox, ctx) = hub();
    save(&ctx, Shelf::Open, &gate(Kind::Plan)).unwrap();
    std::fs::write(
        dir_of(&ctx, Shelf::Open).join("broken.json"),
        "{ not json }",
    )
    .unwrap();
    assert_eq!(list(&ctx.state, &ctx.repo.slug, Shelf::Open).len(), 1);
}

/// The identifier leads, because it is the only thing an agent reading its outbox can
/// match an answer against.
#[test]
fn the_subject_leads_with_the_gate_id() {
    assert_eq!(
        answer_subject(&gate(Kind::Plan), "approve"),
        "[gate 20260922T041233Z-plan] approve"
    );
}

#[test]
fn the_answer_body_names_the_decision_and_the_chosen_option() {
    let body = answer_body(
        &gate(Kind::Plan),
        "choice",
        Some("const"),
        Some("Go ahead with this"),
    );
    assert!(body.contains("## Decision   choice"), "{body}");
    assert!(body.contains("Option A — a constant (const)"), "{body}");
    assert!(body.contains("Go ahead with this"), "{body}");
}

/// An agent that sees no comment section cannot tell "there was none" from "one was
/// lost on the way".
#[test]
fn a_missing_comment_is_said_rather_than_left_out() {
    let body = answer_body(&gate(Kind::Diff), "approve", None, None);
    assert!(body.contains("## Comment\n\n(none)"), "{body}");
}

#[test]
fn a_record_s_id_is_told_apart_from_a_gate_s() {
    let (_sandbox, ctx) = hub();
    assert_eq!(
        claim_id(&ctx, Shelf::Record, "20260922T041233Z", Kind::Diff).unwrap(),
        "20260922T041233Z-diff-record"
    );
}

/// The structured fields are what the board draws its tables from, so they have to come
/// back out exactly as the worker wrote them.
#[test]
fn a_record_with_its_structured_fields_reads_back_the_same() {
    let (_sandbox, ctx) = hub();
    let mut gate = gate(Kind::Diff);
    gate.wait = false;
    gate.review_rounds = vec![ReviewRound {
        engine: "claude".to_string(),
        must: 2,
        want: 1,
        scope: 0,
        false_positives: 1,
    }];
    gate.findings = vec![Finding {
        severity: Severity::Must,
        location: "src/gate.rs:10".to_string(),
        text: "unwrap on a missing file".to_string(),
        outcome: Outcome::Declined,
        reason: Some("the file is created just above".to_string()),
    }];
    gate.answers = vec![Answer {
        decision: "changes".to_string(),
        comment: Some("Please look at it again".to_string()),
        answered_at: "20260922T050000Z".to_string(),
    }];
    save(&ctx, Shelf::Record, &gate).unwrap();
    assert_eq!(
        load(&ctx.state, &ctx.repo.slug, Shelf::Record, &gate.id).unwrap(),
        gate
    );

    let json = serde_json::to_value(&gate).unwrap();
    assert_eq!(json["wait"], false);
    assert_eq!(json["reviewRounds"][0]["falsePositives"], 1);
    assert_eq!(json["findings"][0]["outcome"], "declined");
}

/// A gate written before records existed has no `wait`, and waits.
#[test]
fn a_gate_without_wait_is_one_that_waits() {
    let mut json = serde_json::to_value(gate(Kind::Plan)).unwrap();
    assert!(json.get("wait").is_none(), "{json}");
    json.as_object_mut().unwrap().remove("wait");
    assert!(serde_json::from_value::<Gate>(json).unwrap().wait);
}

#[test]
fn an_answer_to_a_record_says_it_is_one() {
    let mut gate = gate(Kind::Verify);
    gate.wait = false;
    let body = answer_body(&gate, "changes", None, Some("Please fix it"));
    assert!(body.contains("(verify, record)"), "{body}");
}

/// A report is read, not approved, so it must not come with an Approve button.
#[test]
fn a_result_gate_offers_reading_rather_than_approving() {
    assert_eq!(Kind::Result.default_options(), ["ack", "ask"]);
    assert_eq!(Kind::Diff.default_options(), ["approve", "changes"]);
}

/// A plan the hub opened for a task handed to Jules sits in a worktree with no worker, so
/// its answer has to reach the hub. One a worker opened stays the worker's.
#[test]
fn a_plan_is_answered_by_whoever_opened_it() {
    let mut plan = gate(Kind::Plan);
    assert!(!plan.answered_by_hub());
    plan.opened_by = Opener::Hub;
    assert!(plan.answered_by_hub());
    assert!(gate(Kind::Dispatch).answered_by_hub());
}

/// Written only when it says something: every gate a worker opens reads as it always has.
#[test]
fn the_opener_is_written_only_for_the_hub() {
    let worker = serde_json::to_value(gate(Kind::Plan)).unwrap();
    assert!(worker.get("openedBy").is_none());
    let mut plan = gate(Kind::Plan);
    plan.opened_by = Opener::Hub;
    let hub = serde_json::to_value(&plan).unwrap();
    assert_eq!(hub["openedBy"], "hub");
    let back: Gate = serde_json::from_value(hub).unwrap();
    assert_eq!(back, plan);
}

#[test]
fn open_drops_keys_the_gate_does_not_know() {
    let _sandbox = crate::testing::Sandbox::empty();
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = crate::registry::context_of(repo).unwrap();
    let request = GateRequest::from_json(
        &json!({"kind": "question", "title": "q", "worktree": "/tmp/wt", "futureField": 1}),
    )
    .unwrap();
    let (gate, _) = open(&ctx, request).unwrap();
    assert!(gate.extra.is_empty());
    let stored = load(&ctx.state, &ctx.repo.slug, Shelf::Open, &gate.id).unwrap();
    assert!(stored.extra.is_empty());
}

#[test]
fn an_open_payload_cannot_carry_a_decision() {
    let _sandbox = crate::testing::Sandbox::empty();
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = crate::registry::context_of(repo).unwrap();
    let request = GateRequest::from_json(&json!({
        "kind": "question", "title": "q", "worktree": "/tmp/wt",
        "decision": "approve", "choice": "a", "comment": "c",
        "answeredAt": "20260101T000000Z",
        "answers": [{"decision": "approve", "answeredAt": "20260101T000000Z"}],
    }))
    .unwrap();
    let (gate, _) = open(&ctx, request).unwrap();
    let stored = load(&ctx.state, &ctx.repo.slug, Shelf::Open, &gate.id).unwrap();
    for g in [&gate, &stored] {
        assert!(g.decision.is_none() && g.choice.is_none());
        assert!(g.comment.is_none() && g.answered_at.is_none());
        assert!(g.answers.is_empty());
        assert!(!g.answered_on_board());
    }
}

#[test]
fn null_opener_wait_and_worktree_read_as_absent() {
    let request = GateRequest::from_json(
        &json!({"kind": "question", "title": "q", "openedBy": null, "wait": null, "worktree": null}),
    )
    .unwrap();
    assert_eq!(request.opened_by, Opener::Worker);
    assert!(request.wait);
    assert_eq!(request.worktree, None);
}

#[test]
fn a_field_of_the_wrong_type_is_refused_by_name() {
    let err =
        GateRequest::from_json(&json!({"kind": "question", "title": "q", "facts": 5})).unwrap_err();
    assert!(err.starts_with("bad gate: "), "{err}");
    let err = GateRequest::from_json(&json!({"kind": "plan", "title": "t", "stoppedBy": null}))
        .unwrap_err();
    assert_eq!(err, "stoppedBy must be a list of rules");
}

#[test]
fn a_request_without_a_worktree_leaves_no_gate_behind() {
    let _sandbox = crate::testing::Sandbox::empty();
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = crate::registry::context_of(repo).unwrap();
    let request = GateRequest::from_json(&json!({"kind": "question", "title": "q"})).unwrap();
    let err = open(&ctx, request).unwrap_err();
    assert_eq!(err, "not inside a worktree, and no --worktree was given");
    let dir = dir_of(&ctx, Shelf::Open);
    let left = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
    assert_eq!(left, 0);
}

#[test]
fn absent_options_fall_back_to_the_kinds_defaults() {
    let _sandbox = crate::testing::Sandbox::empty();
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = crate::registry::context_of(repo).unwrap();
    let request =
        GateRequest::from_json(&json!({"kind": "question", "title": "q", "worktree": "/tmp/wt"}))
            .unwrap();
    let (gate, _) = open(&ctx, request).unwrap();
    assert_eq!(gate.options, Kind::Question.default_options());
}
