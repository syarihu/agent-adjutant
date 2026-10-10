use super::github::tests::{
    node, set, up, with_commits, with_my_review, with_requests, without_requested,
};
use super::scope::Scope;
use super::*;
use crate::infra::clock::utc_stamp;
use crate::infra::gh::GhRun;
use crate::task::pr_ref;

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

// Somewhere in the middle of October 2026, as seconds.
const NOW: i64 = 1_791_600_000;
const DAY: i64 = 86_400;

fn pr(number: u64) -> crate::task::PrRef {
    pr_ref(
        &format!("https://github.com/acme/widget/pull/{number}"),
        None,
    )
    .unwrap()
}

/// A record as the sync would have written it for a PR asked of you and not yet reviewed.
fn record(number: u64) -> Record {
    Record {
        id: format!("acme/widget#{number}"),
        repo: "acme/widget".to_string(),
        number,
        url: format!("https://github.com/acme/widget/pull/{number}"),
        registered: true,
        title: "Add a thing".to_string(),
        author: Some("alice".to_string()),
        base: "main".to_string(),
        head: "feature".to_string(),
        head_sha: "h1".to_string(),
        draft: false,
        pr_state: PrOpenState::Open,
        additions: 10,
        deletions: 2,
        changed_files: 2,
        files: Vec::new(),
        files_complete: false,
        ci: Default::default(),
        reviewers: Vec::new(),
        my_review: None,
        commits_since_review: None,
        requested: true,
        requested_at: Some("20261001T000000Z".to_string()),
        rerequest: false,
        state: State::Requested,
        done_reason: None,
        done_at: None,
        events: Events::default(),
        ai: None,
        first_seen_at: "20261001T000001Z".to_string(),
        read_at: "20261001T000001Z".to_string(),
        extra: Default::default(),
    }
}

fn reviewed(state: ReviewState, at: &str, commit: &str) -> Option<MyReview> {
    Some(MyReview {
        state,
        submitted_at: at.to_string(),
        commit: commit.to_string(),
    })
}

fn ai(status: &str) -> Option<AiRead> {
    Some(AiRead {
        status: status.to_string(),
        sha: None,
        started_at: None,
        finished_at: None,
        extra: Default::default(),
    })
}

// ── derivation ───────────────────────────────────────────────────────

#[test]
fn a_merged_pr_is_done_as_merged() {
    let mut r = record(1);
    r.pr_state = PrOpenState::Merged;
    assert_eq!(derive(&r), (State::Done, Some(DoneReason::Merged)));
}

#[test]
fn a_closed_pr_is_done_as_closed_even_when_still_requested() {
    let mut r = record(1);
    r.pr_state = PrOpenState::Closed;
    assert!(r.requested);
    assert_eq!(derive(&r), (State::Done, Some(DoneReason::Closed)));
}

#[test]
fn a_request_with_no_review_is_requested_and_the_read_through_moves_it() {
    let mut r = record(1);
    assert_eq!(derive(&r), (State::Requested, None));
    for (status, state) in [
        ("queued", State::AiReading),
        ("running", State::AiReading),
        ("ready", State::AiReady),
        ("failed", State::Requested),
        ("a-status-from-the-future", State::Requested),
    ] {
        r.ai = ai(status);
        assert_eq!(derive(&r), (state, None), "{status}");
    }
}

#[test]
fn a_re_request_is_pushed_when_the_head_moved_and_requested_when_it_did_not() {
    let mut r = record(1);
    r.requested_at = Some("20261003T000000Z".to_string());
    r.rerequest = true;
    r.my_review = reviewed(ReviewState::Commented, "20261002T000000Z", "h1");
    assert_eq!(derive(&r), (State::Requested, None));
    r.head_sha = "h2".to_string();
    assert_eq!(derive(&r), (State::Pushed, None));
    // The read-through of an earlier version does not hide the push.
    r.ai = ai("ready");
    assert_eq!(derive(&r), (State::Pushed, None));
}

#[test]
fn a_request_taken_away_is_done_as_withdrawn() {
    let mut r = record(1);
    r.requested = false;
    assert_eq!(derive(&r), (State::Done, Some(DoneReason::Withdrawn)));
    // Asked again after your review, and that request went away too.
    r.my_review = reviewed(ReviewState::Commented, "20261002T000000Z", "h1");
    r.requested_at = Some("20261003T000000Z".to_string());
    assert_eq!(derive(&r), (State::Done, Some(DoneReason::Withdrawn)));
}

#[test]
fn after_your_review_the_state_follows_the_verdict_and_the_head() {
    let mut r = record(1);
    r.requested = false;
    r.requested_at = Some("20261001T000000Z".to_string());
    r.my_review = reviewed(ReviewState::Approved, "20261002T000000Z", "h1");
    assert_eq!(derive(&r), (State::Done, Some(DoneReason::Approved)));
    for verdict in [ReviewState::Commented, ReviewState::ChangesRequested] {
        r.my_review = reviewed(verdict, "20261002T000000Z", "h1");
        assert_eq!(derive(&r), (State::WaitingOnAuthor, None));
        r.head_sha = "h2".to_string();
        assert_eq!(derive(&r), (State::Pushed, None));
        r.head_sha = "h1".to_string();
    }
}

// ── the store ────────────────────────────────────────────────────────

fn write_raw(root: &Path, name: &str, text: &str) {
    let dir = root.join("others");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), text).unwrap();
}

#[test]
fn a_key_this_binary_does_not_know_survives_a_load_and_save() {
    let root = tempfile::tempdir().unwrap();
    let mut value = serde_json::to_value(record(7)).unwrap();
    value["fromTheFuture"] = json!({"n": 1});
    value["events"]["alsoNew"] = json!("e");
    value["ai"] = json!({"status": "ready", "model": "m"});
    write_raw(root.path(), "acme~widget~7.json", &value.to_string());

    let mut loaded = store::load(root.path(), &pr(7)).unwrap().unwrap();
    assert_eq!(loaded.extra["fromTheFuture"], json!({"n": 1}));
    store::save(root.path(), &mut loaded).unwrap();

    let text = std::fs::read_to_string(root.path().join("others/acme~widget~7.json")).unwrap();
    let saved: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(saved["fromTheFuture"], json!({"n": 1}));
    assert_eq!(saved["events"]["alsoNew"], json!("e"));
    assert_eq!(saved["ai"]["model"], json!("m"));
}

#[test]
fn a_state_on_disk_this_binary_does_not_know_is_derived_again() {
    let root = tempfile::tempdir().unwrap();
    let mut value = serde_json::to_value(record(7)).unwrap();
    value["state"] = json!("from-the-future");
    value["doneReason"] = json!("from-the-future");
    write_raw(root.path(), "acme~widget~7.json", &value.to_string());

    let mut loaded = store::load(root.path(), &pr(7)).unwrap().unwrap();
    assert_eq!((loaded.state, loaded.done_reason), (State::Requested, None));
    assert!(!loaded.extra.contains_key("state"));
    assert!(!loaded.extra.contains_key("doneReason"));
    assert_eq!(store::list(root.path())[0].state, State::Requested);

    store::save(root.path(), &mut loaded).unwrap();
    let text = std::fs::read_to_string(root.path().join("others/acme~widget~7.json")).unwrap();
    // The top-level keys sit at two spaces in the pretty-printed file.
    assert_eq!(text.matches("\n  \"state\":").count(), 1);
    assert_eq!(text.matches("\n  \"doneReason\":").count(), 0);
    let saved: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(saved["state"], json!("requested"));
}

#[test]
fn the_listing_skips_what_is_not_a_record() {
    let root = tempfile::tempdir().unwrap();
    let mut r = record(7);
    store::save(root.path(), &mut r).unwrap();
    write_raw(root.path(), "sync.json", "{}");
    write_raw(root.path(), "acme~widget~7.lock", "");
    write_raw(root.path(), "acme~widget~8.json", "not json");
    write_raw(root.path(), ".staging~acme~9.json", "{}");
    let listed = store::list(root.path());
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "acme/widget#7");
    // A record that is there and cannot be read is an error, not an absent one.
    assert!(store::load(root.path(), &pr(8)).is_err());
    assert_eq!(store::load(root.path(), &pr(9)), Ok(None));
}

#[test]
fn the_listing_is_by_repository_then_request_then_number() {
    let root = tempfile::tempdir().unwrap();
    let make = |number: u64, repo: &str, at: Option<&str>| {
        let mut r = record(number);
        r.repo = repo.to_string();
        r.requested_at = at.map(str::to_string);
        r.id = format!("{}#{number}", repo.to_ascii_lowercase());
        r.url = format!("https://github.com/{repo}/pull/{number}");
        store::save(root.path(), &mut r).unwrap();
    };
    make(3, "acme/widget", None);
    make(2, "acme/widget", Some("20261005T000000Z"));
    make(1, "acme/widget", Some("20261001T000000Z"));
    make(9, "Acme/another", Some("20261009T000000Z"));
    let order: Vec<u64> = list(root.path()).iter().map(|r| r.number).collect();
    assert_eq!(order, vec![9, 1, 2, 3]);
}

// ── the sync ─────────────────────────────────────────────────────────

/// A `gh` that knows a login, a search answer and the PRs by number, and writes down what it
/// was asked.
struct Fake {
    login: Result<String, String>,
    search: Result<String, String>,
    prs: Mutex<BTreeMap<u64, Value>>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl Fake {
    fn new(hits: &[u64]) -> Fake {
        let hits: Vec<Value> = hits
            .iter()
            .map(|n| {
                json!({
                    "number": n,
                    "url": format!("https://github.com/acme/widget/pull/{n}"),
                    "repository": {"nameWithOwner": "acme/widget"}
                })
            })
            .collect();
        Fake {
            login: Ok("me".to_string()),
            search: Ok(Value::Array(hits).to_string()),
            prs: Mutex::default(),
            calls: Mutex::default(),
        }
    }

    fn with(self, number: u64, pr: Value) -> Fake {
        self.prs.lock().unwrap().insert(number, pr);
        self
    }

    fn set_hits(&mut self, hits: &[u64]) {
        self.search = Fake::new(hits).search;
    }

    fn put(&self, number: u64, pr: Value) {
        self.prs.lock().unwrap().insert(number, pr);
    }

    fn forget(&self, number: u64) {
        self.prs.lock().unwrap().remove(&number);
    }

    fn asked(&self, words: &[&str]) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.len() >= words.len() && c.iter().zip(words).all(|(a, w)| a == w))
            .count()
    }

    /// The numbers the details queries asked about so far.
    fn read(&self) -> Vec<u64> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .filter_map(|a| a.strip_prefix('p')?.split_once('=')?.1.parse().ok())
            .collect()
    }

    fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }

    fn run(&self, args: &[&str], _: Instant) -> Result<GhRun, String> {
        self.calls
            .lock()
            .unwrap()
            .push(args.iter().map(|a| a.to_string()).collect());
        let said = |out: &Result<String, String>| match out {
            Ok(stdout) => Ok(GhRun {
                ok: true,
                stdout: stdout.clone(),
                stderr: String::new(),
            }),
            Err(stderr) => Ok(GhRun {
                ok: false,
                stdout: String::new(),
                stderr: stderr.clone(),
            }),
        };
        match args {
            ["api", "user", ..] => said(&self.login),
            ["search", "prs", ..] => said(&self.search),
            ["api", "graphql", ..] => {
                let mut data = serde_json::Map::new();
                let mut errors = Vec::new();
                let prs = self.prs.lock().unwrap();
                for arg in args {
                    let Some((alias, n)) = arg.split_once('=').filter(|(a, _)| {
                        a.starts_with('p') && a[1..].chars().all(|c| c.is_ascii_digit())
                    }) else {
                        continue;
                    };
                    let found = n.parse::<u64>().ok().and_then(|n| prs.get(&n));
                    if found.is_none() {
                        errors.push(json!({
                            "type": up("not_found"),
                            "path": [alias, "pullRequest"],
                            "message": format!("Could not resolve to a PullRequest {n}")
                        }));
                    }
                    data.insert(
                        alias.to_string(),
                        json!({"pullRequest": found.cloned().unwrap_or(Value::Null)}),
                    );
                }
                let mut answer = json!({"data": data});
                let ok = errors.is_empty();
                if !ok {
                    answer["errors"] = Value::Array(errors);
                }
                Ok(GhRun {
                    ok,
                    stdout: answer.to_string(),
                    stderr: String::new(),
                })
            }
            other => panic!("unexpected gh call {other:?}"),
        }
    }
}

fn acme() -> Scope {
    Scope {
        owners: vec!["acme".to_string()],
        registered: ["acme/widget".to_string()].into(),
    }
}

fn sync_at(root: &Path, scope: &Scope, now: i64, fake: &Fake) -> Result<Synced, SyncError> {
    sync_with(root, scope, now, &|args, deadline| fake.run(args, deadline))
}

fn open_pr(head: &str) -> Value {
    node(&up("open"), head)
}

fn record_of(root: &Path, number: u64) -> Option<Record> {
    store::load(root, &pr(number)).unwrap()
}

#[test]
fn the_first_sync_records_a_request_and_says_it_arrived() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::new(&[7]).with(7, open_pr("h1"));
    let synced = sync_at(root.path(), &acme(), NOW, &fake).unwrap();

    assert_eq!(synced.arrived, vec!["acme/widget#7"]);
    assert!(synced.removed.is_empty());
    let r = record_of(root.path(), 7).unwrap();
    assert_eq!(synced.records, vec![r.clone()]);
    assert_eq!(r.events.requested.as_deref(), Some("20261001T000000Z"));
    assert_eq!(r.requested_at.as_deref(), Some("20261001T000000Z"));
    assert!(!r.rerequest);
    assert_eq!((r.state, r.registered), (State::Requested, true));
    assert_eq!(r.first_seen_at, utc_stamp(NOW));
    assert_eq!(r.author.as_deref(), Some("alice"));
    assert_eq!(r.files.len(), 2);
    assert!(r.files_complete);

    let last = last_sync(root.path()).unwrap();
    assert_eq!(last.at, Some(utc_stamp(NOW)));
    assert!(last.complete && !last.truncated);
    assert_eq!(last.owners, vec!["acme"]);
    assert_eq!(synced.last_sync, last);

    // The same request seen again is not news.
    let again = sync_at(root.path(), &acme(), NOW + 60, &fake).unwrap();
    assert!(again.arrived.is_empty());
    let r = record_of(root.path(), 7).unwrap();
    assert_eq!(r.events.requested.as_deref(), Some("20261001T000000Z"));
    assert_eq!(r.read_at, utc_stamp(NOW + 60));
    assert_eq!(r.first_seen_at, utc_stamp(NOW));
}

#[test]
fn a_record_the_search_no_longer_returns_is_read_again_by_number() {
    let root = tempfile::tempdir().unwrap();
    let mut fake = Fake::new(&[7]).with(7, open_pr("h1"));
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();

    // You reviewed it, so GitHub stopped listing you: the search is empty.
    let mut reviewed = open_pr("h1");
    without_requested(&mut reviewed);
    with_my_review(
        &mut reviewed,
        &up("commented"),
        "2026-10-02T00:00:00Z",
        "h1",
    );
    fake.put(7, reviewed);
    fake.set_hits(&[]);
    fake.clear();
    sync_at(root.path(), &acme(), NOW + 60, &fake).unwrap();

    assert_eq!(fake.read(), vec![7]);
    let r = record_of(root.path(), 7).unwrap();
    assert_eq!(r.state, State::WaitingOnAuthor);
    assert!(!r.requested);
    assert_eq!(r.my_review.unwrap().commit, "h1");
    assert_eq!(r.commits_since_review, Some(0));
}

#[test]
fn a_re_request_after_a_review_is_recorded_as_a_new_request() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::new(&[7]).with(7, open_pr("h1"));
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();

    // Reviewed on h1, the author pushed h2 and asked again.
    let mut again = open_pr("h2");
    with_commits(&mut again, &["h1", "h2"]);
    with_my_review(
        &mut again,
        &up("changes_requested"),
        "2026-10-02T00:00:00Z",
        "h1",
    );
    with_requests(
        &mut again,
        &["2026-10-01T00:00:00Z", "2026-10-03T00:00:00Z"],
    );
    fake.put(7, again);
    let synced = sync_at(root.path(), &acme(), NOW + 60, &fake).unwrap();

    assert_eq!(synced.arrived, vec!["acme/widget#7"]);
    let r = record_of(root.path(), 7).unwrap();
    assert!(r.rerequest);
    assert_eq!(r.state, State::Pushed);
    assert_eq!(r.commits_since_review, Some(1));
    assert_eq!(r.events.rerequested.as_deref(), Some("20261003T000000Z"));
    assert_eq!(r.events.requested.as_deref(), Some("20261001T000000Z"));
    assert_eq!(r.events.pushed, Some(utc_stamp(NOW + 60)));

    // Not news the next time.
    assert!(
        sync_at(root.path(), &acme(), NOW + 120, &fake)
            .unwrap()
            .arrived
            .is_empty()
    );
}

#[test]
fn a_re_request_with_nothing_pushed_is_requested_again() {
    let root = tempfile::tempdir().unwrap();
    let mut pr7 = open_pr("h1");
    with_my_review(&mut pr7, &up("commented"), "2026-10-02T00:00:00Z", "h1");
    with_requests(&mut pr7, &["2026-10-01T00:00:00Z", "2026-10-03T00:00:00Z"]);
    let fake = Fake::new(&[7]).with(7, pr7);
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    let r = record_of(root.path(), 7).unwrap();
    assert_eq!((r.state, r.rerequest), (State::Requested, true));
    assert_eq!(r.events.rerequested.as_deref(), Some("20261003T000000Z"));
    assert_eq!(r.events.requested, None);
}

#[test]
fn the_time_of_a_request_the_timeline_did_not_show_is_the_first_time_it_was_seen() {
    let root = tempfile::tempdir().unwrap();
    let mut pr7 = open_pr("h1");
    with_requests(&mut pr7, &[]);
    let fake = Fake::new(&[7]).with(7, pr7);
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    sync_at(root.path(), &acme(), NOW + 60, &fake).unwrap();
    let r = record_of(root.path(), 7).unwrap();
    assert_eq!(r.requested_at, Some(utc_stamp(NOW)));
    assert_eq!(r.events.requested, Some(utc_stamp(NOW)));
}

#[test]
fn each_later_push_after_your_review_is_news_again() {
    let root = tempfile::tempdir().unwrap();
    let review = |pr: &mut Value| {
        without_requested(pr);
        with_my_review(pr, &up("commented"), "2026-10-02T00:00:00Z", "h1");
    };
    let fake = Fake::new(&[7]).with(7, open_pr("h1"));
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    let mut reviewed_pr = open_pr("h1");
    review(&mut reviewed_pr);
    fake.put(7, reviewed_pr);
    sync_at(root.path(), &acme(), NOW + 30, &fake).unwrap();
    assert_eq!(record_of(root.path(), 7).unwrap().events.pushed, None);

    for (i, head) in ["h2", "h3"].into_iter().enumerate() {
        let mut moved = open_pr(head);
        review(&mut moved);
        fake.put(7, moved);
        let at = NOW + 60 * (i as i64 + 1);
        sync_at(root.path(), &acme(), at, &fake).unwrap();
        let r = record_of(root.path(), 7).unwrap();
        assert_eq!(r.events.pushed, Some(utc_stamp(at)));
        assert_eq!(r.state, State::Pushed);
    }
}

#[test]
fn a_pr_that_cannot_be_read_keeps_its_record_as_it_was() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::new(&[7, 8])
        .with(7, open_pr("h1"))
        .with(8, open_pr("h1"));
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    let path = root.path().join("others/acme~widget~7.json");
    let before = std::fs::read(&path).unwrap();

    fake.forget(7);
    let synced = sync_at(root.path(), &acme(), NOW + 60, &fake).unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(!synced.last_sync.complete);
    assert_eq!(synced.last_sync.failed.len(), 1);
    assert_eq!(synced.last_sync.failed[0].id, "acme/widget#7");
    assert!(synced.last_sync.failed[0].why.contains("Could not resolve"));
    // The other one was read.
    assert_eq!(
        record_of(root.path(), 8).unwrap().read_at,
        utc_stamp(NOW + 60)
    );
}

#[test]
fn a_hit_that_cannot_be_read_and_has_no_record_is_not_recorded() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::new(&[7]);
    let synced = sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    assert!(record_of(root.path(), 7).is_none());
    assert_eq!(synced.last_sync.failed.len(), 1);
    assert!(!synced.last_sync.complete);
}

#[test]
fn a_request_to_a_team_only_is_not_recorded() {
    let root = tempfile::tempdir().unwrap();
    let mut team = open_pr("h1");
    set(
        &mut team,
        "/reviewRequests/nodes",
        json!([{"requestedReviewer": {"__typename": "Team", "combinedSlug": "acme/core"}}]),
    );
    let fake = Fake::new(&[7]).with(7, team);
    let synced = sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    assert!(record_of(root.path(), 7).is_none());
    assert!(synced.arrived.is_empty() && synced.records.is_empty());
    assert!(synced.last_sync.complete);
}

#[test]
fn a_pr_in_a_repository_that_is_not_registered_is_recorded_as_such() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::new(&[7]).with(7, open_pr("h1"));
    let scope = Scope {
        owners: vec!["acme".to_string()],
        registered: ["acme/other".to_string()].into(),
    };
    sync_at(root.path(), &scope, NOW, &fake).unwrap();
    assert!(!record_of(root.path(), 7).unwrap().registered);
}

#[test]
fn a_record_done_for_a_day_is_removed_and_a_younger_one_is_kept() {
    let root = tempfile::tempdir().unwrap();
    for (number, done_at) in [
        (1, utc_stamp(NOW - DAY - 1)),
        (2, utc_stamp(NOW - DAY + 60)),
    ] {
        let mut r = record(number);
        r.pr_state = PrOpenState::Merged;
        r.done_at = Some(done_at);
        store::save(root.path(), &mut r).unwrap();
    }
    let fake = Fake::new(&[]);
    let synced = sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    assert_eq!(synced.removed, vec!["acme/widget#1"]);
    assert!(record_of(root.path(), 1).is_none());
    assert!(record_of(root.path(), 2).is_some());
    // A record in done is not read again unless the search brings it back.
    assert!(fake.read().is_empty());
    // And the lock file of a removed record stays.
    assert!(root.path().join("others/acme~widget~1.lock").exists());
}

#[test]
fn a_record_that_just_became_done_is_stamped_and_kept() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::new(&[7]).with(7, open_pr("h1"));
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    fake.set_merged(7);
    fake.clear();
    let synced = sync_at(root.path(), &acme(), NOW + 60, &fake).unwrap();
    assert!(synced.removed.is_empty());
    let r = record_of(root.path(), 7).unwrap();
    assert_eq!(
        (r.state, r.done_reason),
        (State::Done, Some(DoneReason::Merged))
    );
    assert_eq!(r.done_at, Some(utc_stamp(NOW + 60)));
    // The stamp is the first one: a later read does not move it.
    sync_at(root.path(), &acme(), NOW + DAY, &fake).unwrap();
    assert_eq!(
        record_of(root.path(), 7).unwrap().done_at,
        Some(utc_stamp(NOW + 60))
    );
}

impl Fake {
    fn set_merged(&self, number: u64) {
        let mut merged = node(&up("merged"), "h1");
        without_requested(&mut merged);
        self.put(number, merged);
    }
}

#[test]
fn a_done_record_that_is_asked_again_comes_back() {
    let root = tempfile::tempdir().unwrap();
    let mut r = record(7);
    r.requested = false;
    r.my_review = reviewed(ReviewState::Approved, "20261002T000000Z", "h1");
    r.done_at = Some(utc_stamp(NOW - 60));
    store::save(root.path(), &mut r).unwrap();
    assert_eq!(record_of(root.path(), 7).unwrap().state, State::Done);

    let mut again = open_pr("h2");
    with_my_review(&mut again, &up("approved"), "2026-10-02T00:00:00Z", "h1");
    with_requests(&mut again, &["2026-10-03T00:00:00Z"]);
    let fake = Fake::new(&[7]).with(7, again);
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    let r = record_of(root.path(), 7).unwrap();
    assert_eq!((r.state, r.done_at), (State::Pushed, None));
}

#[test]
fn what_the_read_through_wrote_survives_a_sync() {
    let root = tempfile::tempdir().unwrap();
    let mut r = record(7);
    r.ai = ai("ready");
    r.events.ai_ready = Some("20261001T010101Z".to_string());
    r.extra.insert("fromTheFuture".to_string(), json!(1));
    store::save(root.path(), &mut r).unwrap();
    let fake = Fake::new(&[7]).with(7, open_pr("h1"));
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    let r = record_of(root.path(), 7).unwrap();
    assert_eq!(r.ai, ai("ready"));
    assert_eq!(r.events.ai_ready.as_deref(), Some("20261001T010101Z"));
    assert_eq!(r.extra["fromTheFuture"], json!(1));
    assert_eq!(r.state, State::AiReady);
}

#[test]
fn with_no_owner_nothing_is_searched_and_open_records_are_still_read() {
    let root = tempfile::tempdir().unwrap();
    let mut r = record(7);
    store::save(root.path(), &mut r).unwrap();
    let fake = Fake::new(&[]).with(7, open_pr("h1"));
    let synced = sync_at(root.path(), &Scope::default(), NOW, &fake).unwrap();
    assert_eq!(fake.asked(&["search"]), 0);
    assert_eq!(fake.read(), vec![7]);
    assert!(synced.last_sync.complete);
    assert!(synced.last_sync.owners.is_empty());
}

#[test]
fn a_failed_search_changes_no_record_and_keeps_the_last_good_time() {
    let root = tempfile::tempdir().unwrap();
    let mut fake = Fake::new(&[7]).with(7, open_pr("h1"));
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    let before = std::fs::read(root.path().join("others/acme~widget~7.json")).unwrap();

    fake.search = Err("API rate limit exceeded".to_string());
    let err = sync_at(root.path(), &acme(), NOW + 60, &fake).unwrap_err();
    assert_eq!(
        err,
        SyncError::Failed("API rate limit exceeded".to_string())
    );
    assert_eq!(
        std::fs::read(root.path().join("others/acme~widget~7.json")).unwrap(),
        before
    );
    let last = last_sync(root.path()).unwrap();
    assert_eq!(last.at, Some(utc_stamp(NOW)));
    assert!(last.complete);
    assert_eq!(last.error.as_deref(), Some("API rate limit exceeded"));
    assert_eq!(last.error_at, Some(utc_stamp(NOW + 60)));

    // The next good one clears it.
    fake.set_hits(&[7]);
    sync_at(root.path(), &acme(), NOW + 120, &fake).unwrap();
    let last = last_sync(root.path()).unwrap();
    assert_eq!((last.error, last.error_at), (None, None));
}

#[test]
fn a_gh_that_cannot_say_who_you_are_fails_the_sync_with_its_words() {
    let root = tempfile::tempdir().unwrap();
    let mut fake = Fake::new(&[7]).with(7, open_pr("h1"));
    fake.login = Err("You are not logged in".to_string());
    let err = sync_at(root.path(), &acme(), NOW, &fake).unwrap_err();
    assert_eq!(err, SyncError::Failed("You are not logged in".to_string()));
    assert_eq!(fake.asked(&["search"]), 0);
    assert!(store::list(root.path()).is_empty());
    let last = last_sync(root.path()).unwrap();
    assert_eq!(
        (last.at, last.error.as_deref()),
        (None, Some("You are not logged in"))
    );
}

#[test]
fn a_second_sync_at_the_same_time_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let held = store::try_lock_sync(root.path()).unwrap().unwrap();
    let fake = Fake::new(&[7]).with(7, open_pr("h1"));
    assert_eq!(
        sync_at(root.path(), &acme(), NOW, &fake).unwrap_err(),
        SyncError::Busy
    );
    assert!(fake.calls.lock().unwrap().is_empty());
    drop(held);
    assert!(sync_at(root.path(), &acme(), NOW, &fake).is_ok());
}

#[test]
fn a_pr_with_more_files_than_a_page_reads_the_rest_once_per_head() {
    let root = tempfile::tempdir().unwrap();
    let mut big = open_pr("h1");
    set(&mut big, "/changedFiles", json!(3));
    set(
        &mut big,
        "/files/pageInfo",
        json!({"hasNextPage": true, "endCursor": "c1"}),
    );
    let fake = Fake::new(&[7]).with(7, big);
    // The second page, which the fake answers for an `after=` argument.
    let more = |args: &[&str], deadline: Instant| -> Result<GhRun, String> {
        if args.contains(&"after=c1") {
            return Ok(GhRun {
                ok: true,
                stdout: json!({"data": {"repository": {"pullRequest": {"files": {
                    "pageInfo": {"hasNextPage": false, "endCursor": null},
                    "nodes": [{"path": "c.rs", "additions": 1, "deletions": 0, "changeType": up("added")}]
                }}}}})
                .to_string(),
                stderr: String::new(),
            });
        }
        fake.run(args, deadline)
    };
    sync_with(root.path(), &acme(), NOW, &more).unwrap();
    let r = record_of(root.path(), 7).unwrap();
    assert_eq!(r.files.len(), 3);
    assert!(r.files_complete);

    // Complete at this head: the stored files stand, and no page is asked for.
    let refuse = |args: &[&str], deadline: Instant| -> Result<GhRun, String> {
        assert!(!args.contains(&"after=c1"), "asked for a page again");
        fake.run(args, deadline)
    };
    sync_with(root.path(), &acme(), NOW + 60, &refuse).unwrap();
    assert_eq!(record_of(root.path(), 7).unwrap().files.len(), 3);
}

/// A PR you reviewed on `c1`, with the head on `c3`: asked again, so a new record is made for it.
fn pushed_since_review() -> Value {
    let mut pr7 = open_pr("c3");
    with_commits(&mut pr7, &["c1", "c2", "c3"]);
    set(
        &mut pr7,
        "/commits/nodes/1/commit/committedDate",
        json!("2026-10-04T00:00:00Z"),
    );
    with_my_review(&mut pr7, &up("commented"), "2026-10-02T00:00:00Z", "c1");
    pr7
}

#[test]
fn a_push_already_there_at_the_first_sync_is_dated_by_its_first_commit() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::new(&[7]).with(7, pushed_since_review());
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    let r = record_of(root.path(), 7).unwrap();
    assert_eq!(r.events.pushed.as_deref(), Some("20261004T000000Z"));
    // Seen again with the same head: the time stays.
    sync_at(root.path(), &acme(), NOW + 60, &fake).unwrap();
    let r = record_of(root.path(), 7).unwrap();
    assert_eq!(r.events.pushed.as_deref(), Some("20261004T000000Z"));
}

#[test]
fn a_push_whose_reviewed_commit_is_out_of_the_list_is_dated_now() {
    let root = tempfile::tempdir().unwrap();
    let mut pr7 = pushed_since_review();
    with_my_review(&mut pr7, &up("commented"), "2026-10-02T00:00:00Z", "gone");
    let fake = Fake::new(&[7]).with(7, pr7);
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    assert_eq!(
        record_of(root.path(), 7).unwrap().events.pushed,
        Some(utc_stamp(NOW))
    );
}

#[test]
fn a_head_you_reviewed_has_no_push() {
    let root = tempfile::tempdir().unwrap();
    let mut r = record(7);
    r.events.pushed = Some("20261004T000000Z".to_string());
    store::save(root.path(), &mut r).unwrap();
    let mut pr7 = open_pr("c3");
    with_my_review(&mut pr7, &up("commented"), "2026-10-05T00:00:00Z", "c3");
    let fake = Fake::new(&[7]).with(7, pr7);
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    assert_eq!(record_of(root.path(), 7).unwrap().events.pushed, None);
}

fn big_pr(number: u64) -> Value {
    let mut big = open_pr("h1");
    set(&mut big, "/number", json!(number));
    set(&mut big, "/changedFiles", json!(300));
    set(
        &mut big,
        "/files/pageInfo",
        json!({"hasNextPage": true, "endCursor": "c1"}),
    );
    big
}

#[test]
fn running_out_of_time_on_file_pages_does_not_fail_a_pr_that_was_read() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::new(&[7, 8]).with(7, big_pr(7)).with(8, big_pr(8));
    // Every page of files fails the way a deadline does.
    let gh = |args: &[&str], deadline: Instant| -> Result<GhRun, String> {
        if args.iter().any(|a| a.starts_with("after=")) {
            return Err("gh did not answer in time".to_string());
        }
        fake.run(args, deadline)
    };
    let synced = sync_with(root.path(), &acme(), NOW, &gh).unwrap();
    assert!(synced.last_sync.failed.is_empty());
    assert!(synced.last_sync.complete);
    for n in [7, 8] {
        let r = record_of(root.path(), n).unwrap();
        assert_eq!(r.files.len(), 2);
        assert!(!r.files_complete);
    }
}

#[test]
fn no_file_page_is_asked_for_once_the_deadline_has_passed() {
    let root = tempfile::tempdir().unwrap();
    let facts = super::github::parse_details(&super::github::tests::data(&[big_pr(7)]), 1, "me")
        .remove(0)
        .unwrap();
    let gh =
        |_: &[&str], _: Instant| -> Result<GhRun, String> { panic!("asked after the deadline") };
    let files = super::sync::files_of(root.path(), &gh, &pr(7), &facts, Instant::now());
    assert_eq!(files.map(|f| f.len()), Some(2));
}

#[test]
fn a_commit_dated_before_your_review_does_not_date_the_push() {
    let root = tempfile::tempdir().unwrap();
    let mut pr7 = pushed_since_review();
    // Committed before the review of 2026-10-02, pushed after it.
    set(
        &mut pr7,
        "/commits/nodes/1/commit/committedDate",
        json!("2026-10-01T00:00:00Z"),
    );
    let fake = Fake::new(&[7]).with(7, pr7);
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    assert_eq!(
        record_of(root.path(), 7).unwrap().events.pushed,
        Some(utc_stamp(NOW))
    );
}

#[test]
fn stored_files_are_not_kept_when_the_totals_changed_under_the_same_head() {
    let root = tempfile::tempdir().unwrap();
    let mut big = big_pr(7);
    set(&mut big, "/changedFiles", json!(2));
    set(
        &mut big,
        "/files/pageInfo",
        json!({"hasNextPage": true, "endCursor": "c1"}),
    );
    let mut r = record(7);
    r.files_complete = true;
    r.changed_files = 2;
    r.additions = 10;
    r.deletions = 2;
    store::save(root.path(), &mut r).unwrap();
    let parse = |pr: &Value| {
        super::github::parse_details(
            &super::github::tests::data(std::slice::from_ref(pr)),
            1,
            "me",
        )
        .remove(0)
        .unwrap()
    };
    let past = Instant::now();
    let never = |_: &[&str], _: Instant| -> Result<GhRun, String> { panic!("asked") };
    // Same totals, same head: the stored files stand.
    assert_eq!(
        super::sync::files_of(root.path(), &never, &pr(7), &parse(&big), past),
        None
    );
    // The base moved: one more addition, same head. The list is taken afresh.
    set(&mut big, "/additions", json!(11));
    assert!(super::sync::files_of(root.path(), &never, &pr(7), &parse(&big), past).is_some());
}

/// The cause of a flake that showed up once in fifteen runs of the whole suite: a child that
/// forks and takes a while to exec (the pty tests start such children) holds a copy of the
/// sync lock's file, and `flock` belongs to the open file, not to the descriptor, so a sync
/// that has ended can still look held. Reproduced here with a `pre_exec` that waits.
#[test]
fn a_sync_that_ended_is_not_taken_for_a_running_one_while_a_fork_holds_its_lock_file() {
    use std::os::unix::process::CommandExt;
    let root = tempfile::tempdir().unwrap();
    let held = store::try_lock_sync(root.path()).unwrap().unwrap();
    let forker = std::thread::spawn(|| {
        let mut command = std::process::Command::new("true");
        unsafe {
            command.pre_exec(|| {
                libc::usleep(120_000);
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        child.wait().unwrap();
    });
    // Let the fork happen while the lock is still open, then end the "sync".
    std::thread::sleep(std::time::Duration::from_millis(40));
    drop(held);
    assert!(store::try_lock_sync(root.path()).unwrap().is_some());
    forker.join().unwrap();
}

#[test]
fn a_file_lock_held_elsewhere_is_busy_after_the_retries() {
    let root = tempfile::tempdir().unwrap();
    // Held through another open file, as a second process would hold it: none of ours is running.
    let held = crate::infra::fs::try_lock(&root.path().join("others/sync.lock"))
        .unwrap()
        .unwrap();
    let wait = std::time::Duration::from_millis(1);
    assert!(
        store::try_lock_sync_asking(root.path(), 5, wait)
            .unwrap()
            .is_none()
    );
    // The refusal gave back the place it took, so the lock is there to take once it is let go.
    // Asked for longer: a child another test forks can hold a copy of the lock file until it execs.
    drop(held);
    assert!(
        store::try_lock_sync_asking(root.path(), 500, std::time::Duration::from_millis(10))
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_newer_review_at_an_old_commit_does_not_keep_an_older_push_stamp() {
    let root = tempfile::tempdir().unwrap();
    // Pushed (c2) after a review on c1; the push was seen and stamped.
    let fake = Fake::new(&[7]).with(7, pushed_since_review());
    sync_at(root.path(), &acme(), NOW, &fake).unwrap();
    assert_eq!(
        record_of(root.path(), 7).unwrap().events.pushed.as_deref(),
        Some("20261004T000000Z")
    );
    // Reviewed again, still on c1 and later than that stamp, with the head unmoved.
    let mut again = pushed_since_review();
    with_my_review(&mut again, &up("commented"), "2026-10-08T00:00:00Z", "c1");
    fake.put(7, again);
    sync_at(root.path(), &acme(), NOW + 60, &fake).unwrap();
    let pushed = record_of(root.path(), 7).unwrap().events.pushed;
    assert_eq!(pushed, Some(utc_stamp(NOW + 60)));
}
