//! Putting the drafts into a pending review: the answers of GitHub are a Fake that never starts a
//! process, and writes down what it was asked.

use super::github::tests::up;
use super::tests::{NOW, pr, record};
use super::*;
use crate::infra::clock::utc_stamp;
use crate::infra::gh::GhRun;

use serde_json::{Value, json};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

const STAMP: &str = "20261002T000000Z";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// What the hook of a `Fake` is: run when a request that adds threads arrives.
type During = Box<dyn Fn() + Send + Sync>;

/// A `gh` that knows a login, a PR and (once made) a pending review, and writes down every
/// request: the query and the variables the body carried.
#[derive(Default)]
struct Fake {
    /// The pending review the person already has: id and body.
    existing: Mutex<Option<(String, String)>>,
    /// A creation that names a commit fails.
    reject_sha: bool,
    /// Every creation fails.
    reject_create: bool,
    /// A creation fails because one was made meanwhile: it is there afterwards, with this body.
    raced_body: Option<String>,
    /// Paths GitHub refuses a thread on.
    bad_paths: Vec<String>,
    /// The nth request that adds threads (from 0) is answered with errors and no data.
    no_data_at: Option<usize>,
    /// The login lookup fails.
    no_login: bool,
    /// The answer to a request that adds threads has the first one missing, and an error that names
    /// only the second.
    misplaced: bool,
    /// GitHub answers every GraphQL request as it answers a bad token.
    bad_credentials: bool,
    calls: Mutex<Vec<(String, Value)>>,
    threads_seen: AtomicUsize,
    during: Mutex<Option<During>>,
}

fn said(value: Value) -> Result<GhRun, String> {
    Ok(GhRun {
        ok: true,
        stdout: value.to_string(),
        stderr: String::new(),
    })
}

impl Fake {
    fn with_pending(body: &str) -> Fake {
        let fake = Fake::default();
        *fake.existing.lock().unwrap() = Some(("PRR_old".to_string(), body.to_string()));
        fake
    }

    fn node(&self) -> Value {
        let (id, body) = self.existing.lock().unwrap().clone().unwrap();
        json!({
            "id": id, "state": up("pending"), "body": body,
            "url": "https://github.com/acme/widget/pull/7#pullrequestreview-1"
        })
    }

    fn run(&self, args: &[&str], input: Option<&[u8]>) -> Result<GhRun, String> {
        if let ["api", "user", ..] = args {
            return if self.no_login {
                Err("cannot run gh: not found".to_string())
            } else {
                Ok(GhRun {
                    ok: true,
                    stdout: "me\n".to_string(),
                    stderr: String::new(),
                })
            };
        }
        assert_eq!(
            &args[..2],
            ["api", "graphql"],
            "unexpected gh call {args:?}"
        );
        assert!(args.contains(&"--input") && args.contains(&"-"));
        let body: Value = serde_json::from_slice(input.expect("a body on stdin")).unwrap();
        if self.bad_credentials {
            return Ok(GhRun {
                ok: false,
                stdout: json!({"message": "Bad credentials", "status": "401"}).to_string(),
                stderr: String::new(),
            });
        }
        let query = body["query"].as_str().unwrap().to_string();
        let vars = body["variables"].clone();
        self.calls
            .lock()
            .unwrap()
            .push((query.clone(), vars.clone()));
        if query.contains("addPullRequestReviewThread(") {
            let nth = self.threads_seen.fetch_add(1, Ordering::SeqCst);
            if let Some(hook) = self.during.lock().unwrap().as_ref() {
                hook();
            }
            if self.misplaced {
                return said(json!({
                    "data": {"t0": null, "t1": {"thread": {"id": "PRRT_x"}}},
                    "errors": [{"message": "about the second one", "path": ["t1"]}]
                }));
            }
            if self.no_data_at == Some(nth) {
                return said(json!({"errors": [{"message": "Something went wrong"}]}));
            }
            let mut data = serde_json::Map::new();
            let mut errors = Vec::new();
            let mut i = 0;
            while vars.get(format!("b{i}")).is_some() {
                let alias = format!("t{i}");
                let path = vars[format!("p{i}")].as_str().unwrap();
                if self.bad_paths.iter().any(|p| p == path) {
                    errors.push(json!({
                        "type": up("unprocessable"), "path": [alias],
                        "message": "Line must be part of the diff"
                    }));
                    data.insert(alias, Value::Null);
                } else {
                    data.insert(alias, json!({"thread": {"id": format!("PRRT_{nth}_{i}")}}));
                }
                i += 1;
            }
            let mut answer = json!({"data": data});
            if !errors.is_empty() {
                answer["errors"] = Value::Array(errors);
            }
            return said(answer);
        }
        if query.contains("updatePullRequestReview(") {
            self.existing.lock().unwrap().as_mut().unwrap().1 =
                vars["body"].as_str().unwrap().to_string();
            return said(
                json!({"data": {"updatePullRequestReview": {"pullRequestReview": {"id": "PRR_old"}}}}),
            );
        }
        if query.contains("addPullRequestReview(") {
            if let Some(body) = &self.raced_body {
                *self.existing.lock().unwrap() = Some(("PRR_old".to_string(), body.clone()));
                return said(json!({
                    "data": {"addPullRequestReview": null},
                    "errors": [{"message": "user can only have one pending review per pull request"}]
                }));
            }
            if self.reject_create || (self.reject_sha && vars.get("sha").is_some()) {
                return said(json!({
                    "data": {"addPullRequestReview": null},
                    "errors": [{"message": "commit is not in the pull request"}]
                }));
            }
            *self.existing.lock().unwrap() = Some(("PRR_new".to_string(), String::new()));
            return said(
                json!({"data": {"addPullRequestReview": {"pullRequestReview": self.node()}}}),
            );
        }
        assert!(query.contains("reviews("), "unexpected query {query}");
        let has = self.existing.lock().unwrap().is_some();
        let nodes: Vec<Value> = has.then(|| self.node()).into_iter().collect();
        said(json!({"data": {"repository": {"pullRequest": {
            "id": "PR_node", "state": up("open"), "reviews": {"nodes": nodes}
        }}}}))
    }

    fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    /// The requests that add threads, as their variables.
    fn thread_calls(&self) -> Vec<(String, Value)> {
        self.calls()
            .into_iter()
            .filter(|(q, _)| q.contains("addPullRequestReviewThread("))
            .collect()
    }
}

fn push(root: &Path, fake: &Fake) -> Result<Pushed, PushError> {
    push_pending_with(root, &pr(7), NOW, &|args, input, _: Instant| {
        fake.run(args, input)
    })
}

fn draft(key: &str, path: &str, line: u32, body: &str) -> Value {
    json!({"key": key, "source": "person", "path": path, "line": line, "side": "right", "body": body})
}

fn drafts_of(list: Vec<Value>, overall: &str) -> Value {
    json!({"version": 1, "fromSha": null, "drafts": list, "overall": {"body": overall}})
}

/// A record and the stored drafts of PR 7.
fn setup(list: Vec<Value>, overall: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    store::save(root.path(), &mut record(7)).unwrap();
    save_drafts(root.path(), &pr(7), &drafts_of(list, overall), STAMP).unwrap();
    root
}

fn stored(root: &Path) -> Value {
    drafts(root, &pr(7)).unwrap().unwrap()
}

fn no_event_in(fake: &Fake) {
    for (query, _) in fake.calls() {
        assert!(!query.contains("event"), "{query}");
        assert!(!query.contains("submitPullRequestReview"), "{query}");
    }
}

#[test]
fn nothing_to_send_makes_no_call_and_writes_nothing() {
    // A comment on the whole PR, a blank one, one already sent, and no overall comment.
    let mut sent = draft("s1", "a.rs", 1, "already");
    sent["sentAt"] = json!("20261001T000000Z");
    let root = setup(
        vec![
            json!({"key": "p1", "source": "person", "side": "right", "body": "on the PR"}),
            draft("b1", "a.rs", 2, "  "),
            sent,
        ],
        " ",
    );
    let before = stored(root.path());
    let fake = Fake::default();
    assert_eq!(push(root.path(), &fake).unwrap_err(), PushError::Nothing);
    assert!(fake.calls().is_empty());
    assert_eq!(stored(root.path()), before);
    assert_eq!(
        store::load(root.path(), &pr(7))
            .unwrap()
            .unwrap()
            .pending_review_at,
        None
    );
}

#[test]
fn a_pr_without_a_record_or_drafts_or_that_is_closed_is_refused_before_any_call() {
    let fake = Fake::default();
    let root = tempfile::tempdir().unwrap();
    assert_eq!(push(root.path(), &fake).unwrap_err(), PushError::NoRecord);

    store::save(root.path(), &mut record(7)).unwrap();
    assert_eq!(push(root.path(), &fake).unwrap_err(), PushError::NoDrafts);

    save_drafts(
        root.path(),
        &pr(7),
        &drafts_of(vec![draft("a", "a.rs", 1, "x")], ""),
        STAMP,
    )
    .unwrap();
    let mut closed = record(7);
    closed.pr_state = PrOpenState::Closed;
    store::save(root.path(), &mut closed).unwrap();
    assert_eq!(push(root.path(), &fake).unwrap_err(), PushError::Closed);
    assert!(fake.calls().is_empty());
}

#[test]
fn drafts_the_file_cannot_read_are_an_error_and_not_nothing_to_send() {
    let root = setup(vec![draft("a", "a.rs", 1, "x")], "");
    let path = root.path().join("others/drafts/acme~widget~7.json");
    std::fs::write(&path, "{not json").unwrap();
    let fake = Fake::default();
    assert!(matches!(
        push(root.path(), &fake),
        Err(PushError::Failed(_))
    ));
    assert!(fake.calls().is_empty());
}

#[test]
fn a_review_is_made_without_an_event_at_the_commit_that_was_read() {
    let root = setup(
        vec![
            draft("a", "src/a.rs", 4, "why?"),
            draft("b", "src/b.rs", 9, "and this?"),
        ],
        "looks fine",
    );
    store::write_read(
        root.path(),
        &store::stem(&pr(7)),
        &json!({"id": "acme/widget#7", "madeAt": STAMP, "sha": SHA}),
    )
    .unwrap();
    let fake = Fake::default();
    let pushed = push(root.path(), &fake).unwrap();

    assert!(pushed.created);
    assert_eq!(pushed.sent, vec!["a", "b"]);
    assert_eq!(pushed.overall, OverallSent::Set);
    assert!(pushed.failed.is_empty() && pushed.skipped.is_empty());
    assert_eq!(pushed.warning, None);
    assert_eq!(
        pushed.review_url,
        "https://github.com/acme/widget/pull/7#pullrequestreview-1"
    );
    no_event_in(&fake);
    let create = fake
        .calls()
        .into_iter()
        .find(|(q, _)| q.contains("addPullRequestReview("))
        .unwrap();
    assert_eq!(create.1, json!({"pr": "PR_node", "sha": SHA}));
    // The threads carry the values as variables, and the body is the overall comment.
    let threads = fake.thread_calls();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].1["p0"], "src/a.rs");
    assert_eq!(threads[0].1["b1"], "and this?");
    assert_eq!(threads[0].1["s0"], "RIGHT");
    assert!(!threads[0].0.contains("why?"));
    assert_eq!(
        fake.existing.lock().unwrap().clone().unwrap().1,
        "looks fine"
    );

    // adj's own files: the drafts are marked, the review is noted, the row is acted on.
    let file = stored(root.path());
    let stamp = utc_stamp(NOW);
    assert_eq!(file["drafts"][0]["sentAt"], stamp.as_str());
    assert_eq!(file["drafts"][1]["sentAt"], stamp.as_str());
    assert_eq!(file["pending"]["reviewId"], "PRR_new");
    assert_eq!(file["pending"]["overall"], "looks fine");
    assert_eq!(file["pending"]["at"], stamp.as_str());
    assert_eq!(pushed.drafts, file);
    let rec = store::load(root.path(), &pr(7)).unwrap().unwrap();
    assert_eq!(rec.pending_review_at.as_deref(), Some(stamp.as_str()));
    assert_eq!(pushed.record.pending_review_at, rec.pending_review_at);
}

#[test]
fn a_commit_github_refuses_is_dropped_and_the_review_is_made_at_the_head() {
    let root = setup(vec![draft("a", "a.rs", 1, "x")], "");
    store::write_read(
        root.path(),
        &store::stem(&pr(7)),
        &json!({"id": "acme/widget#7", "madeAt": STAMP, "sha": SHA}),
    )
    .unwrap();
    let fake = Fake {
        reject_sha: true,
        ..Fake::default()
    };
    let pushed = push(root.path(), &fake).unwrap();
    assert!(pushed.created);
    let creates: Vec<Value> = fake
        .calls()
        .into_iter()
        .filter(|(q, _)| q.contains("addPullRequestReview("))
        .map(|(_, v)| v)
        .collect();
    assert_eq!(
        creates,
        vec![
            json!({"pr": "PR_node", "sha": SHA}),
            json!({"pr": "PR_node"})
        ]
    );
    no_event_in(&fake);
}

#[test]
fn a_review_that_cannot_be_made_is_an_error_and_writes_nothing() {
    let root = setup(vec![draft("a", "a.rs", 1, "x")], "");
    let before = stored(root.path());
    let fake = Fake {
        reject_create: true,
        ..Fake::default()
    };
    match push(root.path(), &fake) {
        Err(PushError::GitHub(why)) => assert!(why.contains("not in the pull request"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert!(fake.thread_calls().is_empty());
    assert_eq!(stored(root.path()), before);
}

#[test]
fn an_existing_pending_review_is_added_to_and_the_overall_comment_follows_its_body_once() {
    let root = setup(vec![draft("a", "a.rs", 1, "x")], "looks fine");
    let fake = Fake::with_pending("my earlier note");
    let pushed = push(root.path(), &fake).unwrap();

    assert!(!pushed.created);
    assert_eq!(pushed.overall, OverallSent::Appended);
    assert!(
        !fake
            .calls()
            .iter()
            .any(|(q, _)| q.contains("addPullRequestReview("))
    );
    assert_eq!(fake.thread_calls()[0].1["rv"], "PRR_old");
    assert_eq!(
        fake.existing.lock().unwrap().clone().unwrap().1,
        "my earlier note\n\nlooks fine"
    );
    // Everything is in now: pushing again has nothing to say.
    assert_eq!(push(root.path(), &fake).unwrap_err(), PushError::Nothing);
}

#[test]
fn a_second_push_sends_only_what_is_new() {
    let root = setup(vec![draft("a", "a.rs", 1, "x")], "");
    let fake = Fake::default();
    push(root.path(), &fake).unwrap();

    // The page saves what the person added, with the first draft as it read it back.
    let mut file = stored(root.path());
    file["drafts"]
        .as_array_mut()
        .unwrap()
        .push(draft("b", "b.rs", 2, "later"));
    let doc = json!({"version": 1, "fromSha": null, "drafts": file["drafts"], "overall": null});
    save_drafts(root.path(), &pr(7), &doc, STAMP).unwrap();

    let pushed = push(root.path(), &fake).unwrap();
    assert!(!pushed.created);
    assert_eq!(pushed.sent, vec!["b"]);
    let threads = fake.thread_calls();
    assert_eq!(threads.len(), 2);
    assert_eq!(threads[1].1["p0"], "b.rs");
    assert!(threads[1].1.get("p1").is_none());
}

#[test]
fn a_draft_github_refuses_stays_unsent_and_does_not_fail_the_others() {
    let root = setup(
        vec![
            draft("a", "a.rs", 1, "x"),
            draft("b", "gone.rs", 2, "y"),
            draft("c", "c.rs", 3, "z"),
        ],
        "",
    );
    let fake = Fake {
        bad_paths: vec!["gone.rs".to_string()],
        ..Fake::default()
    };
    let pushed = push(root.path(), &fake).unwrap();
    assert_eq!(pushed.sent, vec!["a", "c"]);
    assert_eq!(
        pushed.failed,
        vec![Problem {
            key: "b".to_string(),
            why: "Line must be part of the diff".to_string()
        }]
    );
    let file = stored(root.path());
    assert!(file["drafts"][0]["sentAt"].is_string());
    assert!(file["drafts"][1].get("sentAt").is_none());
    assert!(file["drafts"][2]["sentAt"].is_string());
}

#[test]
fn a_request_that_comes_back_without_data_is_not_taken_for_done() {
    // 21 drafts are two requests; the second one is answered with errors and no data.
    let list: Vec<Value> = (0..21)
        .map(|i| draft(&format!("d{i}"), "a.rs", i + 1, "x"))
        .collect();
    let root = setup(list, "");
    let fake = Fake {
        no_data_at: Some(1),
        ..Fake::default()
    };
    let pushed = push(root.path(), &fake).unwrap();
    assert_eq!(pushed.sent.len(), 20);
    assert_eq!(pushed.failed.len(), 1);
    assert_eq!(pushed.failed[0].key, "d20");
    assert!(
        pushed.failed[0].why.contains("check GitHub"),
        "{:?}",
        pushed.failed
    );
    let file = stored(root.path());
    assert!(file["drafts"][20].get("sentAt").is_none());
    assert!(file["drafts"][19]["sentAt"].is_string());
}

#[test]
fn when_github_takes_nothing_nothing_is_written() {
    let root = setup(vec![draft("a", "gone.rs", 1, "x")], "");
    let before = stored(root.path());
    let fake = Fake {
        bad_paths: vec!["gone.rs".to_string()],
        ..Fake::default()
    };
    match push(root.path(), &fake) {
        Err(PushError::GitHub(why)) => assert!(why.contains("Line must be part"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(stored(root.path()), before);
    assert_eq!(
        store::load(root.path(), &pr(7))
            .unwrap()
            .unwrap()
            .pending_review_at,
        None
    );

    // `gh` itself failing is the same.
    let fake = Fake {
        no_login: true,
        ..Fake::default()
    };
    assert!(matches!(
        push(root.path(), &fake),
        Err(PushError::GitHub(_))
    ));
    assert_eq!(stored(root.path()), before);
}

#[test]
fn a_draft_edited_while_the_push_was_on_its_way_is_not_marked_sent() {
    let root = setup(
        vec![draft("a", "a.rs", 1, "x"), draft("b", "b.rs", 2, "y")],
        "",
    );
    let fake = Fake::default();
    let path = root.path().to_path_buf();
    *fake.during.lock().unwrap() = Some(Box::new(move || {
        let edited = drafts_of(
            vec![
                draft("a", "a.rs", 1, "x, edited"),
                draft("b", "b.rs", 2, "y"),
            ],
            "",
        );
        save_drafts(&path, &pr(7), &edited, STAMP).unwrap();
    }));
    let pushed = push(root.path(), &fake).unwrap();
    assert_eq!(pushed.sent, vec!["a", "b"]);
    let file = stored(root.path());
    assert_eq!(file["drafts"][0]["body"], "x, edited");
    assert!(file["drafts"][0].get("sentAt").is_none());
    assert!(file["drafts"][1]["sentAt"].is_string());
}

#[test]
fn a_second_push_while_one_is_running_is_busy() {
    let root = setup(vec![draft("a", "a.rs", 1, "x")], "");
    let fake = Fake::default();
    let inner = Fake::default();
    let seen = std::sync::Arc::new(Mutex::new(None));
    let (path, out) = (root.path().to_path_buf(), seen.clone());
    *fake.during.lock().unwrap() = Some(Box::new(move || {
        *out.lock().unwrap() = Some(push(&path, &inner));
    }));
    push(root.path(), &fake).unwrap();
    assert_eq!(
        seen.lock().unwrap().take().unwrap().unwrap_err(),
        PushError::Busy
    );
    // The place is given back.
    assert_eq!(push(root.path(), &fake).unwrap_err(), PushError::Nothing);
}

#[test]
fn lines_and_files_are_spelled_as_github_wants_them() {
    let mut multi = draft("m", "a.rs", 8, "range");
    multi["startLine"] = json!(5);
    multi["side"] = json!("left");
    let mut single = draft("s", "a.rs", 8, "one line");
    single["startLine"] = json!(8);
    let whole_file =
        json!({"key": "f", "source": "person", "path": "a.rs", "side": "right", "body": "file"});
    let root = setup(vec![multi, single, whole_file], "");
    let fake = Fake::default();
    push(root.path(), &fake).unwrap();
    let (query, vars) = fake.thread_calls().remove(0);
    assert!(query.contains("t0:addPullRequestReviewThread(input:{pullRequestReviewId:$rv,path:$p0,body:$b0,subjectType:LINE,line:$l0,side:$s0,startLine:$sl0,startSide:$ss0})"), "{query}");
    assert_eq!(
        (&vars["sl0"], &vars["ss0"], &vars["s0"]),
        (&json!(5), &json!("LEFT"), &json!("LEFT"))
    );
    // A start on the line itself is a single line.
    assert!(query.contains("t1:addPullRequestReviewThread(input:{pullRequestReviewId:$rv,path:$p1,body:$b1,subjectType:LINE,line:$l1,side:$s1})"), "{query}");
    assert!(vars.get("sl1").is_none());
    // No line is the whole file.
    assert!(
        query.contains("path:$p2,body:$b2,subjectType:FILE})"),
        "{query}"
    );
    assert!(vars.get("l2").is_none() && vars.get("s2").is_none());
}

#[test]
fn drafts_that_cannot_be_sent_are_reported_and_never_folded_into_the_body() {
    let root = setup(
        vec![
            json!({"key": "p1", "source": "person", "side": "right", "body": "on the PR"}),
            draft("b1", "a.rs", 2, " "),
            draft("ok", "a.rs", 3, "fine"),
        ],
        "",
    );
    let fake = Fake::default();
    let pushed = push(root.path(), &fake).unwrap();
    assert_eq!(pushed.sent, vec!["ok"]);
    let keys: Vec<&str> = pushed.skipped.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, vec!["p1", "b1"]);
    assert_eq!(pushed.overall, OverallSent::None);
    assert!(
        !fake
            .calls()
            .iter()
            .any(|(q, _)| q.contains("updatePullRequestReview("))
    );
    // They stay as they were.
    let file = stored(root.path());
    assert!(file["drafts"][0].get("sentAt").is_none() && file["drafts"][1].get("sentAt").is_none());
}

#[test]
fn keys_this_binary_does_not_know_survive_a_push() {
    let root = tempfile::tempdir().unwrap();
    let mut rec = record(7);
    rec.extra.insert("futureKey".to_string(), json!({"n": 1}));
    store::save(root.path(), &mut rec).unwrap();
    save_drafts(
        root.path(),
        &pr(7),
        &drafts_of(vec![draft("a", "a.rs", 1, "x")], ""),
        STAMP,
    )
    .unwrap();
    // Something a newer binary put in the file.
    let path = root.path().join("others/drafts/acme~widget~7.json");
    let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file["note"] = json!("keep me");
    file["pending"] = json!({"reviewId": "PRR_old", "url": "u", "at": STAMP, "later": true});
    std::fs::write(&path, file.to_string()).unwrap();

    let fake = Fake::with_pending("");
    push(root.path(), &fake).unwrap();
    let file = stored(root.path());
    assert_eq!(file["note"], "keep me");
    assert_eq!(file["pending"]["later"], true);
    assert_eq!(file["pending"]["reviewId"], "PRR_old");
    let rec = store::load(root.path(), &pr(7)).unwrap().unwrap();
    assert_eq!(rec.extra["futureKey"], json!({"n": 1}));
}

#[test]
fn a_save_keeps_the_sent_mark_of_a_draft_whose_body_is_the_same() {
    let root = setup(
        vec![draft("a", "a.rs", 1, "x"), draft("b", "b.rs", 2, "y")],
        "",
    );
    push(root.path(), &Fake::default()).unwrap();
    // A page that has not read the marks back saves both drafts, one of them edited.
    let again = drafts_of(
        vec![
            draft("a", "a.rs", 1, "x"),
            draft("b", "b.rs", 2, "y, edited"),
        ],
        "",
    );
    let saved = save_drafts(root.path(), &pr(7), &again, STAMP).unwrap();
    assert!(saved["drafts"][0]["sentAt"].is_string());
    assert!(saved["drafts"][1].get("sentAt").is_none());
    // The review's note is kept as well.
    assert_eq!(saved["pending"]["reviewId"], "PRR_new");
}

#[test]
fn a_sent_mark_that_is_not_a_stamp_is_refused() {
    let root = setup(vec![], "");
    for bad in ["yesterday", "2026-10-02T00:00:00Z", ""] {
        let mut d = draft("a", "a.rs", 1, "x");
        d["sentAt"] = json!(bad);
        match save_drafts(root.path(), &pr(7), &drafts_of(vec![d], ""), STAMP) {
            Err(SaveDraftsError::Invalid(problems)) => {
                assert!(problems[0].contains("sentAt"), "{problems:?}");
            }
            other => panic!("{bad}: {other:?}"),
        }
    }
    let mut d = draft("a", "a.rs", 1, "x");
    d["sentAt"] = json!(STAMP);
    assert!(save_drafts(root.path(), &pr(7), &drafts_of(vec![d], ""), STAMP).is_ok());
}

/// The note an earlier push left in the drafts file.
fn note_pending(root: &Path, review_id: &str, overall: &str) {
    let path = root.join("others/drafts/acme~widget~7.json");
    let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file["pending"] = json!({"reviewId": review_id, "url": "u", "at": STAMP, "overall": overall});
    std::fs::write(&path, file.to_string()).unwrap();
}

#[test]
fn the_overall_comment_is_in_the_review_only_if_it_is_the_review_it_was_put_into() {
    // The earlier review was submitted or discarded on GitHub: the one there now is another.
    let root = setup(vec![], "all of it");
    note_pending(root.path(), "PRR_gone", "all of it");
    let fake = Fake::with_pending("");
    let pushed = push(root.path(), &fake).unwrap();
    assert_eq!(pushed.overall, OverallSent::Set);
    assert_eq!(
        fake.existing.lock().unwrap().clone().unwrap().1,
        "all of it"
    );
    assert_eq!(stored(root.path())["pending"]["reviewId"], "PRR_old");

    // The same review, with the same text in it: not sent again, and nothing is written.
    let root = setup(vec![], "all of it");
    note_pending(root.path(), "PRR_old", "all of it");
    let before = stored(root.path());
    let fake = Fake::with_pending("all of it");
    assert_eq!(push(root.path(), &fake).unwrap_err(), PushError::Nothing);
    assert!(
        !fake
            .calls()
            .iter()
            .any(|(q, _)| q.contains("updatePullRequestReview("))
    );
    assert_eq!(stored(root.path()), before);

    // With threads to add, the same review's body is left as it is.
    let root = setup(vec![draft("a", "a.rs", 1, "x")], "all of it");
    note_pending(root.path(), "PRR_old", "all of it");
    let fake = Fake::with_pending("all of it");
    let pushed = push(root.path(), &fake).unwrap();
    assert_eq!((pushed.sent.len(), pushed.overall), (1, OverallSent::None));
    assert_eq!(
        fake.existing.lock().unwrap().clone().unwrap().1,
        "all of it"
    );

    // No pending review now: nothing to compare with, so it goes into the new one.
    let root = setup(vec![], "all of it");
    note_pending(root.path(), "PRR_old", "all of it");
    let pushed = push(root.path(), &Fake::default()).unwrap();
    assert_eq!((pushed.created, pushed.overall), (true, OverallSent::Set));
}

#[test]
fn a_failure_gh_prints_without_errors_is_reported_as_its_message() {
    let root = setup(vec![draft("a", "a.rs", 1, "x")], "");
    let before = stored(root.path());
    let fake = Fake {
        bad_credentials: true,
        ..Fake::default()
    };
    assert_eq!(
        push(root.path(), &fake).unwrap_err(),
        PushError::GitHub("Bad credentials".to_string())
    );
    assert_eq!(stored(root.path()), before);
}

#[test]
fn an_overall_comment_edited_after_it_was_put_in_takes_the_old_ones_place() {
    let root = setup(vec![], "LGTM");
    let fake = Fake::with_pending("earlier");
    push(root.path(), &fake).unwrap();
    assert_eq!(
        fake.existing.lock().unwrap().clone().unwrap().1,
        "earlier\n\nLGTM"
    );

    // Edited, as the page saves it, and put in again.
    save_drafts(
        root.path(),
        &pr(7),
        &drafts_of(vec![], "LGTM except the rounding"),
        STAMP,
    )
    .unwrap();
    let pushed = push(root.path(), &fake).unwrap();
    assert_eq!(pushed.overall, OverallSent::Appended);
    assert_eq!(
        fake.existing.lock().unwrap().clone().unwrap().1,
        "earlier\n\nLGTM except the rounding"
    );
    assert_eq!(
        stored(root.path())["pending"]["overall"],
        "LGTM except the rounding"
    );

    // A body that was only the comment is replaced whole.
    let root = setup(vec![], "one");
    let fake = Fake::default();
    push(root.path(), &fake).unwrap();
    save_drafts(root.path(), &pr(7), &drafts_of(vec![], "two"), STAMP).unwrap();
    assert_eq!(push(root.path(), &fake).unwrap().overall, OverallSent::Set);
    assert_eq!(fake.existing.lock().unwrap().clone().unwrap().1, "two");
}

#[test]
fn a_body_that_already_ends_with_the_overall_comment_is_not_added_again() {
    // An earlier update that timed out but landed: no note says so, the body does.
    let root = setup(vec![], "LGTM");
    let fake = Fake::with_pending("earlier\n\nLGTM");
    let pushed = push(root.path(), &fake).unwrap();
    assert_eq!(pushed.overall, OverallSent::Appended);
    assert!(
        !fake
            .calls()
            .iter()
            .any(|(q, _)| q.contains("updatePullRequestReview("))
    );
    assert_eq!(stored(root.path())["pending"]["overall"], "LGTM");

    // Another review than the noted one, whose body does not end with it, is appended to.
    let root = setup(vec![], "LGTM");
    note_pending(root.path(), "PRR_gone", "LGTM");
    let fake = Fake::with_pending("other");
    push(root.path(), &fake).unwrap();
    assert_eq!(
        fake.existing.lock().unwrap().clone().unwrap().1,
        "other\n\nLGTM"
    );
}

#[test]
fn an_error_that_names_another_draft_is_not_given_as_the_reason() {
    let root = setup(
        vec![draft("a", "a.rs", 1, "x"), draft("b", "b.rs", 2, "y")],
        "",
    );
    let fake = Fake {
        misplaced: true,
        ..Fake::default()
    };
    let pushed = push(root.path(), &fake).unwrap();
    assert_eq!(pushed.sent, vec!["b"]);
    assert_eq!(pushed.failed[0].why, "GitHub did not add it");
}

#[test]
fn a_comment_that_only_ends_with_the_same_letters_is_not_the_overall_comment() {
    // "ok" ends "Thanks, looks ok": it is someone's text, not ours, so ours is added after it.
    let root = setup(vec![], "ok");
    let fake = Fake::with_pending("Thanks, looks ok");
    assert_eq!(
        push(root.path(), &fake).unwrap().overall,
        OverallSent::Appended
    );
    assert_eq!(
        fake.existing.lock().unwrap().clone().unwrap().1,
        "Thanks, looks ok\n\nok"
    );

    // The same for the old text of an edit: it is not cut out of the middle of a word.
    let root = setup(vec![], "ok");
    let fake = Fake::with_pending("start");
    push(root.path(), &fake).unwrap();
    fake.existing.lock().unwrap().as_mut().unwrap().1 = "Thanks, looks ok".to_string();
    save_drafts(root.path(), &pr(7), &drafts_of(vec![], "fine"), STAMP).unwrap();
    push(root.path(), &fake).unwrap();
    assert_eq!(
        fake.existing.lock().unwrap().clone().unwrap().1,
        "Thanks, looks ok\n\nfine"
    );
}

#[test]
fn line_ends_and_trailing_space_do_not_make_an_edit_an_append() {
    let root = setup(vec![], "LGTM\r\n");
    let fake = Fake::with_pending("earlier");
    push(root.path(), &fake).unwrap();
    assert_eq!(
        fake.existing.lock().unwrap().clone().unwrap().1,
        "earlier\n\nLGTM"
    );
    assert_eq!(stored(root.path())["pending"]["overall"], "LGTM");
    // GitHub trimmed what was sent; the same text with its newline is not sent again.
    save_drafts(root.path(), &pr(7), &drafts_of(vec![], "LGTM\n"), STAMP).unwrap();
    assert_eq!(push(root.path(), &fake).unwrap_err(), PushError::Nothing);
    // An edit replaces.
    save_drafts(
        root.path(),
        &pr(7),
        &drafts_of(vec![], "LGTM, but\r\nthe rounding\n"),
        STAMP,
    )
    .unwrap();
    push(root.path(), &fake).unwrap();
    assert_eq!(
        fake.existing.lock().unwrap().clone().unwrap().1,
        "earlier\n\nLGTM, but\nthe rounding"
    );
}

#[test]
fn a_draft_moved_while_it_was_on_its_way_is_not_the_one_that_was_sent() {
    let root = setup(vec![draft("a", "a.rs", 1, "x")], "");
    let fake = Fake::default();
    let path = root.path().to_path_buf();
    *fake.during.lock().unwrap() = Some(Box::new(move || {
        // The same key and body, on another line.
        save_drafts(
            &path,
            &pr(7),
            &drafts_of(vec![draft("a", "a.rs", 9, "x")], ""),
            STAMP,
        )
        .unwrap();
    }));
    push(root.path(), &fake).unwrap();
    let file = stored(root.path());
    assert_eq!(file["drafts"][0]["line"], 9);
    assert!(file["drafts"][0].get("sentAt").is_none());
}

#[test]
fn a_save_keeps_the_sent_mark_only_for_the_draft_that_was_sent() {
    let root = setup(
        vec![draft("a", "a.rs", 1, "x"), draft("b", "b.rs", 2, "y")],
        "",
    );
    push(root.path(), &Fake::default()).unwrap();
    // `a` is unchanged; `b` has the same key and body on another line.
    let again = drafts_of(
        vec![draft("a", "a.rs", 1, "x"), draft("b", "b.rs", 3, "y")],
        "",
    );
    let saved = save_drafts(root.path(), &pr(7), &again, STAMP).unwrap();
    assert!(saved["drafts"][0]["sentAt"].is_string());
    assert!(saved["drafts"][1].get("sentAt").is_none());
}

#[test]
fn a_review_made_meanwhile_that_already_has_everything_is_nothing_to_send_and_not_an_error() {
    let root = setup(vec![], "all of it");
    note_pending(root.path(), "PRR_old", "all of it");
    let before = stored(root.path());
    let fake = Fake {
        raced_body: Some("all of it".to_string()),
        ..Fake::default()
    };
    assert_eq!(push(root.path(), &fake).unwrap_err(), PushError::Nothing);
    assert_eq!(stored(root.path()), before);
}
