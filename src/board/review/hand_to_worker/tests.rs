use super::*;

use crate::testing::Sandbox;
use serde_json::{Value, json};

const PR: &str = "acme/widget#7";

fn hub() -> (Sandbox, Context) {
    let sandbox = Sandbox::empty();
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

fn put_task(ctx: &Context, id: &str, pr: Option<&str>, executor: Option<&str>) {
    let dir = ctx.state.join("tasks/acme-widget");
    std::fs::create_dir_all(&dir).unwrap();
    let mut task = json!({
        "id": id, "kind": "investigate", "title": "t", "doneWhen": "report-only",
        "status": "backlog", "autoStart": true, "createdAt": "20261001T000000Z",
        "updatedAt": "20261001T000000Z"
    });
    if let Some(pr) = pr {
        task["pr"] = json!(pr);
    }
    if let Some(executor) = executor {
        task["executor"] = json!(executor);
    }
    std::fs::write(dir.join(format!("{id}.json")), task.to_string()).unwrap();
}

fn put_record(ctx: &Context, pr_state: &str) {
    let record = json!({
        "id": PR, "repo": "Acme/Widget", "number": 7,
        "url": "https://github.com/acme/widget/pull/7",
        "registered": true, "title": "t", "base": "main", "head": "f", "headSha": "h1",
        "prState": pr_state, "additions": 1, "deletions": 1, "changedFiles": 1,
        "firstSeenAt": "20261001T000000Z", "readAt": "20261001T000000Z",
        "task": {"board": "acme-widget", "id": "t1"}
    });
    let dir = ctx.state.join("others");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("acme~widget~7.json"), record.to_string()).unwrap();
}

fn put_file(ctx: &Context, dir: &str, text: &str) {
    let dir = ctx.state.join("others").join(dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("acme~widget~7.json"), text).unwrap();
}

fn put_drafts(ctx: &Context, drafts: Value, overall: Option<&str>) {
    let mut file = json!({"id": PR, "savedAt": "20261001T000000Z", "version": 1, "drafts": drafts});
    if let Some(body) = overall {
        file["overall"] = json!({ "body": body });
    }
    put_file(ctx, "drafts", &file.to_string());
}

fn put_read(ctx: &Context) {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    put_file(
        ctx,
        "reads",
        &json!({"id": PR, "madeAt": "20261001T000000Z", "sha": sha}).to_string(),
    );
}

fn draft(key: &str, source: &str, path: Option<&str>, body: &str, sent: bool) -> Value {
    let mut d = json!({"key": key, "source": source, "side": "right", "body": body});
    if let Some(path) = path {
        d["path"] = json!(path);
        d["line"] = json!(5);
    }
    if sent {
        d["sentAt"] = json!("20261001T000001Z");
    }
    d
}

/// A task with a read PR, drafts of every sort, and a diff record to send them back on.
fn ready() -> (Sandbox, Context) {
    let (sandbox, ctx) = hub();
    put_task(
        &ctx,
        "t1",
        Some("https://github.com/acme/widget/pull/7"),
        None,
    );
    put_record(&ctx, "open");
    put_read(&ctx);
    put_drafts(
        &ctx,
        json!([
            draft("r1", "reader", Some("src/a.rs"), "from the reader", false),
            draft("p1", "person", None, "on the whole", false),
            draft("blank", "person", Some("src/b.rs"), "  ", false),
            draft("sent", "person", Some("src/c.rs"), "already", true),
        ]),
        Some("  Looks close  "),
    );
    let wt = ctx.state.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let record = json!({
        "id": "20261001T000100Z-diff-record", "kind": "diff", "worktree": wt.display().to_string(),
        "task": "t1", "title": "Diff", "wait": false, "openedAt": "20261001T000100Z"
    });
    let dir = ctx.state.join("gates/acme-widget/records");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("20261001T000100Z-diff-record.json"),
        record.to_string(),
    )
    .unwrap();
    (sandbox, ctx)
}

fn outbox(ctx: &Context) -> String {
    std::fs::read_to_string(ctx.state.join("wt/.claude/adjutant-outbox.md")).unwrap_or_default()
}

fn drafts_text(ctx: &Context) -> String {
    std::fs::read_to_string(ctx.state.join("others/drafts/acme~widget~7.json")).unwrap()
}

#[test]
fn the_unsent_written_drafts_and_the_overall_comment_are_handed_over_and_nothing_else_is_written() {
    let (_sandbox, ctx) = ready();
    let before = drafts_text(&ctx);
    let record_before =
        std::fs::read_to_string(ctx.state.join("others/acme~widget~7.json")).unwrap();
    let done = hand_to_worker(&ctx, "t1", PR).unwrap();
    assert_eq!(done.sent, ["r1", "p1"]);
    assert!(done.overall);
    assert_eq!(done.kind, Kind::Diff);
    assert_eq!(done.shelf, "record");
    assert_eq!(done.gate_id, "20261001T000100Z-diff-record");
    assert!(!done.at.is_empty());
    assert!(!done.present);

    let out = outbox(&ctx);
    // The reader's point is marked, the person's is not, and the pathless one is the whole PR.
    assert!(
        out.contains("### 1. src/a.rs:5 (AI)\nfrom the reader"),
        "{out}"
    );
    assert!(out.contains("### 2. the whole PR\non the whole"), "{out}");
    assert!(out.contains("### Overall\nLooks close"), "{out}");
    assert!(
        !out.contains("already") && !out.contains("src/b.rs"),
        "{out}"
    );
    assert!(out.contains("@ 0123456"), "{out}");
    // The drafts and the PR's record are the page's and the sync's: untouched.
    assert_eq!(drafts_text(&ctx), before);
    assert_eq!(
        std::fs::read_to_string(ctx.state.join("others/acme~widget~7.json")).unwrap(),
        record_before
    );
}

#[test]
fn a_left_side_draft_is_sent_as_the_old_side() {
    let (_sandbox, ctx) = ready();
    let mut d = draft("l", "person", Some("src/a.rs"), "gone line", false);
    d["side"] = json!("left");
    put_drafts(&ctx, json!([d]), None);
    let done = hand_to_worker(&ctx, "t1", PR).unwrap();
    assert!(!done.overall);
    assert!(outbox(&ctx).contains("### 1. src/a.rs:5 (old side)"));
}

#[test]
fn refusals_that_are_known_beforehand_write_nothing() {
    let (_sandbox, ctx) = hub();
    assert_eq!(hand_to_worker(&ctx, "t1", PR), Err(HandRefused::NoTask));
    put_task(&ctx, "t1", None, None);
    assert_eq!(hand_to_worker(&ctx, "t1", PR), Err(HandRefused::NoPr));
    put_task(
        &ctx,
        "t1",
        Some("https://github.com/acme/widget/pull/7"),
        None,
    );
    assert_eq!(
        hand_to_worker(&ctx, "t1", "acme/widget#8"),
        Err(HandRefused::PrChanged)
    );
    assert_eq!(hand_to_worker(&ctx, "t1", PR), Err(HandRefused::NoRecord));
    put_record(&ctx, "merged");
    assert_eq!(hand_to_worker(&ctx, "t1", PR), Err(HandRefused::PrDone));
    put_record(&ctx, "closed");
    assert_eq!(hand_to_worker(&ctx, "t1", PR), Err(HandRefused::PrDone));
    put_task(
        &ctx,
        "t1",
        Some("https://github.com/acme/widget/pull/7"),
        Some("jules"),
    );
    assert_eq!(hand_to_worker(&ctx, "t1", PR), Err(HandRefused::NoWorker));
    assert_eq!(outbox(&ctx), "");
}

#[test]
fn nothing_to_send_writes_nothing() {
    let (_sandbox, ctx) = ready();
    put_drafts(
        &ctx,
        json!([
            draft("blank", "person", Some("src/b.rs"), " ", false),
            draft("sent", "person", Some("src/c.rs"), "already", true),
        ]),
        Some(" "),
    );
    assert_eq!(hand_to_worker(&ctx, "t1", PR), Err(HandRefused::Nothing));
    // No drafts file at all.
    std::fs::remove_file(ctx.state.join("others/drafts/acme~widget~7.json")).unwrap();
    assert_eq!(hand_to_worker(&ctx, "t1", PR), Err(HandRefused::Nothing));
    assert_eq!(outbox(&ctx), "");
}

#[test]
fn drafts_that_cannot_be_read_are_an_error_and_not_none() {
    let (_sandbox, ctx) = ready();
    put_file(&ctx, "drafts", "{ not json");
    assert!(matches!(
        hand_to_worker(&ctx, "t1", PR),
        Err(HandRefused::Unreadable(_))
    ));
    assert_eq!(outbox(&ctx), "");
}

#[test]
fn a_gate_refusal_is_passed_on() {
    let (_sandbox, ctx) = ready();
    std::fs::remove_file(
        ctx.state
            .join("gates/acme-widget/records/20261001T000100Z-diff-record.json"),
    )
    .unwrap();
    assert_eq!(hand_to_worker(&ctx, "t1", PR), Err(HandRefused::NoGate));
    let dir = ctx.state.join("gates/acme-widget");
    let plan = json!({
        "id": "20261001T000200Z-plan", "kind": "plan", "worktree": "/tmp/wt", "task": "t1",
        "title": "Plan", "openedAt": "20261001T000200Z"
    });
    std::fs::write(dir.join("20261001T000200Z-plan.json"), plan.to_string()).unwrap();
    assert_eq!(
        hand_to_worker(&ctx, "t1", PR),
        Err(HandRefused::GateWaiting(Kind::Plan))
    );
    assert_eq!(outbox(&ctx), "");
}

#[test]
fn a_hand_over_running_for_the_task_is_busy_and_the_next_one_is_not() {
    let (_sandbox, ctx) = ready();
    let held = Running::take(&ctx.state, "acme-widget", "t1").unwrap();
    assert_eq!(hand_to_worker(&ctx, "t1", PR), Err(HandRefused::Busy));
    assert_eq!(outbox(&ctx), "");
    drop(held);
    assert!(hand_to_worker(&ctx, "t1", PR).is_ok());
}

#[test]
fn drafts_kept_from_an_earlier_read_say_so_in_the_message() {
    let (_sandbox, ctx) = ready();
    let earlier = "b".repeat(40);
    let file = json!({"id": PR, "savedAt": "20261001T000000Z", "version": 1, "fromSha": earlier,
        "drafts": [draft("r1", "reader", Some("src/a.rs"), "old point", false)]});
    put_file(&ctx, "drafts", &file.to_string());
    hand_to_worker(&ctx, "t1", PR).unwrap();
    let out = outbox(&ctx);
    assert!(out.contains("Lines are of 0123456"), "{out}");
    assert!(
        out.contains("kept from an earlier read at bbbbbbb; their lines are of that head"),
        "{out}"
    );
}

#[test]
fn drafts_of_the_current_read_say_nothing_of_an_earlier_one() {
    let (_sandbox, ctx) = ready();
    let now = "0123456789abcdef0123456789abcdef01234567";
    let file = json!({"id": PR, "savedAt": "20261001T000000Z", "version": 1, "fromSha": now,
        "drafts": [draft("r1", "reader", Some("src/a.rs"), "point", false)]});
    put_file(&ctx, "drafts", &file.to_string());
    hand_to_worker(&ctx, "t1", PR).unwrap();
    assert!(!outbox(&ctx).contains("earlier read"));
}

#[test]
fn a_second_concurrent_hand_over_is_busy_and_a_refusal_releases_the_guard() {
    let (_sandbox, ctx) = ready();
    let held = Running::take(&ctx.state, "acme-widget", "t1").unwrap();
    assert_eq!(hand_to_worker(&ctx, "t1", PR), Err(HandRefused::Busy));
    drop(held);
    // An early refusal gives the place back: the next call is refused for its own reason, not Busy.
    assert_eq!(
        hand_to_worker(&ctx, "t1", "acme/widget#8"),
        Err(HandRefused::PrChanged)
    );
    assert!(hand_to_worker(&ctx, "t1", PR).is_ok());
}
