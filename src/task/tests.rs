use super::*;

use super::store::{lock, save};

use serde_json::json;

/// Production builds a task out of the form's JSON; this is the shorthand the tests
/// need and nothing else does.
impl Task {
    /// A task as the form hands it over. Everything the hub fills in later is absent.
    pub fn new(id: String, kind: Kind, title: String, done_when: DoneWhen, stamp: &str) -> Task {
        Task {
            id,
            kind,
            title,
            body: String::new(),
            issue_url: None,
            done_when,
            stop_at: StopAt::default(),
            executor: Executor::default(),
            base: None,
            parent: None,
            worktree_name: None,
            auto_start: true,
            order: 0,
            status: Status::Backlog,
            worktree: None,
            issue: None,
            pr: None,
            jules_session: None,
            jules_by: None,
            relayed: Vec::new(),
            announced: Vec::new(),
            relay_rounds: 0,
            note: None,
            instruction: None,
            gate_answered_at: None,
            issue_snapshot: None,
            title_pending: false,
            needs_reading: false,
            pr_status: None,
            pr_turn_at: None,
            parked: None,
            created_at: stamp.to_string(),
            updated_at: stamp.to_string(),
            extra: serde_json::Map::new(),
        }
    }
}

/// A hub's context in a sandboxed state directory. Hold the sandbox for the whole test.
fn hub() -> (crate::testing::Sandbox, crate::registry::Context) {
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

/// Where hub `ctx`'s record `id` lives.
fn record_path(ctx: &crate::registry::Context, id: &str) -> std::path::PathBuf {
    store::path_of(&dir(&ctx.state, &ctx.repo.slug), id)
}

fn sample() -> Task {
    let mut task = Task::new(
        new_id("20260922T041233Z", "Fix the login retry"),
        Kind::Investigate,
        "ログインのリトライを調べる".to_string(),
        DoneWhen::ReportOnly,
        "20260922T041233Z",
    );
    task.body = "The retry does not seem to take effect".to_string();
    task
}

#[test]
fn a_key_this_binary_does_not_know_survives_a_load_and_save() {
    let (_sandbox, ctx) = hub();
    let task = sample();
    let mut raw = serde_json::to_value(&task).unwrap();
    raw["futureField"] = serde_json::json!({"n": 1});
    std::fs::create_dir_all(dir(&ctx.state, &ctx.repo.slug)).unwrap();
    std::fs::write(record_path(&ctx, &task.id), raw.to_string()).unwrap();

    let loaded = get(&ctx.state, &ctx.repo.slug, &task.id).unwrap();
    assert_eq!(loaded.extra["futureField"], serde_json::json!({"n": 1}));
    save(&ctx, &loaded).unwrap();

    let text = std::fs::read_to_string(record_path(&ctx, &task.id)).unwrap();
    let back: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(back["futureField"], serde_json::json!({"n": 1}));
    assert!(back.get("issueSnapshot").is_none(), "{text}");
}

fn snap(url: &str) -> IssueSnapshot {
    IssueSnapshot {
        url: url.to_string(),
        title: "T".to_string(),
        body: "B".to_string(),
        truncated: false,
        fetched_at: "20260922T041233Z".to_string(),
    }
}

#[test]
fn a_snapshot_survives_a_save_and_load_and_an_absent_one_is_not_written() {
    let (_sandbox, ctx) = hub();
    let mut task = sample();
    task.issue_snapshot = Some(snap("https://github.com/a/b/issues/1"));
    save(&ctx, &task).unwrap();
    assert_eq!(get(&ctx.state, &ctx.repo.slug, &task.id).unwrap(), task);
    let text = std::fs::read_to_string(record_path(&ctx, &task.id)).unwrap();
    assert!(text.contains("\"issueSnapshot\""), "{text}");
    assert!(text.contains("\"fetchedAt\""), "{text}");
    assert!(!text.contains("truncated"), "{text}");

    task.issue_snapshot = None;
    save(&ctx, &task).unwrap();
    let text = std::fs::read_to_string(record_path(&ctx, &task.id)).unwrap();
    assert!(!text.contains("issueSnapshot"), "{text}");
    assert_eq!(
        get(&ctx.state, &ctx.repo.slug, &task.id)
            .unwrap()
            .issue_snapshot,
        None
    );
}

#[test]
fn only_a_github_style_issue_url_is_fetchable() {
    for ok in [
        "https://github.com/a/b/issues/12",
        "https://github.com/a/b/issues/12#issuecomment-1",
        "https://github.com/a/b/issues/12?x=1",
        "https://github.com/a/b/issues/12/",
        "https://github.com/a/b/issues/12/#x",
        "https://ghe.example.com/a/b/issues/7",
    ] {
        assert!(fetchable_issue(ok), "{ok}");
    }
    for bad in [
        "https://github.com/a/b/pull/12",
        "https://github.com/a/b/issues/",
        "https://github.com/a/b/issues/x1",
        "https://github.com/a/b/issues/12x",
        "https://github.com/a/b/issues/12/anything",
        "https://github.com/a/b/issues/12//",
        "https://h/a?x/b/issues/1",
        "https://h/a#x/b/issues/1",
        "https://linear.app/team/issue/ABC-1/title",
        "https://example.atlassian.net/browse/ABC-1",
        "-x",
        "file:///a/b/issues/1",
        "",
    ] {
        assert!(!fetchable_issue(bad), "{bad}");
    }
}

#[test]
fn an_issue_ref_names_owner_repo_and_number() {
    assert_eq!(
        issue_ref("https://github.com/a/b/issues/12#issuecomment-1").as_deref(),
        Some("a/b#12")
    );
    assert_eq!(
        issue_ref("https://ghe.example.com/a/b/issues/7/").as_deref(),
        Some("a/b#7")
    );
    assert_eq!(issue_ref("https://github.com/a/b/pull/12"), None);
    assert_eq!(issue_ref("https://linear.app/x/issue/ABC-1"), None);
}

#[test]
fn the_issue_url_is_read_before_the_issue_the_worker_recorded() {
    let mut task = sample();
    task.issue = Some("https://github.com/a/b/issues/2".to_string());
    assert_eq!(
        issue_to_fetch(&task),
        Some("https://github.com/a/b/issues/2")
    );
    task.issue_url = Some("https://github.com/a/b/issues/1".to_string());
    assert_eq!(
        issue_to_fetch(&task),
        Some("https://github.com/a/b/issues/1")
    );
    task.issue_url = Some("https://linear.app/t/issue/A-1".to_string());
    assert_eq!(issue_to_fetch(&task), None);
}

#[test]
fn a_snapshot_is_wanted_once_the_task_is_started_and_none_of_that_url_is_kept() {
    let url = "https://github.com/a/b/issues/1";
    let mut task = sample();
    task.issue_url = Some(url.to_string());
    for status in [
        Status::Backlog,
        Status::Queued,
        Status::Done,
        Status::Cancelled,
    ] {
        task.status = status;
        assert_eq!(needs_snapshot(&task), None, "{}", status.as_str());
    }
    for status in [Status::Dispatched, Status::Pr] {
        task.status = status;
        assert_eq!(needs_snapshot(&task), Some(url), "{}", status.as_str());
    }
    task.issue_snapshot = Some(snap(url));
    assert_eq!(needs_snapshot(&task), None);
    task.issue_url = Some("https://github.com/a/b/issues/2".to_string());
    assert_eq!(
        needs_snapshot(&task),
        Some("https://github.com/a/b/issues/2")
    );
}

#[test]
fn a_snapshot_is_cut_to_the_caps_and_a_null_body_is_empty() {
    let url = "https://github.com/a/b/issues/1";
    let s = snapshot_from_gh(r#"{"title":"T","body":null}"#, url, "s").unwrap();
    assert_eq!(
        (s.title.as_str(), s.body.as_str(), s.truncated),
        ("T", "", false)
    );

    let big = "あ".repeat(ISSUE_BODY_CAP);
    let json = serde_json::json!({"title": "t".repeat(400), "body": big}).to_string();
    let s = snapshot_from_gh(&json, url, "s").unwrap();
    assert!(s.truncated);
    assert!(s.body.len() <= ISSUE_BODY_CAP && s.body.len() > ISSUE_BODY_CAP - 4);
    assert_eq!(s.title.chars().count(), ISSUE_TITLE_CAP);

    let json = serde_json::json!({"title": "t", "body": "x".repeat(ISSUE_BODY_CAP)});
    let s = snapshot_from_gh(&json.to_string(), url, "s").unwrap();
    assert!(!s.truncated);

    assert!(snapshot_from_gh("not json", url, "s").is_err());
    assert!(snapshot_from_gh(r#"{"body":"x"}"#, url, "s").is_err());
}

#[test]
fn a_parent_key_names_the_issue_of_the_one_repository_with_that_prefix() {
    let keys = serde_json::json!({"acme/team-app": "ALPHA", "acme/api": "BETA"});
    let keys = keys.as_object().unwrap();
    assert_eq!(
        parent_issue_url("ALPHA-233", keys).as_deref(),
        Some("https://github.com/acme/team-app/issues/233")
    );
    assert_eq!(
        parent_issue_url("alpha-7", keys).as_deref(),
        Some("https://github.com/acme/team-app/issues/7")
    );
    assert_eq!(parent_issue_url("GAMMA-1", keys), None);
    assert_eq!(parent_issue_url("ALPHA", keys), None);
    assert_eq!(parent_issue_url("ALPHA-x", keys), None);
    assert_eq!(parent_issue_url("-12", keys), None);
    let twice = serde_json::json!({"acme/a": "ALPHA", "acme/b": "alpha"});
    assert_eq!(
        parent_issue_url("ALPHA-1", twice.as_object().unwrap()),
        None
    );
}

#[test]
fn next_is_the_first_queued_task_and_ignores_every_other_status() {
    let mut t1 = sample();
    t1.id = "1".to_string();
    t1.status = Status::Backlog;
    let mut t2 = sample();
    t2.id = "2".to_string();
    t2.status = Status::Dispatched;
    let mut t3 = sample();
    t3.id = "3".to_string();
    t3.status = Status::Queued;
    let mut t4 = sample();
    t4.id = "4".to_string();
    t4.status = Status::Queued;

    let gated = std::collections::HashSet::new();
    let next_result = next(vec![t1, t2, t3.clone(), t4], &gated);
    assert_eq!(next_result.task, Some(t3));
    assert!(next_result.needs_dispatch_gate.is_empty());
}

#[test]
fn a_task_that_asks_first_is_passed_over_and_listed_until_its_gate_is_open() {
    let mut t1 = sample();
    t1.id = "1".to_string();
    t1.status = Status::Queued;
    t1.auto_start = false;
    let mut t2 = sample();
    t2.id = "2".to_string();
    t2.status = Status::Queued;
    t2.auto_start = true;

    let mut gated = std::collections::HashSet::new();
    let next_result = next(vec![t1.clone(), t2.clone()], &gated);
    assert_eq!(next_result.task, Some(t2.clone()));
    assert_eq!(next_result.needs_dispatch_gate, vec![t1.clone()]);

    gated.insert("1".to_string());
    let next_result_gated = next(vec![t1, t2.clone()], &gated);
    assert_eq!(next_result_gated.task, Some(t2));
    assert!(next_result_gated.needs_dispatch_gate.is_empty());
}

#[test]
fn a_task_that_could_not_start_waits_for_a_person() {
    let mut t1 = sample();
    t1.id = "1".to_string();
    t1.status = Status::Queued;
    t1.note = Some(format!("{COULD_NOT_START} fatal: bad base"));
    let mut t2 = sample();
    t2.id = "2".to_string();
    t2.status = Status::Queued;
    t2.note = Some("Waiting for a worker slot (resume with --resume)".to_string());
    let mut t3 = sample();
    t3.id = "3".to_string();
    t3.status = Status::Queued;
    t3.note = Some("Waiting for confirmation to start".to_string());

    let gated = std::collections::HashSet::new();
    let next_result = next(vec![t1, t2.clone(), t3], &gated);
    assert_eq!(next_result.task, Some(t2));
}

#[test]
fn a_task_that_could_not_start_is_not_asked_about_either() {
    let mut t1 = sample();
    t1.id = "1".to_string();
    t1.status = Status::Queued;
    t1.auto_start = false;
    t1.note = Some(format!("{COULD_NOT_START} fatal: bad base"));
    let mut t2 = sample();
    t2.id = "2".to_string();
    t2.status = Status::Queued;

    let gated = std::collections::HashSet::new();
    let next_result = next(vec![t1, t2.clone()], &gated);
    assert_eq!(next_result.task, Some(t2));
    assert!(next_result.needs_dispatch_gate.is_empty());
}

#[test]
fn the_whole_queue_is_searched_for_tasks_that_need_a_gate() {
    let mut t1 = sample();
    t1.id = "1".to_string();
    t1.status = Status::Queued;
    t1.auto_start = true;
    let mut t2 = sample();
    t2.id = "2".to_string();
    t2.status = Status::Queued;
    t2.auto_start = false;

    let gated = std::collections::HashSet::new();
    let next_result = next(vec![t1.clone(), t2.clone()], &gated);
    assert_eq!(next_result.task, Some(t1));
    assert_eq!(next_result.needs_dispatch_gate, vec![t2]);
}

#[test]
fn nothing_is_next_when_every_queued_task_is_held() {
    let mut t1 = sample();
    t1.id = "1".to_string();
    t1.status = Status::Queued;
    t1.auto_start = false;
    let mut t2 = sample();
    t2.id = "2".to_string();
    t2.status = Status::Queued;
    t2.note = Some(format!("{COULD_NOT_START} fatal: bad base"));

    let gated = std::collections::HashSet::new();
    let next_result = next(vec![t1.clone(), t2], &gated);
    assert_eq!(next_result.task, None);
    assert_eq!(next_result.needs_dispatch_gate, vec![t1]);
}

#[test]
fn id_carries_a_readable_tail_when_the_title_has_one() {
    assert_eq!(
        new_id("20260922T041233Z", "Fix the login retry"),
        "20260922T041233Z-fix-the-login-retry"
    );
}

/// A Japanese title leaves nothing to slugify, and the stamp alone is still a usable
/// id. The alternative — refusing the task — would reject the common case here.
#[test]
fn id_is_the_stamp_alone_when_nothing_survives_slugging() {
    assert_eq!(
        new_id("20260922T041233Z", "ログインのリトライ"),
        "20260922T041233Z"
    );
}

#[test]
fn slug_does_not_run_past_its_cap_or_end_on_a_separator() {
    let slug = slug("a very long english title that keeps going and going and going");
    assert!(slug.len() <= 32, "{slug}");
    assert!(!slug.ends_with('-'), "{slug}");
}

/// Two tasks written in the same second, with titles that slug to nothing, must not
/// land on the same file — which is what filling the form twice in a row looks like.
#[test]
fn ids_claimed_in_the_same_second_do_not_collide() {
    let (_sandbox, ctx) = hub();
    let ids: Vec<String> = (0..3)
        .map(|_| store::claim_id(&ctx, "20260922T041233Z", "ログインのリトライ").unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "20260922T041233Z",
            "20260922T041233Z-2",
            "20260922T041233Z-3"
        ]
    );
}

/// The claim leaves the file behind, so `save` has somewhere to land and no second
/// caller can take the name in between.
#[test]
fn a_claimed_id_is_held_before_anything_is_written_to_it() {
    let (_sandbox, ctx) = hub();
    let id = store::claim_id(&ctx, "20260922T041233Z", "x").unwrap();
    assert!(record_path(&ctx, &id).exists());
}

#[test]
fn a_saved_task_reads_back_the_same() {
    let (_sandbox, ctx) = hub();
    let task = sample();
    save(&ctx, &task).unwrap();
    assert_eq!(get(&ctx.state, &ctx.repo.slug, &task.id).unwrap(), task);
}

#[test]
fn listing_is_in_queue_order() {
    let (_sandbox, ctx) = hub();
    for (id, order) in [("a", 3u32), ("b", 1), ("c", 2)] {
        let mut task = sample();
        task.id = id.to_string();
        task.order = order;
        save(&ctx, &task).unwrap();
    }
    let ids: Vec<String> = list(&ctx.state, &ctx.repo.slug)
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids, ["b", "c", "a"]);
}

/// One unreadable file must not blank the board.
#[test]
fn a_file_that_will_not_parse_is_skipped_rather_than_fatal() {
    let (_sandbox, ctx) = hub();
    let task = sample();
    save(&ctx, &task).unwrap();
    std::fs::write(record_path(&ctx, "broken"), "{ not json }").unwrap();
    assert_eq!(list(&ctx.state, &ctx.repo.slug).len(), 1);
}

#[test]
fn listing_a_directory_that_is_not_there_is_empty_rather_than_an_error() {
    let (_sandbox, ctx) = hub();
    assert!(list(&ctx.state, "nope").is_empty());
}

#[test]
fn a_record_reads_back_through_get_and_list_by_root_and_slug() {
    let (_sandbox, ctx) = hub();
    let task = sample();
    save(&ctx, &task).unwrap();
    assert_eq!(get(&ctx.state, &ctx.repo.slug, &task.id).unwrap(), task);
    let ids: Vec<String> = list(&ctx.state, &ctx.repo.slug)
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids, std::slice::from_ref(&task.id));
    assert!(list(&ctx.state, "other-hub").is_empty());

    // Pins where the records are on disk: another hub's directory under the same root.
    let other = ctx.state.join("tasks").join("other-hub");
    std::fs::create_dir_all(&other).unwrap();
    let mut written = sample();
    written.id = "x".to_string();
    std::fs::write(
        other.join("x.json"),
        serde_json::to_string(&written).unwrap(),
    )
    .unwrap();
    assert_eq!(get(&ctx.state, "other-hub", "x").unwrap(), written);

    let err = get(&ctx.state, &ctx.repo.slug, "../x").unwrap_err();
    assert!(err.starts_with("no such task"), "{err}");
    std::fs::write(other.join("broken.json"), "{ not json }").unwrap();
    let err = get(&ctx.state, "other-hub", "broken").unwrap_err();
    assert!(err.starts_with("cannot read"), "{err}");
}

#[test]
fn remove_deletes_the_record_and_leaves_its_lock() {
    let (_sandbox, ctx) = hub();
    let task = sample();
    save(&ctx, &task).unwrap();
    drop(lock(&ctx, &task.id).unwrap());
    remove(&ctx, &task.id).unwrap();
    assert!(!record_path(&ctx, &task.id).exists());
    let lock_file = dir(&ctx.state, &ctx.repo.slug).join(format!("{}.lock", task.id));
    assert!(lock_file.exists());
    let err = get(&ctx.state, &ctx.repo.slug, &task.id).unwrap_err();
    assert!(err.starts_with("no such task"), "{err}");
    assert!(remove(&ctx, "../x").is_err());
}

#[test]
fn note_gate_answered_stamps_the_record_and_leaves_a_missing_one_alone() {
    let (_sandbox, ctx) = hub();
    let task = sample();
    save(&ctx, &task).unwrap();
    note_gate_answered(&ctx, &task.id, "20260922T050000Z");
    let after = get(&ctx.state, &ctx.repo.slug, &task.id).unwrap();
    assert_eq!(after.gate_answered_at.as_deref(), Some("20260922T050000Z"));

    // Best effort: no record, and no panic, and nothing written.
    note_gate_answered(&ctx, "nothing-here", "20260922T050000Z");
    assert!(!record_path(&ctx, "nothing-here").exists());
}

/// The message the hub reads has to carry everything the form asked for; otherwise the
/// hub goes back to asking the questions the form existed to answer.
#[test]
fn the_request_body_carries_what_the_form_collected() {
    let mut task = sample();
    task.base = Some("origin/release/1.2".to_string());
    task.worktree_name = Some("login-retry".to_string());
    task.auto_start = false;
    let body = render_request(&task);
    assert!(body.contains(&task.id), "{body}");
    assert!(body.contains("origin/release/1.2"), "{body}");
    assert!(body.contains("login-retry"), "{body}");
    assert!(body.contains("ask before starting"), "{body}");
    assert!(
        body.contains("investigation only (report and stop)"),
        "{body}"
    );
    assert!(
        body.contains("The retry does not seem to take effect"),
        "{body}"
    );
}

#[test]
fn a_title_not_read_yet_is_said_in_the_request() {
    let mut task = sample();
    assert!(!render_request(&task).contains("## Title"));
    task.title_pending = true;
    assert!(render_request(&task).contains("## Title         not read yet"));
}

/// Absent fields are written as `-` rather than left out: the hub reads this as prose,
/// and a missing line reads as "nobody said" while an empty one reads as "said nothing".
#[test]
fn unset_fields_say_so_rather_than_vanishing() {
    let body = render_request(&sample());
    assert!(body.contains("## Base          -"), "{body}");
    assert!(body.contains("## Parent task   -"), "{body}");
}

/// The hub copies the stop point into the brief, so the message has to say it — and say
/// the default out loud rather than leave the line off.
#[test]
fn the_request_body_says_where_the_task_stops() {
    let mut task = sample();
    assert!(
        render_request(&task).contains("## Stop at       plan ("),
        "{}",
        render_request(&task)
    );
    task.stop_at = StopAt::All;
    assert!(
        render_request(&task).contains("## Stop at       all ("),
        "{}",
        render_request(&task)
    );
}

/// The brief and the request say it in these words, and the worker matches on them.
#[test]
fn done_when_prose_is_what_the_request_says() {
    let mut task = sample();
    for done_when in [
        DoneWhen::ReportOnly,
        DoneWhen::Verify,
        DoneWhen::Pr,
        DoneWhen::Review,
    ] {
        task.done_when = done_when;
        let line = format!("## Done when     {}\n", done_when.as_prose());
        assert!(render_request(&task).contains(&line));
    }
    assert_eq!(DoneWhen::Pr.as_prose(), "up to a PR");
}

/// A record written before the field existed stopped at the plan, and still does.
#[test]
fn a_record_without_a_stop_point_stops_at_the_plan() {
    let mut value = serde_json::to_value(sample()).unwrap();
    value.as_object_mut().unwrap().remove("stopAt");
    let task: Task = serde_json::from_value(value).unwrap();
    assert_eq!(task.stop_at, StopAt::Plan);
}

#[test]
fn the_request_body_includes_instruction_when_present() {
    let mut task = sample();
    assert!(!render_request(&task).contains("## Handover note"));

    task.instruction = Some("Look into how the existing code behaves first".to_string());
    let body = render_request(&task);
    assert!(body.contains("## Handover note\n\nLook into how the existing code behaves first\n"));
}

#[test]
fn a_record_without_an_instruction_deserializes_with_none() {
    let value = serde_json::to_value(sample()).unwrap();
    let task: Task = serde_json::from_value(value).unwrap();
    assert_eq!(task.instruction, None);
}

fn at(owner: &str, repo: &str, number: u64) -> PrRef {
    PrRef {
        host: "github.com".to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
    }
}

#[test]
fn a_pr_url_is_read_whatever_follows_the_number() {
    for url in [
        "https://github.com/acme/widget/pull/12",
        "https://github.com/acme/widget/pull/12/",
        "https://github.com/acme/widget/pull/12/files",
        "https://github.com/acme/widget/pull/12?diff=split",
        "https://github.com/acme/widget/pull/12#issuecomment-1",
        "https://GitHub.com/Acme/Widget/pull/12",
    ] {
        assert_eq!(pr_ref(url, None), Some(at("acme", "widget", 12)), "{url}");
    }
    let enterprise = pr_ref("https://git.example.com/acme/widget/pull/3", None).unwrap();
    assert_eq!(enterprise.host, "git.example.com");
    assert_eq!(enterprise.nwo(), "acme/widget");
}

#[test]
fn a_bare_number_needs_the_repository_to_be_known() {
    let here = Some(("github.com", "acme/widget"));
    assert_eq!(pr_ref("12", here), Some(at("acme", "widget", 12)));
    assert_eq!(pr_ref("#12", here), Some(at("acme", "widget", 12)));
    assert_eq!(pr_ref("12", None), None);
    assert_eq!(pr_ref("#12", None), None);
    // A directory name is not a repository.
    assert_eq!(pr_ref("12", Some(("github.com", "widget"))), None);
    // It is read on the host the repository's origin is on, not on github.com.
    let enterprise = pr_ref("12", Some(("git.example.com", "acme/widget"))).unwrap();
    assert_eq!(enterprise.host, "git.example.com");
}

#[test]
fn what_is_not_a_pull_request_is_not_guessed_at() {
    for odd in [
        "",
        "-x",
        "--web",
        "-1",
        "https://github.com/acme/widget/issues/12",
        "https://github.com/acme/widget/pull/",
        "https://github.com/acme/widget/pull/x",
        "https://github.com/acme/pull/12",
        "https://github.com/-acme/widget/pull/12",
        "ftp://github.com/acme/widget/pull/12",
        "https://github.com/acme/widget?x=/pull/12",
        "twelve",
    ] {
        assert_eq!(
            pr_ref(odd, Some(("github.com", "acme/widget"))),
            None,
            "{odd:?}"
        );
    }
}

#[test]
fn a_notification_names_its_pull_request_by_an_api_url() {
    assert_eq!(
        pr_ref_from_api("https://api.github.com/repos/acme/widget/pulls/7"),
        Some(at("acme", "widget", 7))
    );
    let enterprise =
        pr_ref_from_api("https://git.example.com/api/v3/repos/acme/widget/pulls/7").unwrap();
    assert_eq!(
        (enterprise.host.as_str(), enterprise.number),
        ("git.example.com", 7)
    );
    for odd in [
        "https://api.github.com/repos/acme/widget/issues/7",
        "https://api.github.com/repos/acme/widget/pulls/x",
        "https://git.example.com/repos/acme/widget/pulls/7",
        "https://github.com/acme/widget/pull/7",
        "",
    ] {
        assert_eq!(pr_ref_from_api(odd), None, "{odd:?}");
    }
}

fn pr(state: &str, review: &str, fail: u32, pending: u32) -> PrStatus {
    PrStatus {
        state: state.to_string(),
        title: String::new(),
        review: review.to_string(),
        ci: CheckCounts {
            pass: 1,
            fail,
            pending,
        },
        head: None,
    }
}

#[test]
fn a_turn_is_read_from_the_state_the_review_and_the_checks() {
    let turn = |s: PrStatus| pr_turn(&s);
    assert_eq!(turn(pr("draft", "none", 0, 0)), Some(PrTurn::Draft));
    assert_eq!(turn(pr("open", "none", 0, 0)), Some(PrTurn::Unrequested));
    assert_eq!(
        turn(pr("open", "required", 0, 0)),
        Some(PrTurn::OtherReviewer)
    );
    assert_eq!(turn(pr("open", "required", 0, 2)), Some(PrTurn::Checks));
    assert_eq!(turn(pr("open", "changes", 0, 0)), Some(PrTurn::Changes));
    assert_eq!(turn(pr("open", "approved", 0, 0)), Some(PrTurn::Merge));
    assert_eq!(turn(pr("open", "none", 1, 0)), Some(PrTurn::CiFailed));
    assert_eq!(turn(pr("merged", "approved", 0, 0)), Some(PrTurn::Merged));
    assert_eq!(turn(pr("closed", "none", 0, 0)), Some(PrTurn::Closed));
    assert_eq!(turn(pr("something-new", "none", 0, 0)), None);
}

#[test]
fn what_the_person_can_act_on_comes_first() {
    let turn = |s: PrStatus| pr_turn(&s);
    assert_eq!(turn(pr("open", "changes", 0, 3)), Some(PrTurn::Changes));
    assert_eq!(turn(pr("open", "changes", 1, 0)), Some(PrTurn::Changes));
    assert_eq!(turn(pr("open", "approved", 0, 3)), Some(PrTurn::Checks));
    assert_eq!(turn(pr("open", "approved", 1, 3)), Some(PrTurn::CiFailed));
    // A draft is not asked about its reviews or its checks.
    assert_eq!(turn(pr("draft", "approved", 1, 1)), Some(PrTurn::Draft));
}

#[test]
fn a_turn_serializes_as_a_word() {
    assert_eq!(
        serde_json::to_value(PrTurn::OtherReviewer).unwrap(),
        serde_json::json!("other-reviewer")
    );
    assert_eq!(
        serde_json::to_value(PrTurn::CiFailed).unwrap(),
        serde_json::json!("ci-failed")
    );
}

#[test]
fn a_pr_waits_on_a_person_by_its_turn_and_then_by_the_worker_phase() {
    let waits = |status: &PrStatus, phase: Option<Option<&str>>| {
        pr_waits_on_person(Status::Pr, true, Some(status), phase)
    };
    let changes = pr("open", "changes", 0, 0);
    let approved = pr("open", "approved", 0, 0);
    let failed = pr("open", "none", 1, 0);
    let closed = pr("closed", "none", 0, 0);
    for person in [&changes, &approved, &failed, &closed] {
        for phase in [None, Some(Some("pr")), Some(Some("pr-bots")), Some(None)] {
            // A worker with no phase is not handing over anything.
            let expected = phase != Some(None);
            assert_eq!(waits(person, phase), expected, "{person:?} {phase:?}");
        }
        // The worker is still at work: it stays on the agent board.
        assert!(!waits(person, Some(Some("review"))), "{person:?}");
        assert!(!waits(person, Some(Some("implement"))), "{person:?}");
    }
    for other in [
        pr("open", "required", 0, 0),
        pr("open", "none", 0, 2),
        pr("merged", "approved", 0, 0),
    ] {
        for phase in [None, Some(Some("pr")), Some(Some("pr-bots"))] {
            assert!(!waits(&other, phase), "{other:?} {phase:?}");
        }
    }
    // A draft, or a PR nobody was asked to review, is decided as it was before.
    for undecided in [pr("draft", "none", 0, 0), pr("open", "none", 0, 0)] {
        assert!(waits(&undecided, Some(Some("pr"))));
        assert!(!waits(&undecided, Some(Some("pr-bots"))));
        assert!(waits(&undecided, None));
    }
    assert!(pr_waits_on_person(Status::Pr, true, None, Some(Some("pr"))));
    assert!(!pr_waits_on_person(
        Status::Pr,
        true,
        None,
        Some(Some("pr-bots"))
    ));
    assert!(!pr_waits_on_person(Status::Dispatched, true, None, None));
}

#[test]
fn a_jules_pr_follows_the_turn_and_waits_when_the_turn_says_nothing() {
    for waits in [
        pr("open", "changes", 0, 0),
        pr("open", "approved", 0, 0),
        pr("open", "none", 1, 0),
        pr("closed", "none", 0, 0),
        pr("draft", "none", 0, 0),
        pr("open", "none", 0, 0),
    ] {
        assert!(jules_pr_waits_on_person(Some(&waits)), "{waits:?}");
    }
    for off in [
        pr("open", "required", 0, 0),
        pr("open", "none", 0, 2),
        pr("merged", "approved", 0, 0),
    ] {
        assert!(!jules_pr_waits_on_person(Some(&off)), "{off:?}");
    }
    assert!(jules_pr_waits_on_person(None));
}

#[test]
fn nothing_waits_on_a_person_without_a_pr_or_after_the_task_ended() {
    let closed = pr("closed", "none", 0, 0);
    assert!(!pr_waits_on_person(Status::Pr, false, Some(&closed), None));
    for status in [Status::Done, Status::Cancelled] {
        assert!(!pr_waits_on_person(status, true, Some(&closed), None));
    }
}

/// A sandbox hub to create records in.
fn sandbox_ctx() -> crate::registry::Context {
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    crate::registry::context_of(repo).unwrap()
}

/// What a caller may say about a new task is `NewTask`; the rest of the record is the hub's,
/// so a field outside it never reaches the file.
#[test]
fn a_new_task_takes_no_field_outside_new_task() {
    let _sandbox = crate::testing::Sandbox::empty();
    let ctx = sandbox_ctx();
    let input = json!({
        "title": "t",
        "pr": "https://github.com/o/r/pull/1",
        "relayed": ["1"],
        "issueSnapshot": { "url": "u", "title": "x", "fetchedAt": "s" },
        "futureField": 1,
    });
    let new = NewTask::from_json(&input).unwrap();
    assert_eq!(
        new,
        NewTask {
            title: Some("t".to_string()),
            ..NewTask::default()
        }
    );
    let (task, handed) = create(&ctx, new, true).unwrap();
    assert!(task.pr.is_none());
    assert!(task.relayed.is_empty());
    assert!(task.issue_snapshot.is_none());
    assert!(task.extra.is_empty());
    assert!(handed.is_none());
    let text =
        std::fs::read_to_string(dir(&ctx.state, &ctx.repo.slug).join(format!("{}.json", task.id)))
            .unwrap();
    for left_out in ["pull/1", "relayed", "issueSnapshot", "futureField"] {
        assert!(!text.contains(left_out), "{text}");
    }
}

#[test]
fn title_is_taken_verbatim_when_present() {
    assert_eq!(
        derive_title(Some("explicit title"), "first line\nsecond line").as_deref(),
        Some("explicit title")
    );
}

#[test]
fn title_is_derived_from_the_first_non_empty_line_of_the_body() {
    assert_eq!(
        derive_title(
            None,
            "\n\n  Fix the flaky network retry logic  \nand more details"
        )
        .as_deref(),
        Some("Fix the flaky network retry logic")
    );
}

#[test]
fn title_keeps_the_whole_first_line() {
    let derived = derive_title(None, &"a".repeat(120)).expect("derived");
    assert_eq!(derived, "a".repeat(120));
    let japanese = "あ".repeat(100);
    let derived = derive_title(None, &format!("{japanese}\nsecond")).expect("derived");
    assert_eq!(derived, japanese);
}

/// A form sends the stop point whether or not one was picked, and nothing picked is the
/// default rather than a refusal.
#[test]
fn a_stop_point_left_blank_is_the_default_and_anything_unknown_is_refused() {
    for blank in [json!({}), json!({"stopAt": null}), json!({"stopAt": ""})] {
        assert_eq!(
            NewTask::from_json(&blank).unwrap().stop_at,
            StopAt::Plan,
            "{blank}"
        );
    }
    assert_eq!(
        NewTask::from_json(&json!({"stopAt": "all"}))
            .unwrap()
            .stop_at,
        StopAt::All
    );
    assert_eq!(
        NewTask::from_json(&json!({"stopAt": "verify"})).unwrap_err(),
        "no such stop point: verify (plan, diff or all)"
    );
    assert_eq!(
        StopAt::parse("verify").unwrap_err(),
        "no such stop point: verify (plan, diff or all)"
    );
    assert_eq!(
        NewTask::from_json(&json!({"stopAt": 1})).unwrap_err(),
        "no such stop point: 1"
    );
}

#[test]
fn a_value_a_task_cannot_hold_is_refused_in_the_words_the_record_uses() {
    let err = NewTask::from_json(&json!({"doneWhen": "never"})).unwrap_err();
    assert!(err.starts_with("bad task: unknown variant"), "{err}");
    assert_eq!(err, DoneWhen::parse("never").unwrap_err());
    assert_eq!(
        NewTask::from_json(&json!({"kind": "nope"})).unwrap_err(),
        Kind::parse("nope").unwrap_err()
    );
    assert_eq!(
        NewTask::from_json(&json!({"executor": "julse"})).unwrap_err(),
        "no such executor: julse (worker or jules)"
    );
    assert!(
        NewTask::from_json(&json!({"autoStart": "yes"}))
            .unwrap_err()
            .starts_with("bad task: ")
    );
    assert_eq!(
        NewTask::from_json(&json!([])).unwrap_err(),
        "expected an object"
    );
}

/// The parent task is typed on the form and quoted on a command line, so it is held to
/// the same rule as the issue URL.
#[test]
fn a_parent_task_url_that_could_close_a_quote_is_refused() {
    let with_parent = |parent: &str| NewTask {
        parent: Some(parent.to_string()),
        ..NewTask::default()
    };
    assert!(check_typed(&with_parent("https://github.com/acme/widget/issues/1")).is_ok());
    // A key typed on the board is turned into its URL by the hub, not refused here.
    assert!(check_typed(&with_parent("ALPHA-233")).is_ok());
    assert!(check_typed(&with_parent("")).is_ok());
    for bad in [
        "https://x.test/a'; rm -rf ~; '",
        "not a url",
        "https://x.test/a b",
        "ALPHA-233; rm",
    ] {
        let err = check_typed(&with_parent(bad)).unwrap_err();
        assert!(err.starts_with("not a task URL: "), "{err}");
    }
}

#[test]
fn a_parent_may_be_a_key_or_a_plain_url_and_nothing_else() {
    assert!(check_parent("ABC-123").is_ok());
    assert!(check_parent("https://example.test/browse/ABC-123").is_ok());
    assert!(check_parent("ABC-123 && id").is_err());
}

#[test]
fn a_text_field_of_a_patch_is_left_alone_absent_cleared_by_null_or_empty_and_set_by_a_string() {
    let pr = |input: serde_json::Value| TaskPatch::from_json(&input).map(|p| p.pr);
    assert_eq!(pr(json!({})).unwrap(), None);
    assert_eq!(pr(json!({ "pr": null })).unwrap(), Some(None));
    assert_eq!(pr(json!({ "pr": "" })).unwrap(), Some(None));
    assert_eq!(
        pr(json!({ "pr": "https://x/pull/1" })).unwrap(),
        Some(Some("https://x/pull/1".to_string()))
    );
    assert_eq!(
        pr(json!({ "pr": 42 })).unwrap_err(),
        "pr has to be a string or null, not 42"
    );
    let patch = TaskPatch::from_json(&json!({
        "instruction": "優先して実装してください",
        "julesSession": "s-1",
        "note": "",
    }))
    .unwrap();
    assert_eq!(
        patch.instruction,
        Some(Some("優先して実装してください".to_string()))
    );
    assert_eq!(patch.jules_session, Some(Some("s-1".to_string())));
    assert_eq!(patch.note, Some(None));
    for bad in [json!(42), json!(true), json!(["a"]), json!({"a": 1})] {
        assert!(pr(json!({ "pr": bad })).is_err(), "{bad} was taken");
    }
}

#[test]
fn the_parent_of_a_patch_is_set_by_a_url_or_key_and_cleared_by_null_or_empty() {
    let parent = |input: serde_json::Value| TaskPatch::from_json(&input).map(|p| p.parent);
    assert_eq!(parent(json!({})).unwrap(), None);
    assert_eq!(parent(json!({ "parent": null })).unwrap(), Some(None));
    assert_eq!(parent(json!({ "parent": "" })).unwrap(), Some(None));
    assert_eq!(
        parent(json!({ "parent": "https://github.com/acme/widget/issues/549" })).unwrap(),
        Some(Some(
            "https://github.com/acme/widget/issues/549".to_string()
        ))
    );
    assert_eq!(
        parent(json!({ "parent": "ABC-123" })).unwrap(),
        Some(Some("ABC-123".to_string()))
    );
    assert_eq!(
        parent(json!({ "parent": 549 })).unwrap_err(),
        "parent has to be a string or null, not 549"
    );
}

#[test]
fn a_parent_new_task_would_refuse_is_refused_by_a_patch_before_any_write() {
    for bad in [
        "ABC-123 && id",
        "not a url",
        "https://x.test/a'b",
        "javascript:1",
    ] {
        assert!(
            check_parent(bad).is_err(),
            "{bad} passed check_parent, which this test assumes it does not"
        );
        let err = TaskPatch::from_json(&json!({ "parent": bad })).unwrap_err();
        assert!(err.contains(bad), "{err}");
    }
    // The same check guards a patch built by hand, which is what the command line does.
    let (_sandbox, ctx) = hub();
    let task = sample();
    save(&ctx, &task).unwrap();
    let patch = TaskPatch {
        parent: Some(Some("a; b".to_string())),
        status: Some(Status::Done),
        ..TaskPatch::default()
    };
    assert!(update(&ctx, &task.id, &patch, false).is_err());
    let after = get(&ctx.state, &ctx.repo.slug, &task.id).unwrap();
    assert_eq!(after, task, "a refused update wrote something");
}

#[test]
fn an_update_sets_and_clears_the_parent_and_keeps_what_it_does_not_know() {
    let (_sandbox, ctx) = hub();
    let task = sample();
    let mut raw = serde_json::to_value(&task).unwrap();
    raw["futureField"] = json!({"n": 1});
    std::fs::create_dir_all(dir(&ctx.state, &ctx.repo.slug)).unwrap();
    std::fs::write(record_path(&ctx, &task.id), raw.to_string()).unwrap();

    let url = "https://github.com/acme/widget/issues/549".to_string();
    let set = TaskPatch {
        parent: Some(Some(url.clone())),
        ..TaskPatch::default()
    };
    let (updated, _) = update(&ctx, &task.id, &set, false).unwrap();
    assert_eq!(updated.parent.as_deref(), Some(url.as_str()));
    assert_eq!(updated.extra["futureField"], json!({"n": 1}));
    let stored = get(&ctx.state, &ctx.repo.slug, &task.id).unwrap();
    assert_eq!(stored.parent.as_deref(), Some(url.as_str()));

    // A patch that does not name it leaves it.
    let other = TaskPatch {
        note: Some(Some("n".to_string())),
        ..TaskPatch::default()
    };
    assert_eq!(
        update(&ctx, &task.id, &other, false).unwrap().0.parent,
        Some(url)
    );

    let clear = TaskPatch {
        parent: Some(None),
        ..TaskPatch::default()
    };
    let (cleared, _) = update(&ctx, &task.id, &clear, false).unwrap();
    assert_eq!(cleared.parent, None);
    assert_eq!(cleared.extra["futureField"], json!({"n": 1}));
    let text = std::fs::read_to_string(record_path(&ctx, &task.id)).unwrap();
    assert!(!text.contains("\"parent\""), "{text}");
}

#[test]
fn a_patch_passes_over_a_status_or_executor_that_is_blank_or_not_a_string_and_trims_the_rest() {
    let read = |input: serde_json::Value| TaskPatch::from_json(&input);
    assert_eq!(
        read(json!({ "status": " queued " })).unwrap().status,
        Some(Status::Queued)
    );
    for blank in [json!(""), json!("  "), json!(42), json!(null)] {
        let patch = read(json!({ "status": blank, "executor": blank })).unwrap();
        assert_eq!(patch.status, None, "{blank} was taken");
        assert_eq!(patch.executor, None, "{blank} was taken");
    }
    assert_eq!(
        read(json!({ "status": "nope" })).unwrap_err(),
        "no such status: nope"
    );
    assert_eq!(
        read(json!({ "executor": "julse" })).unwrap_err(),
        "no such executor: julse (worker or jules)"
    );
    assert_eq!(
        read(json!({ "executor": "jules" })).unwrap().executor,
        Some(Executor::Jules)
    );
}

#[test]
fn a_patch_passes_over_an_order_or_auto_start_of_the_wrong_type_and_cuts_the_order_to_u32() {
    let read = |input: serde_json::Value| TaskPatch::from_json(&input).unwrap();
    for bad in [json!("3"), json!(-1), json!(5.5)] {
        assert_eq!(read(json!({ "order": bad })).order, None, "{bad} was taken");
    }
    assert_eq!(read(json!({ "order": 4294967297u64 })).order, Some(1));
    assert_eq!(read(json!({ "autoStart": "yes" })).auto_start, None);
    assert_eq!(read(json!({ "autoStart": false })).auto_start, Some(false));
}

#[test]
fn a_patch_ignores_hand_over_and_keys_it_does_not_name() {
    assert_eq!(
        TaskPatch::from_json(&json!({ "handOver": false, "title": "x", "relayed": ["1"] }))
            .unwrap(),
        TaskPatch::default()
    );
    assert_eq!(
        TaskPatch::from_json(&json!([])).unwrap(),
        TaskPatch::default()
    );
}

#[test]
fn the_first_bad_value_in_a_patch_is_the_one_refused() {
    assert_eq!(
        TaskPatch::from_json(&json!({ "status": "nope", "pr": 42 })).unwrap_err(),
        "no such status: nope"
    );
    assert_eq!(
        TaskPatch::from_json(&json!({ "executor": "x", "pr": 42 })).unwrap_err(),
        "no such executor: x (worker or jules)"
    );
}

#[test]
fn a_pr_that_cannot_be_read_says_whether_it_is_a_bare_number_or_not_a_pr() {
    assert!(unreadable_pr("12").contains("bare pull request number"));
    assert!(unreadable_pr("#12").contains("bare pull request number"));
    assert!(unreadable_pr("--web").contains("not a pull request this can read"));
    assert!(unreadable_pr("https://example.com/x").contains("not a pull request"));
}

#[test]
fn empty_title_and_body_produce_nothing() {
    assert!(derive_title(Some("   "), "   \n\n  ").is_none());
}

#[test]
fn a_review_task_is_written_as_the_pr_the_worker_branches_on() {
    assert_eq!(brief_done_when(DoneWhen::Review), "up to a PR");
    assert_eq!(brief_done_when(DoneWhen::Pr), "up to a PR");
    assert_eq!(
        brief_done_when(DoneWhen::Verify),
        "up to handing over for verification"
    );
    assert_eq!(
        brief_done_when(DoneWhen::ReportOnly),
        "investigation only (report and stop)"
    );
}

/// A worktree on `branch`, outside the sandbox's repository: `write_brief` only needs git to
/// name its branch.
fn worktree_on(branch: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    crate::testing::init_repo(dir.path(), branch);
    dir
}

#[test]
fn write_brief_writes_a_task_from_the_record_and_the_settings() {
    let (_sandbox, mut ctx) = hub();
    ctx.settings.issue_keys = json!({"acme/widget": "WID"}).as_object().unwrap().clone();
    ctx.settings.task_sources = vec![json!({"type": "github", "issueRepo": "acme/widget"})];
    ctx.settings.verify = vec!["cargo test".to_string()];
    ctx.settings.copilot_review = crate::kernel::config::CopilotReview::Never;
    let mut task = sample();
    task.issue_url = Some("https://github.com/acme/widget/issues/12".to_string());
    task.done_when = DoneWhen::Review;
    save(&ctx, &task).unwrap();
    let worktree = worktree_on("me/wid-12");

    let written = write_brief(
        &ctx,
        &BriefRequest {
            worktree: worktree.path().display().to_string(),
            base: "origin/main".to_string(),
            out: None,
            language: None,
            of: BriefOf::Task {
                id: task.id.clone(),
                key: None,
                tracker: None,
                parent: None,
            },
        },
    )
    .unwrap();

    assert_eq!(written.branch, "me/wid-12");
    assert!(written.path.ends_with(".claude/task-brief.md"));
    let text = std::fs::read_to_string(&written.path).unwrap();
    assert!(
        text.contains("- Task: WID-12 \"ログインのリトライを調べる\" (github)\n"),
        "{text}"
    );
    assert!(
        text.contains("https://github.com/acme/widget/issues/12"),
        "{text}"
    );
    assert!(text.contains("(branch me/wid-12)"), "{text}");
    assert!(text.contains("- Base branch: origin/main"), "{text}");
    assert!(text.contains("- Done when: up to a PR"), "{text}");
    assert!(text.contains("- Copilot review: never"), "{text}");
    assert!(text.contains("cargo test"), "{text}");
}

#[test]
fn write_brief_writes_a_session_brief_ending_with_the_instruction() {
    let (_sandbox, ctx) = hub();
    let worktree = worktree_on("me/scratch");
    let elsewhere = tempfile::tempdir().unwrap();
    let out = elsewhere.path().join("elsewhere").join("brief.md");
    let request = |instruction: &str| BriefRequest {
        worktree: worktree.path().display().to_string(),
        base: "-".to_string(),
        out: Some(out.display().to_string()),
        language: None,
        of: BriefOf::Session {
            instruction: instruction.to_string(),
        },
    };

    let written = write_brief(&ctx, &request("do this\n## not a header")).unwrap();
    assert_eq!(written.path, out);
    assert!(
        !worktree
            .path()
            .join(".claude")
            .join("task-brief.md")
            .exists()
    );
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("- Task: -"), "{text}");
    assert!(
        text.ends_with("## Instruction\n\ndo this\n## not a header\n"),
        "{text}"
    );

    write_brief(&ctx, &request("-")).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(
        text.ends_with(&format!("{}\n", crate::kernel::brief::NO_INSTRUCTION)),
        "{text}"
    );
}

/// The setting beats the flag, and with neither the line is a dash for the worker to read as
/// "ask the person's own language".
#[test]
fn write_brief_writes_the_language_from_the_setting_then_the_flag_then_a_dash() {
    let (_sandbox, mut ctx) = hub();
    let task = sample();
    save(&ctx, &task).unwrap();
    let worktree = worktree_on("me/wid-12");
    let brief_with = |ctx: &crate::registry::Context, language: Option<&str>| {
        let written = write_brief(
            ctx,
            &BriefRequest {
                worktree: worktree.path().display().to_string(),
                base: "-".to_string(),
                out: None,
                language: language.map(str::to_string),
                of: BriefOf::Task {
                    id: task.id.clone(),
                    key: None,
                    tracker: None,
                    parent: None,
                },
            },
        )
        .unwrap();
        std::fs::read_to_string(written.path).unwrap()
    };

    assert!(brief_with(&ctx, None).contains("- Language: -\n"));
    assert!(brief_with(&ctx, Some("  ")).contains("- Language: -\n"));
    assert!(brief_with(&ctx, Some("Korean")).contains("- Language: Korean\n"));
    ctx.settings.language = Some("ja".to_string());
    assert!(brief_with(&ctx, None).contains("- Language: ja\n"));
    assert!(brief_with(&ctx, Some("Korean")).contains("- Language: ja\n"));
}

#[test]
fn an_unknown_copilot_review_is_written_as_ask() {
    let sandbox = crate::testing::Sandbox::new(
        r#"{"repos": {"acme/widget": {"copilotReview": "sometimes"}}}"#,
    );
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
    let mut task = sample();
    task.done_when = DoneWhen::Pr;
    save(&ctx, &task).unwrap();
    let worktree = worktree_on("me/ask");

    let written = write_brief(
        &ctx,
        &BriefRequest {
            worktree: worktree.path().display().to_string(),
            base: "-".to_string(),
            out: None,
            language: None,
            of: BriefOf::Task {
                id: task.id.clone(),
                key: None,
                tracker: None,
                parent: None,
            },
        },
    )
    .unwrap();
    let text = std::fs::read_to_string(&written.path).unwrap();
    assert!(text.contains("- Copilot review: ask"), "{text}");
}

/// A saved record to edit, with a stale `updatedAt` so a stamp shows.
fn saved_sample(ctx: &crate::registry::Context) -> Task {
    let mut task = sample();
    task.updated_at = "20200101T000000Z".to_string();
    save(ctx, &task).unwrap();
    task
}

fn on_disk(ctx: &crate::registry::Context, id: &str) -> Task {
    get(&ctx.state, &ctx.repo.slug, id).unwrap()
}

#[test]
fn edit_keep_returns_the_value_and_writes_nothing() {
    let (_sandbox, ctx) = hub();
    let task = saved_sample(&ctx);
    let edited = edit(&ctx, &task.id, |t| {
        t.note = Some("not saved".to_string());
        Ok(Edit::Keep(7))
    })
    .unwrap();
    assert_eq!(edited.value, 7);
    assert_eq!(edited.saved, Ok(()));
    assert_eq!(on_disk(&ctx, &task.id), task);
}

#[test]
fn edit_write_saves_the_record_with_a_fresh_stamp() {
    let (_sandbox, ctx) = hub();
    let task = saved_sample(&ctx);
    let edited = edit(&ctx, &task.id, |t| {
        t.note = Some("kept".to_string());
        Ok(Edit::Write("done"))
    })
    .unwrap();
    assert_eq!(edited.value, "done");
    assert_eq!(edited.saved, Ok(()));
    let back = on_disk(&ctx, &task.id);
    assert_eq!(back.note.as_deref(), Some("kept"));
    assert_ne!(back.updated_at, task.updated_at);
}

#[test]
fn edit_returns_the_closures_error_and_writes_nothing() {
    let (_sandbox, ctx) = hub();
    let task = saved_sample(&ctx);
    let err = edit(&ctx, &task.id, |t| -> Result<Edit<()>, String> {
        t.note = Some("not saved".to_string());
        Err("refused".to_string())
    })
    .err();
    assert_eq!(err.as_deref(), Some("refused"));
    assert_eq!(on_disk(&ctx, &task.id), task);
    assert!(edit(&ctx, "no-such-task", |_| Ok(Edit::Keep(()))).is_err());
}

#[test]
fn edit_returns_a_failed_save_beside_the_value() {
    let (_sandbox, ctx) = hub();
    let task = saved_sample(&ctx);
    let path = record_path(&ctx, &task.id);
    let edited = edit(&ctx, &task.id, |_| {
        // Something non-empty where the record was, so the rename over it fails. Not a
        // permission change: that does not stop root.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(path.join("x")).unwrap();
        Ok(Edit::Write("session-1"))
    })
    .unwrap();
    assert_eq!(edited.value, "session-1");
    let error = edited.saved.unwrap_err();
    assert!(error.starts_with("cannot write "), "{error}");
    assert!(error.contains(&task.id), "{error}");
}

fn open_pr_task(ctx: &crate::registry::Context) -> Task {
    let mut task = sample();
    task.pr = Some("https://github.com/acme/widget/pull/7".to_string());
    task.status = Status::Pr;
    save(ctx, &task).unwrap();
    task
}

fn refreshed(ctx: &crate::registry::Context, id: &str, status: PrStatus) -> Task {
    let task = get(&ctx.state, &ctx.repo.slug, id).unwrap();
    let state = if status.state == "merged" {
        PrState::Merged
    } else {
        PrState::Open
    };
    let mut checked = apply(ctx, vec![task], vec![(state, Some(status))]);
    checked.remove(0).task
}

#[test]
fn a_refresh_stamps_the_time_a_prs_turn_changed_and_only_then() {
    let (_sandbox, ctx) = hub();
    let id = open_pr_task(&ctx).id;
    assert_eq!(
        get(&ctx.state, &ctx.repo.slug, &id).unwrap().pr_turn_at,
        None
    );

    // The first read is a turn the record had none of.
    let first = refreshed(&ctx, &id, pr("open", "none", 0, 1));
    let at = first.pr_turn_at.clone().expect("the first turn is stamped");

    // Same turn (checks), different facts: nothing is stamped.
    let mut quiet = refreshed(&ctx, &id, pr("open", "none", 0, 2));
    assert_eq!(quiet.pr_turn_at.as_deref(), Some(at.as_str()));
    assert_eq!(quiet.pr_status.as_ref().unwrap().ci.pending, 2);

    // A different turn moves the stamp, whatever it is stamped with.
    quiet.pr_turn_at = Some("20200101T000000Z".to_string());
    store::save(&ctx, &quiet).unwrap();
    let failed = refreshed(&ctx, &id, pr("open", "none", 1, 0));
    assert_ne!(failed.pr_turn_at.as_deref(), Some("20200101T000000Z"));
    assert!(failed.pr_turn_at.is_some());
    assert_eq!(
        get(&ctx.state, &ctx.repo.slug, &id).unwrap().pr_turn_at,
        failed.pr_turn_at
    );
}

#[test]
fn a_record_whose_turn_was_read_before_the_stamp_existed_gets_none_until_the_turn_changes() {
    let (_sandbox, ctx) = hub();
    let mut task = open_pr_task(&ctx);
    task.pr_status = Some(pr("open", "none", 1, 0));
    save(&ctx, &task).unwrap();
    // The same turn again: the summary may differ, the turn does not.
    let same = refreshed(&ctx, &task.id, pr("open", "none", 2, 0));
    assert_eq!(same.pr_turn_at, None);
    let moved = refreshed(&ctx, &task.id, pr("open", "changes", 0, 0));
    assert!(moved.pr_turn_at.is_some());
}

#[test]
fn a_merge_stamps_its_turn_and_another_pr_clears_the_stamp_with_the_summary() {
    let (_sandbox, ctx) = hub();
    let task = open_pr_task(&ctx);
    refreshed(&ctx, &task.id, pr("open", "approved", 0, 0));
    let merged = refreshed(&ctx, &task.id, pr("merged", "approved", 0, 0));
    assert_eq!(merged.status, Status::Done);
    assert!(merged.pr_turn_at.is_some());

    let other = open_pr_task(&ctx);
    refreshed(&ctx, &other.id, pr("open", "none", 1, 0));
    let patch = TaskPatch {
        pr: Some(Some("https://github.com/acme/widget/pull/8".to_string())),
        ..TaskPatch::default()
    };
    let (updated, _) = update(&ctx, &other.id, &patch, false).unwrap();
    assert_eq!((updated.pr_status, updated.pr_turn_at), (None, None));
}

fn park_patch(reason: &str, text: Option<&str>) -> TaskPatch {
    TaskPatch {
        parked: Some(Some(ParkRequest {
            reason: reason.to_string(),
            text: text.map(str::to_string),
        })),
        ..TaskPatch::default()
    }
}

fn unpark_patch() -> TaskPatch {
    TaskPatch {
        parked: Some(None),
        ..TaskPatch::default()
    }
}

#[test]
fn a_park_in_a_patch_is_set_by_an_object_cleared_by_null_and_left_alone_when_absent() {
    let parked = |input: serde_json::Value| TaskPatch::from_json(&input).map(|p| p.parked);
    assert_eq!(parked(json!({})).unwrap(), None);
    assert_eq!(parked(json!({ "parked": null })).unwrap(), Some(None));
    // The client's `since` is not read; the text is trimmed and blank is none.
    assert_eq!(
        parked(json!({ "parked": { "reason": " pdm ", "text": " 資料待ち ", "since": "20200101T000000Z" } }))
            .unwrap(),
        Some(Some(ParkRequest {
            reason: "pdm".to_string(),
            text: Some("資料待ち".to_string()),
        }))
    );
    assert_eq!(
        parked(json!({ "parked": { "reason": "review", "text": "  " } })).unwrap(),
        Some(Some(ParkRequest {
            reason: "review".to_string(),
            text: None,
        }))
    );
}

#[test]
fn a_park_that_cannot_be_written_is_refused_before_any_record_is_touched() {
    let parked = |input: serde_json::Value| TaskPatch::from_json(&input).map(|p| p.parked);
    for bad in [
        json!({ "parked": {} }),
        json!({ "parked": { "reason": "" } }),
        json!({ "parked": { "reason": "nope" } }),
        json!({ "parked": { "reason": "other" } }),
        json!({ "parked": { "reason": "other", "text": " " } }),
        json!({ "parked": "pdm" }),
        json!({ "parked": 1 }),
        json!({ "parked": ["pdm"] }),
    ] {
        assert!(parked(bad.clone()).is_err(), "{bad} was taken");
    }
    assert!(
        parked(json!({ "parked": { "reason": "nope" } }))
            .unwrap_err()
            .contains("pdm, design, review, merge-timing, other")
    );
    assert!(parked(json!({ "parked": { "reason": "other", "text": "法務" } })).is_ok());

    let (_sandbox, ctx) = hub();
    let task = sample();
    save(&ctx, &task).unwrap();
    let before = std::fs::read_to_string(record_path(&ctx, &task.id)).unwrap();
    assert!(update(&ctx, &task.id, &park_patch("other", None), false).is_err());
    assert!(update(&ctx, &task.id, &park_patch("nope", None), false).is_err());
    assert_eq!(
        std::fs::read_to_string(record_path(&ctx, &task.id)).unwrap(),
        before
    );
}

#[test]
fn parking_stamps_since_once_keeps_it_for_the_same_reason_and_text_and_unparking_removes_the_key() {
    let (_sandbox, ctx) = hub();
    let task = sample();
    save(&ctx, &task).unwrap();

    let (first, _) = update(&ctx, &task.id, &park_patch("pdm", Some("資料待ち")), false).unwrap();
    let park = first.parked.clone().expect("parked");
    assert_eq!(
        (park.reason.as_str(), park.text.as_deref()),
        ("pdm", Some("資料待ち"))
    );
    assert!(stamp_ok(&park.since), "{}", park.since);
    // A direct caller's untrimmed words are stored trimmed, and blank text is none.
    let (trimmed, _) = update(&ctx, &task.id, &park_patch(" pdm ", Some("  ")), false).unwrap();
    let t = trimmed.parked.unwrap();
    assert_eq!((t.reason.as_str(), t.text), ("pdm", None));
    update(&ctx, &task.id, &park_patch("pdm", Some("資料待ち")), false).unwrap();

    // A retry of the same park keeps when it began, even if a second has passed.
    let mut aged = get(&ctx.state, &ctx.repo.slug, &task.id).unwrap();
    aged.parked.as_mut().unwrap().since = "20200101T000000Z".to_string();
    save(&ctx, &aged).unwrap();
    let (same, _) = update(&ctx, &task.id, &park_patch("pdm", Some("資料待ち")), false).unwrap();
    assert_eq!(same.parked.unwrap().since, "20200101T000000Z");
    // Another reason or text is a new park.
    let (other, _) = update(&ctx, &task.id, &park_patch("design", None), false).unwrap();
    assert_ne!(other.parked.as_ref().unwrap().since, "20200101T000000Z");
    assert_eq!(other.parked.unwrap().text, None);

    let (gone, _) = update(&ctx, &task.id, &unpark_patch(), false).unwrap();
    assert_eq!(gone.parked, None);
    let text = std::fs::read_to_string(record_path(&ctx, &task.id)).unwrap();
    assert!(!text.contains("parked"), "{text}");
    // A patch that says nothing about it leaves a park alone.
    update(&ctx, &task.id, &park_patch("review", None), false).unwrap();
    let (kept, _) = update(
        &ctx,
        &task.id,
        &TaskPatch {
            note: Some(Some("n".to_string())),
            ..TaskPatch::default()
        },
        false,
    )
    .unwrap();
    assert_eq!(kept.parked.unwrap().reason, "review");
}

/// `YYYYMMDDTHHMMSSZ`.
fn stamp_ok(s: &str) -> bool {
    s.len() == 16 && s.ends_with('Z') && s.as_bytes()[8] == b'T'
}

#[test]
fn a_key_this_binary_does_not_know_in_a_park_or_beside_it_survives_a_park_and_an_unpark() {
    let (_sandbox, ctx) = hub();
    let task = sample();
    let mut raw = serde_json::to_value(&task).unwrap();
    raw["futureField"] = json!(1);
    raw["parked"] =
        json!({ "reason": "pdm", "since": "20260101T000000Z", "futureKey": { "n": 2 } });
    std::fs::create_dir_all(dir(&ctx.state, &ctx.repo.slug)).unwrap();
    std::fs::write(record_path(&ctx, &task.id), raw.to_string()).unwrap();

    // A save that keeps the park keeps what is inside it.
    update(
        &ctx,
        &task.id,
        &TaskPatch {
            note: Some(Some("n".to_string())),
            ..TaskPatch::default()
        },
        false,
    )
    .unwrap();
    let back: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(record_path(&ctx, &task.id)).unwrap())
            .unwrap();
    assert_eq!(back["parked"]["futureKey"], json!({ "n": 2 }));
    assert_eq!(back["futureField"], json!(1));

    update(&ctx, &task.id, &unpark_patch(), false).unwrap();
    let back: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(record_path(&ctx, &task.id)).unwrap())
            .unwrap();
    assert!(back.get("parked").is_none(), "{back}");
    assert_eq!(back["futureField"], json!(1));
}

#[test]
fn a_finished_task_is_never_parked_and_writing_done_or_cancelled_clears_the_park() {
    let (_sandbox, ctx) = hub();
    for finished in [Status::Done, Status::Cancelled] {
        let mut task = sample();
        task.id = format!("{}-{}", task.id, finished.as_str());
        save(&ctx, &task).unwrap();
        update(&ctx, &task.id, &park_patch("pdm", None), false).unwrap();
        let (done, _) = update(
            &ctx,
            &task.id,
            &TaskPatch {
                status: Some(finished),
                ..TaskPatch::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(done.parked, None);
        assert!(
            !std::fs::read_to_string(record_path(&ctx, &task.id))
                .unwrap()
                .contains("parked")
        );
        let refused = update(&ctx, &task.id, &park_patch("pdm", None), false).unwrap_err();
        assert!(
            refused.contains("finished task cannot be parked"),
            "{refused}"
        );
        // Parking and finishing in one patch is refused too.
        let mut both = park_patch("pdm", None);
        both.status = Some(finished);
        assert!(update(&ctx, &task.id, &both, false).is_err());
    }
}

#[test]
fn a_blank_park_on_disk_and_a_park_on_a_finished_task_read_as_none() {
    let mut task = sample();
    assert!(task.park().is_none());
    task.parked = Some(Park {
        reason: "  ".to_string(),
        text: None,
        since: String::new(),
        extra: serde_json::Map::new(),
    });
    assert!(task.park().is_none());
    task.parked.as_mut().unwrap().reason = "pdm".to_string();
    assert_eq!(task.park().map(|p| p.reason.as_str()), Some("pdm"));
    task.status = Status::Done;
    assert!(task.park().is_none());

    // `null` on disk is none, and a park with no `since` still loads.
    let raw = |parked: serde_json::Value| {
        let mut v = serde_json::to_value(sample()).unwrap();
        v["parked"] = parked;
        serde_json::from_value::<Task>(v).unwrap()
    };
    assert!(raw(json!(null)).park().is_none());
    assert!(raw(json!({ "reason": "" })).park().is_none());
    assert_eq!(raw(json!({ "reason": "pdm" })).park().unwrap().since, "");
}

#[test]
fn a_refresh_that_moves_a_parked_task_to_done_clears_the_park() {
    let (_sandbox, ctx) = hub();
    let task = open_pr_task(&ctx);
    update(&ctx, &task.id, &park_patch("merge-timing", None), false).unwrap();
    let merged = refreshed(&ctx, &task.id, pr("merged", "approved", 0, 0));
    assert_eq!(merged.status, Status::Done);
    assert_eq!(merged.parked, None);
    let text = std::fs::read_to_string(record_path(&ctx, &task.id)).unwrap();
    assert!(!text.contains("parked"), "{text}");
}

#[test]
fn a_request_not_yet_read_says_so_first_and_keeps_every_other_line() {
    let mut task = sample();
    let plain = render_request(&task);
    assert!(!plain.contains("## Read first"), "{plain}");
    task.needs_reading = true;
    let body = render_request(&task);
    let marker = body
        .lines()
        .find(|l| l.starts_with("## Read first"))
        .unwrap();
    assert!(
        body.contains(&format!("## task          {}\n{marker}\n", task.id)),
        "{body}"
    );
    // The old lines stay under the marker, so a hub that does not know it still asks.
    assert_eq!(body.replace(&format!("{marker}\n"), ""), plain);
}

#[test]
fn a_task_the_hub_has_not_read_is_never_picked_even_when_it_may_start_unasked() {
    let mut unread = sample();
    unread.id = "1".to_string();
    unread.status = Status::Queued;
    unread.auto_start = true;
    unread.needs_reading = true;
    let mut ready = sample();
    ready.id = "2".to_string();
    ready.status = Status::Queued;

    let mut gated = std::collections::HashSet::new();
    let picked = next(vec![unread.clone(), ready.clone()], &gated);
    assert_eq!(picked.task, Some(ready.clone()));
    assert_eq!(picked.needs_dispatch_gate, vec![unread.clone()]);

    gated.insert("1".to_string());
    assert!(next(vec![unread], &gated).needs_dispatch_gate.is_empty());
}

#[test]
fn a_patch_writes_what_the_hub_read_and_clears_the_marker() {
    let (_sandbox, ctx) = hub();
    let mut task = sample();
    task.needs_reading = true;
    task.title_pending = true;
    save(&ctx, &task).unwrap();
    let patch = TaskPatch {
        kind: Some(Kind::FileAndStart),
        done_when: Some(DoneWhen::Verify),
        stop_at: Some(StopAt::All),
        issue_url: Some(Some("https://github.com/acme/widget/issues/9".to_string())),
        worktree_name: Some(Some("login-crash".to_string())),
        title: Some("  Fix the login crash ".to_string()),
        read: true,
        ..TaskPatch::default()
    };
    let (updated, _) = update(&ctx, &task.id, &patch, false).unwrap();
    assert_eq!(updated.kind, Kind::FileAndStart);
    assert_eq!(updated.done_when, DoneWhen::Verify);
    assert_eq!(updated.stop_at, StopAt::All);
    assert_eq!(
        updated.issue_url.as_deref(),
        Some("https://github.com/acme/widget/issues/9")
    );
    assert_eq!(updated.worktree_name.as_deref(), Some("login-crash"));
    assert_eq!(updated.title, "Fix the login crash");
    assert!(!updated.title_pending);
    assert!(!updated.needs_reading);
    let text = std::fs::read_to_string(record_path(&ctx, &task.id)).unwrap();
    assert!(!text.contains("needsReading"), "{text}");

    let clear = TaskPatch {
        issue_url: Some(None),
        worktree_name: Some(None),
        ..TaskPatch::default()
    };
    let (cleared, _) = update(&ctx, &task.id, &clear, false).unwrap();
    assert_eq!(cleared.issue_url, None);
    assert_eq!(cleared.worktree_name, None);
}

#[test]
fn a_patch_with_a_value_the_hub_could_not_quote_is_refused_before_any_write() {
    let (_sandbox, ctx) = hub();
    let mut task = sample();
    task.needs_reading = true;
    save(&ctx, &task).unwrap();
    let bad = [
        TaskPatch {
            worktree_name: Some(Some("a b".to_string())),
            ..TaskPatch::default()
        },
        TaskPatch {
            issue_url: Some(Some("https://x/'; id".to_string())),
            ..TaskPatch::default()
        },
        TaskPatch {
            base: Some(Some("main'; id".to_string())),
            ..TaskPatch::default()
        },
        TaskPatch {
            base: Some(Some("-flag".to_string())),
            ..TaskPatch::default()
        },
        TaskPatch {
            title: Some("  ".to_string()),
            ..TaskPatch::default()
        },
    ];
    for patch in bad {
        let patch = TaskPatch {
            read: true,
            ..patch
        };
        assert!(update(&ctx, &task.id, &patch, false).is_err(), "{patch:?}");
    }
    assert_eq!(get(&ctx.state, &ctx.repo.slug, &task.id).unwrap(), task);
}

#[test]
fn a_new_request_that_needs_reading_is_held_and_not_checked_for_a_kind() {
    let (_sandbox, ctx) = hub();
    // No issue, no worktree name, a kind that would want one: not the server's to demand.
    let new = NewTask {
        body: "https://github.com/acme/widget/issues/9".to_string(),
        auto_start: true,
        needs_reading: true,
        status: Status::Queued,
        ..NewTask::default()
    };
    let (task, handed) = create(&ctx, new, false).unwrap();
    assert!(task.needs_reading);
    assert!(!task.auto_start);
    assert_eq!(task.title, "https://github.com/acme/widget/issues/9");
    assert!(task.issue_snapshot.is_none());
    assert!(handed.is_none());

    // A base is the caller's to choose, as it always was.
    let odd = NewTask {
        base: Some("HEAD~1".to_string()),
        ..NewTask::default()
    };
    assert!(check_typed(&odd).is_ok());
    let json = NewTask::from_json(&json!({"body": "x", "needsReading": true})).unwrap();
    assert!(json.needs_reading);
    assert!(NewTask::from_json(&json!({"body": "x", "needsReading": "yes"})).is_err());
}

#[test]
fn a_title_is_its_first_non_blank_line_cut_to_an_issue_titles_length() {
    assert_eq!(
        one_line_title("\n  \n Fix it \nmore").as_deref(),
        Some("Fix it")
    );
    assert_eq!(one_line_title(" \n "), None);
    let long = "x".repeat(ISSUE_TITLE_CAP + 50);
    assert_eq!(
        one_line_title(&long).unwrap().chars().count(),
        ISSUE_TITLE_CAP
    );
    let (_sandbox, ctx) = hub();
    let task = sample();
    save(&ctx, &task).unwrap();
    let patch = TaskPatch {
        title: Some("first\nsecond".to_string()),
        ..TaskPatch::default()
    };
    assert_eq!(
        update(&ctx, &task.id, &patch, false).unwrap().0.title,
        "first"
    );
}

#[test]
fn a_read_record_has_to_be_startable_and_only_reading_clears_the_marker() {
    let (_sandbox, ctx) = hub();
    let mut task = sample();
    task.kind = Kind::FileAndStart;
    task.needs_reading = true;
    task.auto_start = false;
    save(&ctx, &task).unwrap();
    // Nothing names the worktree: refused, and the record is as it was.
    let read = TaskPatch {
        read: true,
        ..TaskPatch::default()
    };
    assert!(update(&ctx, &task.id, &read, false).is_err());
    // Starting without asking is refused for an unread record, startable-looking or not.
    let approve = TaskPatch {
        auto_start: Some(true),
        ..TaskPatch::default()
    };
    assert!(update(&ctx, &task.id, &approve, false).is_err());
    let looks_ready = TaskPatch {
        issue_url: Some(Some("https://github.com/acme/widget/issues/9".to_string())),
        auto_start: Some(true),
        ..TaskPatch::default()
    };
    let why = update(&ctx, &task.id, &looks_ready, false).unwrap_err();
    assert!(why.contains("has not read this request"), "{why}");
    assert_eq!(get(&ctx.state, &ctx.repo.slug, &task.id).unwrap(), task);

    let named = TaskPatch {
        worktree_name: Some(Some("login-crash".to_string())),
        ..TaskPatch::default()
    };
    let (still, _) = update(&ctx, &task.id, &named, false).unwrap();
    assert!(
        still.needs_reading,
        "writing the reading is not confirming it"
    );
    // The answer is one call: the confirmation and the go-ahead together.
    let (both, _) = update(
        &ctx,
        &task.id,
        &TaskPatch {
            read: true,
            auto_start: Some(true),
            ..TaskPatch::default()
        },
        false,
    )
    .unwrap();
    assert!(both.auto_start && !both.needs_reading);

    // A start needs its issue.
    let mut start = sample();
    start.id = "2".to_string();
    start.kind = Kind::Start;
    start.needs_reading = true;
    save(&ctx, &start).unwrap();
    let read = TaskPatch {
        read: true,
        ..TaskPatch::default()
    };
    assert!(update(&ctx, &start.id, &read, false).is_err());
    // A task that was never unread is not held to any of it.
    let mut plain = sample();
    plain.id = "3".to_string();
    plain.kind = Kind::Start;
    save(&ctx, &plain).unwrap();
    assert!(update(&ctx, &plain.id, &approve, false).is_ok());
}

#[test]
fn a_base_is_held_to_the_branch_rule_only_when_the_hub_reads_the_record() {
    let (_sandbox, ctx) = hub();
    let mut task = sample();
    task.kind = Kind::Investigate;
    task.needs_reading = true;
    task.worktree_name = Some("look-into-it".to_string());
    save(&ctx, &task).unwrap();
    // A plain update takes any value, as it always did.
    let odd = TaskPatch {
        base: Some(Some("x; id".to_string())),
        ..TaskPatch::default()
    };
    assert!(update(&ctx, &task.id, &odd, false).is_ok());
    // The read refuses it and leaves the record unread.
    let read = TaskPatch {
        read: true,
        ..TaskPatch::default()
    };
    let why = update(&ctx, &task.id, &read, false).unwrap_err();
    assert!(why.contains("x; id"), "{why}");
    assert!(
        get(&ctx.state, &ctx.repo.slug, &task.id)
            .unwrap()
            .needs_reading
    );
    // Clearing the base lets it through.
    let clear = TaskPatch {
        base: Some(None),
        ..TaskPatch::default()
    };
    update(&ctx, &task.id, &clear, false).unwrap();
    assert!(
        !update(&ctx, &task.id, &read, false)
            .unwrap()
            .0
            .needs_reading
    );
}

#[test]
fn what_the_hub_reads_cannot_be_rewritten_once_the_task_has_started() {
    let (_sandbox, ctx) = hub();
    let mut task = sample();
    task.status = Status::Dispatched;
    save(&ctx, &task).unwrap();
    let patch = TaskPatch {
        title: Some("new".to_string()),
        ..TaskPatch::default()
    };
    assert!(update(&ctx, &task.id, &patch, false).is_err());
    assert_eq!(get(&ctx.state, &ctx.repo.slug, &task.id).unwrap(), task);
    for status in [Status::Backlog, Status::Queued] {
        let mut open = sample();
        open.id = format!("open-{}", status.as_str());
        open.status = status;
        save(&ctx, &open).unwrap();
        assert!(update(&ctx, &open.id, &patch, false).is_ok());
    }
}
