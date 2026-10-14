use super::*;
use crate::mail::RepoHubState;
use crate::registry::context_of;
use crate::testing::Sandbox;

const SLUG: &str = "acme-widget-ALPHA-9";
const URL: &str = "https://github.com/acme/widget/issues/9";

fn ctx() -> Context {
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: Some("ALPHA-9".to_string()),
        slug: SLUG.to_string(),
        hub_name: "adjutant-acme-widget-ALPHA-9".to_string(),
        nwo_source: "dirname",
    };
    let mut ctx = context_of(repo).unwrap();
    ctx.settings.issue_keys = serde_json::json!({"acme/widget": "ALPHA"})
        .as_object()
        .unwrap()
        .clone();
    ctx
}

fn hub(parent: bool, key: Option<&str>) -> RepoHub {
    RepoHub {
        id: "hub-ALPHA-9".to_string(),
        parent,
        key: key.map(str::to_string),
        name: "adjutant-acme-widget-ALPHA-9".to_string(),
        title: None,
        title_state: None,
        slug: SLUG.to_string(),
        state: RepoHubState {
            present: false,
            stale: false,
            pid: None,
            started_at: None,
        },
        inbox_count: 0,
        unseen: 0,
        seen: 0,
        oldest_unseen_at: None,
        inbox: Vec::new(),
        children: 0,
    }
}

fn found(title: &str) -> Outcome {
    Outcome::Found {
        title: title.to_string(),
        counts: Some((5, 2)),
    }
}

fn state_of_look(titles: &HubTitles, ctx: &Context, now: i64) -> Looked {
    titles.look_at(ctx, &hub(true, Some("ALPHA-9")), now)
}

fn file(ctx: &Context) -> Option<Value> {
    crate::infra::fs::read_json(&file_of(&ctx.state, SLUG))
}

#[test]
fn a_failed_read_is_tried_again_after_30_seconds_then_2_minutes_then_10() {
    let secs = |n| retry_after(n).as_secs();
    assert_eq!(
        [secs(1), secs(2), secs(3), secs(4), secs(40)],
        [30, 120, 600, 600, 600]
    );
    // A count of zero is not a failure yet; it must not index before the first step.
    assert_eq!(secs(0), 30);
}

#[test]
fn a_file_from_before_the_counts_reads_as_read_without_them() {
    let _sandbox = Sandbox::empty();
    let ctx = ctx();
    let old = serde_json::json!({ "url": URL, "title": "Old title" });
    crate::infra::fs::write_json(&file_of(&ctx.state, SLUG), &old).unwrap();
    let titles = HubTitles::default();
    let looked = state_of_look(&titles, &ctx, 1_000);
    assert_eq!(looked.title.as_deref(), Some("Old title"));
    let state = looked.state.unwrap();
    assert_eq!(state.status, HubTitleStatus::Read);
    assert_eq!(
        (state.total, state.completed, state.read_at),
        (None, None, None)
    );
    assert_eq!(state.parent_key.as_deref(), Some("acme/widget#9"));
    assert_eq!(looked.ask, None);
}

#[test]
fn keeping_a_read_writes_the_counts_and_keeps_keys_it_does_not_know() {
    let _sandbox = Sandbox::empty();
    let ctx = ctx();
    let path = file_of(&ctx.state, SLUG);
    let old = serde_json::json!({ "url": URL, "title": "Old", "future": [1] });
    crate::infra::fs::write_json(&path, &old).unwrap();
    keep(&path, URL, "New", Some((5, 2)), "20261014T010203Z").unwrap();
    let kept = crate::infra::fs::read_json(&path).unwrap();
    assert_eq!(
        kept,
        serde_json::json!({
            "url": URL, "title": "New", "total": 5, "completed": 2,
            "readAt": "20261014T010203Z", "future": [1],
        })
    );
    // Counts of the issue before are not the new one's; a read without counts keeps this one's.
    keep(&path, URL, "Newer", None, "20261014T020202Z").unwrap();
    assert_eq!(file(&ctx).unwrap()["total"], 5);
    keep(
        &path,
        "https://github.com/acme/widget/issues/10",
        "Other",
        None,
        "x",
    )
    .unwrap();
    let kept = crate::infra::fs::read_json(&path).unwrap();
    assert!(kept.get("total").is_none() && kept.get("readAt").is_none());
    assert_eq!(kept["future"], serde_json::json!([1]));
}

#[test]
fn a_failed_read_leaves_the_file_and_is_tried_again_on_the_schedule() {
    let _sandbox = Sandbox::empty();
    let ctx = ctx();
    let titles = HubTitles::default();
    let first = state_of_look(&titles, &ctx, 1_000);
    assert_eq!(first.ask.as_deref(), Some(URL));
    assert_eq!(
        first.state.as_ref().unwrap().status,
        HubTitleStatus::Pending
    );
    assert!(first.state.unwrap().reading);
    // Asked once: another poll while the read is out does not ask again.
    assert_eq!(state_of_look(&titles, &ctx, 1_001).ask, None);

    let fail = |_: &Context, _: &str| Outcome::Failed("boom".to_string());
    for (round, wait) in [(1, 30), (2, 120), (3, 600), (4, 600)] {
        titles.ask_with(&ctx, SLUG, &[], URL, &fail);
        // `ask_with` stamps with the real clock; look at what it set instead.
        let retry_at = titles.seen.lock().unwrap()[SLUG].failed.as_ref().unwrap().1;
        let now = retry_at - wait;
        let waiting = state_of_look(&titles, &ctx, now);
        let state = waiting.state.unwrap();
        assert_eq!(state.status, HubTitleStatus::Failed, "round {round}");
        assert_eq!(state.why.as_deref(), Some("boom"));
        assert_eq!(state.retry_at, Some(retry_at));
        assert_eq!(waiting.ask, None);
        assert!(file(&ctx).is_none());
        // Once the time has come the read is started again.
        let due = state_of_look(&titles, &ctx, retry_at);
        assert_eq!(due.ask.as_deref(), Some(URL));
    }
}

#[test]
fn an_issue_github_does_not_have_is_not_asked_about_again() {
    let _sandbox = Sandbox::empty();
    let ctx = ctx();
    let titles = HubTitles::default();
    assert!(state_of_look(&titles, &ctx, 1).ask.is_some());
    titles.ask_with(&ctx, SLUG, &[], URL, &|_, _| Outcome::NotFound);
    let looked = state_of_look(&titles, &ctx, 1_000_000);
    let state = looked.state.unwrap();
    assert_eq!(state.status, HubTitleStatus::Missing);
    assert_eq!(state.missing, Some(MissingWhy::NotFound));
    assert_eq!(looked.ask, None);
}

#[test]
fn another_issue_starts_the_failures_over() {
    let _sandbox = Sandbox::empty();
    let mut ctx = ctx();
    let titles = HubTitles::default();
    assert!(state_of_look(&titles, &ctx, 1).ask.is_some());
    titles.ask_with(&ctx, SLUG, &[], URL, &|_, _| {
        Outcome::Failed("x".to_string())
    });
    titles.ask_with(&ctx, SLUG, &[], URL, &|_, _| {
        Outcome::Failed("x".to_string())
    });
    assert_eq!(titles.seen.lock().unwrap()[SLUG].failures, 2);
    // The key now stands for another repository's issue.
    ctx.settings.issue_keys = serde_json::json!({"acme/other": "ALPHA"})
        .as_object()
        .unwrap()
        .clone();
    let looked = state_of_look(&titles, &ctx, 1);
    assert_eq!(
        looked.ask.as_deref(),
        Some("https://github.com/acme/other/issues/9")
    );
    assert_eq!(titles.seen.lock().unwrap()[SLUG].failures, 0);
}

#[test]
fn a_hub_that_says_nothing_of_its_issue_says_why() {
    let _sandbox = Sandbox::empty();
    let ctx = ctx();
    let titles = HubTitles::default();
    let state_for = |hub: &RepoHub| titles.look_at(&ctx, hub, 1).state;
    assert_eq!(state_for(&hub(false, None)), None);
    let no_key = state_for(&hub(true, Some("  "))).unwrap();
    assert_eq!(
        (no_key.status, no_key.missing),
        (HubTitleStatus::Missing, Some(MissingWhy::NoKey))
    );
    let no_issue = state_for(&hub(true, Some("BETA-3"))).unwrap();
    assert_eq!(no_issue.missing, Some(MissingWhy::NoIssue));
    assert_eq!(no_issue.parent_key, None);
}

#[test]
fn a_title_read_by_another_board_shows_without_a_read_of_our_own() {
    let _sandbox = Sandbox::empty();
    let ctx = ctx();
    let titles = HubTitles::default();
    assert!(state_of_look(&titles, &ctx, 1).ask.is_some());
    keep(
        &file_of(&ctx.state, SLUG),
        URL,
        "Kept",
        Some((3, 1)),
        "20261014T000000Z",
    )
    .unwrap();
    let looked = state_of_look(&titles, &ctx, 2);
    assert_eq!(looked.title.as_deref(), Some("Kept"));
    let state = looked.state.unwrap();
    assert_eq!((state.total, state.completed), (Some(3), Some(1)));
    assert_eq!(state.read_at.as_deref(), Some("20261014T000000Z"));
    // The background read that was out is still said to be out.
    assert!(state.reading);
}

#[test]
fn reading_now_keeps_the_title_and_the_counts_and_says_them() {
    let _sandbox = Sandbox::empty();
    let ctx = ctx();
    let titles = HubTitles::default();
    let got = titles
        .read_now_with(&ctx, SLUG, "ALPHA-9", &|_, url| {
            assert_eq!(url, URL);
            found("Fresh")
        })
        .unwrap();
    let ReadNow::Read {
        title,
        total,
        completed,
        read_at,
    } = got
    else {
        panic!("{got:?}");
    };
    assert_eq!(
        (title.as_str(), total, completed),
        ("Fresh", Some(5), Some(2))
    );
    let kept = file(&ctx).unwrap();
    assert_eq!(kept["title"], "Fresh");
    assert_eq!(kept["readAt"], read_at.as_str());
    assert!(!titles.seen.lock().unwrap()[SLUG].asking);
}

#[test]
fn reading_now_while_a_read_is_out_is_busy_and_reads_nothing() {
    let _sandbox = Sandbox::empty();
    let ctx = ctx();
    let titles = HubTitles::default();
    assert!(state_of_look(&titles, &ctx, 1).ask.is_some());
    let got = titles.read_now_with(&ctx, SLUG, "ALPHA-9", &|_, _| panic!("read while busy"));
    assert_eq!(got, Ok(ReadNow::Busy));
    assert!(titles.seen.lock().unwrap()[SLUG].asking);
}

#[test]
fn reading_now_says_when_there_is_nothing_to_read_or_it_failed() {
    let _sandbox = Sandbox::empty();
    let ctx = ctx();
    let titles = HubTitles::default();
    let never = |_: &Context, _: &str| -> Outcome { panic!("nothing to read") };
    assert_eq!(
        titles.read_now_with(&ctx, SLUG, " ", &never),
        Ok(ReadNow::Missing(MissingWhy::NoKey))
    );
    assert_eq!(
        titles.read_now_with(&ctx, SLUG, "BETA-3", &never),
        Ok(ReadNow::Missing(MissingWhy::NoIssue))
    );
    assert_eq!(
        titles.read_now_with(&ctx, SLUG, "ALPHA-9", &|_, _| Outcome::NotFound),
        Ok(ReadNow::NotFound)
    );
    assert_eq!(
        titles.read_now_with(&ctx, SLUG, "ALPHA-9", &|_, _| Outcome::Failed(
            "nope".into()
        )),
        Err("nope".to_string())
    );
    // The failure counts for the background read, which waits its first step.
    assert_eq!(titles.seen.lock().unwrap()[SLUG].failures, 1);
}

#[test]
fn a_title_that_could_not_be_written_down_is_held_and_not_read_again() {
    let _sandbox = Sandbox::empty();
    let ctx = ctx();
    // A file where the directory should be: the write fails.
    std::fs::create_dir_all(&ctx.state).unwrap();
    std::fs::write(ctx.state.join("hub-titles"), "").unwrap();
    let titles = HubTitles::default();
    assert!(state_of_look(&titles, &ctx, 1).ask.is_some());
    titles.ask_with(&ctx, SLUG, &[], URL, &|_, _| found("Held"));
    assert!(file(&ctx).is_none());
    let looked = state_of_look(&titles, &ctx, 2);
    assert_eq!(looked.title.as_deref(), Some("Held"));
    assert_eq!(looked.ask, None);
    let state = looked.state.unwrap();
    assert_eq!((state.status, state.total), (HubTitleStatus::Read, Some(5)));
}
