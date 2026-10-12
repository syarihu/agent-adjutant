//! Putting the person's draft comments into their pending review on GitHub.
//!
//! A review made without an event stays pending: only its author sees it, and GitHub keeps it
//! until the person submits it there. adj never submits one and never chooses a verdict, so no
//! query here contains `submitPullRequestReview` or an `event`. If the person already has a
//! pending review on the PR the drafts are added to it.
//!
//! Every call goes through one runner argument (`GhIn`), so the tests answer without a process.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::infra::clock::{now_secs, utc_stamp};
use crate::task::PrRef;

use super::drafts::{DraftItem, DraftsInput, PendingNote, StoredDrafts, validate};
use super::github::{self, GhIn};
use super::model::{PrOpenState, Record};
use super::read_through::{Side, Stored};
use super::store;

/// How long the whole push may take. The click that asks for it is waited on.
const PUSH_TIMEOUT: Duration = Duration::from_secs(60);
/// How many threads one request adds.
const THREAD_CHUNK: usize = 20;

/// One draft that was not put into the review, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// The draft's key; `overall` for the overall comment.
    pub key: String,
    pub why: String,
}

/// What became of the overall comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OverallSent {
    /// Nothing was sent: it was blank, already in the review, or could not be sent (see `failed`).
    None,
    /// It is the body of the review.
    Set,
    /// The review had a body already; it was added after it.
    Appended,
}

/// What a push came to.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pushed {
    /// Where the pending review is on GitHub.
    pub review_url: String,
    /// The review was made by this push; false when the person's pending review was added to.
    pub created: bool,
    /// The keys of the drafts that are in the review now.
    pub sent: Vec<String>,
    /// Drafts that were left out because they cannot be sent: no path, or nothing written.
    pub skipped: Vec<Problem>,
    /// Drafts (and the overall comment) GitHub refused or that may not have arrived; still unsent.
    pub failed: Vec<Problem>,
    pub overall: OverallSent,
    /// The stored drafts after the push (`null` when they could not be written back).
    pub drafts: Value,
    pub record: Record,
    /// What went wrong writing adj's own files after GitHub had the comments, when something did.
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError {
    /// adj has no record of this PR.
    NoRecord,
    /// There is no drafts file.
    NoDrafts,
    /// The PR is merged or closed: nothing is added to a review of it.
    Closed,
    /// Nothing is left to send: every draft is sent, blank or has no path, and the overall comment
    /// is blank or already in.
    Nothing,
    /// A push of this PR is running.
    Busy,
    /// The stored drafts break the contract: one problem per entry.
    Invalid(Vec<String>),
    /// GitHub or `gh` did not take any of it, and why.
    GitHub(String),
    /// The record or the drafts could not be read or written, and why.
    Failed(String),
}

/// Put the stored drafts of `pr` into the person's pending review, with the real `gh` and clock.
pub fn push_pending(root: &Path, pr: &PrRef) -> Result<Pushed, PushError> {
    push_pending_with(root, pr, now_secs(), &|args, input, deadline| {
        crate::infra::gh::run_with_input(None, args, input, deadline)
    })
}

/// What is to be sent, taken from the stored drafts before any call is made.
struct Plan {
    drafts: Vec<DraftItem>,
    skipped: Vec<Problem>,
    overall: Option<String>,
}

fn plan_of(input: &DraftsInput) -> Plan {
    let mut drafts = Vec::new();
    let mut skipped = Vec::new();
    for d in input.drafts.iter().filter(|d| d.sent_at.is_none()) {
        if d.path.is_none() {
            skipped.push(Problem {
                key: d.key.clone(),
                why: "it has no file: a comment on the whole PR goes in the overall comment"
                    .to_string(),
            });
        } else if d.body.trim().is_empty() {
            skipped.push(Problem {
                key: d.key.clone(),
                why: "nothing is written in it".to_string(),
            });
        } else {
            drafts.push(d.clone());
        }
    }
    // Whether it is already in the review is known only once the review is: see `send`.
    let overall = input
        .overall
        .as_ref()
        .map(|o| norm(&o.body))
        .filter(|body| !body.is_empty());
    Plan {
        drafts,
        skipped,
        overall,
    }
}

/// A comment as it is compared: line ends as `\n`, and no trailing space, which GitHub may trim.
fn norm(text: &str) -> String {
    text.replace("\r\n", "\n").trim_end().to_string()
}

/// Where `text` starts in `body` when it is the whole of it, or follows the blank line adj puts
/// between comments. A body that merely ends with the same characters (`ok` in "looks ok") is not it.
fn tail_start(body: &str, text: &str) -> Option<usize> {
    if text.is_empty() {
        return None;
    }
    if body == text {
        return Some(0);
    }
    body.ends_with(&format!("\n\n{text}"))
        .then(|| body.len() - text.len())
}

/// The PRs a push is running for, by root and id: a second click is refused while the first is
/// on its way, so one draft is not added twice.
static RUNNING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The place of a running push among `RUNNING`, given back by `Drop`.
struct Running(String);

impl Running {
    fn take(root: &Path, pr: &PrRef) -> Option<Running> {
        let key = format!("{}|{}", root.display(), store::id_of(pr));
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if running.contains(&key) {
            return None;
        }
        running.push(key.clone());
        Some(Running(key))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        running.retain(|k| k != &self.0);
    }
}

/// A review on GitHub, as far as a push needs it.
struct Review {
    id: String,
    url: String,
    body: String,
}

/// `push_pending`, with the time and the way to run `gh` handed in.
///
/// Everything that can refuse does so before the first call to GitHub. No lock is held while
/// GitHub is asked; the drafts file and the record are written afterwards under the record's lock,
/// one step, and a draft edited while the push was on its way is not marked as sent.
pub(crate) fn push_pending_with(
    root: &Path,
    pr: &PrRef,
    now: i64,
    gh: GhIn,
) -> Result<Pushed, PushError> {
    // First, so a second request cannot read the drafts before the first one has marked them sent.
    let _running = Running::take(root, pr).ok_or(PushError::Busy)?;
    let record = match store::load(root, pr) {
        Ok(Some(record)) => record,
        Ok(None) => return Err(PushError::NoRecord),
        Err(e) => return Err(PushError::Failed(e)),
    };
    if record.pr_state != PrOpenState::Open {
        return Err(PushError::Closed);
    }
    let stem = store::stem(pr);
    let text = store::load_drafts(root, &stem)
        .map_err(PushError::Failed)?
        .ok_or(PushError::NoDrafts)?;
    let stored: StoredDrafts = serde_json::from_str(&text).map_err(|e| {
        PushError::Failed(format!(
            "cannot read the drafts of {}: {e}",
            store::id_of(pr)
        ))
    })?;
    let input = stored.input().map_err(PushError::Failed)?;
    let problems = validate(&input);
    if !problems.is_empty() {
        return Err(PushError::Invalid(problems));
    }
    let note = stored.pending().map_err(PushError::Failed)?;
    let plan = plan_of(&input);
    if plan.drafts.is_empty() && plan.overall.is_none() {
        return Err(PushError::Nothing);
    }

    let deadline = Instant::now() + PUSH_TIMEOUT;
    let sha = read_sha(root, &stem);
    let sent = send(gh, pr, sha.as_deref(), &plan, note.as_ref(), deadline)?;

    let stamp = utc_stamp(now);
    let accepted: Vec<&DraftItem> = plan
        .drafts
        .iter()
        .filter(|d| sent.added.contains(&d.key))
        .collect();
    let overall_body = plan
        .overall
        .as_deref()
        .filter(|_| sent.overall != OverallSent::None);
    let local = write_back(root, pr, &stamp, &accepted, &sent.review, overall_body);
    let (drafts, record, warning) = match local {
        Ok(done) => (done.drafts, done.record, done.warning),
        // GitHub has the comments: not writing adj's own files is reported, not an error.
        Err(why) => (Value::Null, record, Some(why)),
    };
    Ok(Pushed {
        review_url: sent.review.url,
        created: sent.created,
        sent: sent.added,
        skipped: plan.skipped,
        failed: sent.failed,
        overall: sent.overall,
        drafts,
        record,
        warning,
    })
}

/// The head the stored read-through was made of: the commit the review is made against, so the
/// lines of the drafts are the lines of that diff. Not knowing it (no read-through, or one that
/// cannot be read) leaves the commit to GitHub, which takes the head.
fn read_sha(root: &Path, stem: &str) -> Option<String> {
    let text = store::load_read(root, stem).ok().flatten()?;
    let stored: Stored = serde_json::from_str(&text).ok()?;
    stored.rest.get("sha")?.as_str().map(str::to_string)
}

/// What GitHub took.
struct Sent {
    review: Review,
    created: bool,
    /// Keys of the drafts that are in the review.
    added: Vec<String>,
    failed: Vec<Problem>,
    overall: OverallSent,
}

/// The first message GitHub gave, for an answer with errors.
fn message_of(answer: &Value) -> Option<String> {
    answer
        .get("errors")?
        .as_array()?
        .iter()
        .find_map(|e| e.get("message").and_then(Value::as_str))
        .map(str::to_string)
}

fn text_of(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// The review in `node`, which must be pending: a review that is not is never written to.
fn review_of(node: &Value) -> Result<Review, String> {
    if text_of(node, "state").as_deref() != Some("PENDING") {
        return Err("GitHub answered a review that is not pending".to_string());
    }
    match (text_of(node, "id"), text_of(node, "url")) {
        (Some(id), Some(url)) => Ok(Review {
            id,
            url,
            body: text_of(node, "body").unwrap_or_default(),
        }),
        _ => Err("GitHub answered a review without an id".to_string()),
    }
}

const FIND_PENDING: &str = "query($o:String!,$r:String!,$p:Int!,$me:String!){\
    repository(owner:$o,name:$r){pullRequest(number:$p){id state \
    reviews(first:1,states:[PENDING],author:$me){nodes{id url state body}}}}}";

/// The PR's node id and the person's pending review on it, when there is one.
fn find_pending(
    gh: GhIn,
    pr: &PrRef,
    me: &str,
    deadline: Instant,
) -> Result<(String, Option<Review>), PushError> {
    let vars = json!({ "o": pr.owner, "r": pr.repo, "p": pr.number, "me": me });
    let answer = github::graphql(gh, FIND_PENDING, &vars, deadline).map_err(PushError::GitHub)?;
    // Not read as "there is none": a review the answer left out would be made a second time.
    if let Some(why) = message_of(&answer) {
        return Err(PushError::GitHub(why));
    }
    let Some(pull) = answer
        .pointer("/data/repository/pullRequest")
        .filter(|p| p.is_object())
    else {
        return Err(PushError::GitHub("no such pull request".to_string()));
    };
    if text_of(pull, "state").as_deref() != Some("OPEN") {
        return Err(PushError::Closed);
    }
    let id = text_of(pull, "id")
        .ok_or_else(|| PushError::GitHub("GitHub answered no id for the PR".to_string()))?;
    let review = match pull.pointer("/reviews/nodes/0").filter(|n| n.is_object()) {
        Some(node) => Some(review_of(node).map_err(PushError::GitHub)?),
        None => None,
    };
    Ok((id, review))
}

/// A review with no event: GitHub keeps it pending.
fn create_review(
    gh: GhIn,
    pr_id: &str,
    sha: Option<&str>,
    deadline: Instant,
) -> Result<Review, String> {
    let (query, vars) = match sha {
        Some(sha) => (
            "mutation($pr:ID!,$sha:GitObjectID!){addPullRequestReview(\
             input:{pullRequestId:$pr,commitOID:$sha}){pullRequestReview{id url state body}}}",
            json!({ "pr": pr_id, "sha": sha }),
        ),
        None => (
            "mutation($pr:ID!){addPullRequestReview(\
             input:{pullRequestId:$pr}){pullRequestReview{id url state body}}}",
            json!({ "pr": pr_id }),
        ),
    };
    let answer = github::graphql(gh, query, &vars, deadline)?;
    match answer
        .pointer("/data/addPullRequestReview/pullRequestReview")
        .filter(|n| n.is_object())
    {
        Some(node) => review_of(node),
        None => Err(message_of(&answer).unwrap_or_else(|| "GitHub made no review".to_string())),
    }
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Right => "RIGHT",
        Side::Left => "LEFT",
    }
}

/// The mutation that adds `chunk` as threads of review `review_id`, and its variables. Only values
/// are variables; what is spelled in the text (the aliases, the field names, `LINE` and `FILE`) is
/// ours. A draft with a line is a thread on that line; one with a path only is on the whole file.
fn threads_mutation(review_id: &str, chunk: &[DraftItem]) -> (String, Value) {
    let mut declared = vec!["$rv:ID!".to_string()];
    let mut vars = Map::new();
    vars.insert("rv".to_string(), json!(review_id));
    let mut fields = String::new();
    for (i, d) in chunk.iter().enumerate() {
        declared.push(format!("$p{i}:String!"));
        declared.push(format!("$b{i}:String!"));
        vars.insert(format!("p{i}"), json!(d.path));
        vars.insert(format!("b{i}"), json!(d.body));
        let mut input = format!("pullRequestReviewId:$rv,path:$p{i},body:$b{i}");
        match d.line {
            None => input.push_str(",subjectType:FILE"),
            Some(line) => {
                declared.push(format!("$l{i}:Int!"));
                declared.push(format!("$s{i}:DiffSide!"));
                vars.insert(format!("l{i}"), json!(line));
                vars.insert(format!("s{i}"), json!(side_name(d.side)));
                input.push_str(&format!(",subjectType:LINE,line:$l{i},side:$s{i}"));
                if let Some(start) = d.start_line.filter(|start| *start < line) {
                    declared.push(format!("$sl{i}:Int!"));
                    declared.push(format!("$ss{i}:DiffSide!"));
                    vars.insert(format!("sl{i}"), json!(start));
                    vars.insert(format!("ss{i}"), json!(side_name(d.side)));
                    input.push_str(&format!(",startLine:$sl{i},startSide:$ss{i}"));
                }
            }
        }
        fields.push_str(&format!(
            "t{i}:addPullRequestReviewThread(input:{{{input}}}){{thread{{id}}}} "
        ));
    }
    (
        format!("mutation({}){{{fields}}}", declared.join(",")),
        Value::Object(vars),
    )
}

/// One answer per draft of the chunk out of what GitHub replied. A reply with no data at all says
/// nothing of any draft: each of them may have been added.
fn thread_answers(answer: &Value, n: usize) -> Vec<Result<(), String>> {
    let Some(data) = answer.get("data").filter(|d| d.is_object()) else {
        let why = message_of(answer).unwrap_or_else(|| "GitHub answered without data".to_string());
        return (0..n).map(|_| Err(unsure(&why))).collect();
    };
    let errors = answer
        .get("errors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    (0..n)
        .map(|i| {
            let alias = format!("t{i}");
            if data
                .get(&alias)
                .and_then(|t| t.pointer("/thread/id"))
                .is_some()
            {
                return Ok(());
            }
            // An error that names another alias is that draft's; only one that names none can be this one's.
            let said = errors
                .iter()
                .find(|e| e.pointer("/path/0").and_then(Value::as_str) == Some(alias.as_str()))
                .or_else(|| {
                    errors.iter().find(|e| {
                        e.get("path")
                            .is_none_or(|p| p.as_array().is_none_or(Vec::is_empty))
                    })
                })
                .and_then(|e| e.get("message").and_then(Value::as_str));
            Err(said.unwrap_or("GitHub did not add it").to_string())
        })
        .collect()
}

fn unsure(why: &str) -> String {
    format!("{why}; it may have been added, check GitHub")
}

/// Ask GitHub to put the drafts of `plan` in the person's pending review: find it or make it, add
/// the threads, then the overall comment.
fn send(
    gh: GhIn,
    pr: &PrRef,
    sha: Option<&str>,
    plan: &Plan,
    note: Option<&PendingNote>,
    deadline: Instant,
) -> Result<Sent, PushError> {
    let plain = |args: &[&str], deadline: Instant| gh(args, None, deadline);
    let me = github::viewer(&plain, deadline).map_err(PushError::GitHub)?;
    let (pr_id, found) = find_pending(gh, pr, &me, deadline)?;
    // The overall comment is in the review already only if the review is the one it was put into:
    // after that one was submitted or discarded on GitHub, a new review has none of it.
    let in_review = |review: &Review| {
        plan.overall.is_some()
            && note.is_some_and(|n| {
                n.review_id == review.id && n.overall.as_deref().map(norm) == plan.overall
            })
    };
    if plan.drafts.is_empty() && found.as_ref().is_some_and(in_review) {
        return Err(PushError::Nothing);
    }
    let (review, created) = match found {
        Some(review) => (review, false),
        None => {
            // A commit that is not in the PR any more (a force-push) is refused: the review is
            // made at the head instead.
            let first = create_review(gh, &pr_id, sha, deadline);
            let made = match (&first, sha) {
                (Err(_), Some(_)) => create_review(gh, &pr_id, None, deadline),
                _ => first,
            };
            match made {
                Ok(review) => (review, true),
                // One made meanwhile (GitHub allows one pending review each) is added to.
                Err(why) => match find_pending(gh, pr, &me, deadline) {
                    Ok((_, Some(review))) => (review, false),
                    _ => return Err(PushError::GitHub(why)),
                },
            }
        }
    };

    let mut added = Vec::new();
    let mut failed = Vec::new();
    for chunk in plan.drafts.chunks(THREAD_CHUNK) {
        let answers: Vec<Result<(), String>> = if Instant::now() >= deadline {
            chunk
                .iter()
                .map(|_| Err("not sent: ran out of time".to_string()))
                .collect()
        } else {
            let (query, vars) = threads_mutation(&review.id, chunk);
            match github::graphql(gh, &query, &vars, deadline) {
                Ok(answer) => thread_answers(&answer, chunk.len()),
                Err(why) => chunk.iter().map(|_| Err(unsure(&why))).collect(),
            }
        };
        for (d, answer) in chunk.iter().zip(answers) {
            match answer {
                Ok(()) => added.push(d.key.clone()),
                Err(why) => failed.push(Problem {
                    key: d.key.clone(),
                    why,
                }),
            }
        }
    }

    let mut overall = OverallSent::None;
    if let Some(body) = plan.overall.as_ref().filter(|_| !in_review(&review)) {
        // What this person's earlier push of the overall comment put in the body, when it is the same review.
        let before = note
            .filter(|n| n.review_id == review.id)
            .and_then(|n| n.overall.as_deref())
            .map(norm);
        let now = norm(&review.body);
        let kind = |head: &str| {
            if head.trim().is_empty() {
                OverallSent::Set
            } else {
                OverallSent::Appended
            }
        };
        let (whole, sent_as) = if now.is_empty() {
            (Some(body.clone()), OverallSent::Set)
        } else if let Some(at) = tail_start(&now, body) {
            // Already there (an update that timed out may have landed): not added again.
            (None, kind(&now[..at]))
        } else if let Some(at) = before.as_deref().and_then(|old| tail_start(&now, old)) {
            // The comment was edited since it was put in: the new text takes the old one's place.
            (Some(format!("{}{body}", &now[..at])), kind(&now[..at]))
        } else {
            (Some(format!("{now}\n\n{body}")), OverallSent::Appended)
        };
        match whole.map_or(Ok(()), |whole| set_body(gh, &review.id, &whole, deadline)) {
            Ok(()) => overall = sent_as,
            Err(why) => failed.push(Problem {
                key: "overall".to_string(),
                why,
            }),
        }
    }

    if added.is_empty() && overall == OverallSent::None {
        // Nothing to add, and nothing refused: what was to be sent is in the review already.
        if failed.is_empty() {
            return Err(PushError::Nothing);
        }
        let why = failed
            .iter()
            .take(3)
            .map(|p| format!("{}: {}", p.key, p.why))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(PushError::GitHub(why));
    }
    Ok(Sent {
        review,
        created,
        added,
        failed,
        overall,
    })
}

fn set_body(gh: GhIn, review_id: &str, body: &str, deadline: Instant) -> Result<(), String> {
    let answer = github::graphql(
        gh,
        "mutation($rv:ID!,$body:String!){updatePullRequestReview(\
         input:{pullRequestReviewId:$rv,body:$body}){pullRequestReview{id}}}",
        &json!({ "rv": review_id, "body": body }),
        deadline,
    )?;
    if answer
        .pointer("/data/updatePullRequestReview/pullRequestReview/id")
        .is_some()
    {
        Ok(())
    } else {
        Err(unsure(
            &message_of(&answer).unwrap_or_else(|| "GitHub did not take it".to_string()),
        ))
    }
}

/// What was written back.
struct WrittenBack {
    drafts: Value,
    record: Record,
    warning: Option<String>,
}

/// Mark the drafts GitHub took as sent, note the review, and move the record off 新着, under the
/// record's lock. A draft whose body is no longer the one that was sent was edited meanwhile and
/// is left unmarked. `Err` when nothing could be written; a record that could not be saved after
/// the drafts were is a `warning`.
fn write_back(
    root: &Path,
    pr: &PrRef,
    stamp: &str,
    accepted: &[&DraftItem],
    review: &Review,
    overall: Option<&str>,
) -> Result<WrittenBack, String> {
    let stem = store::stem(pr);
    let _lock = store::lock(root, &stem)?;
    let mut record = store::load(root, pr)?.ok_or("the record was removed while it was sent")?;
    let text =
        store::load_drafts(root, &stem)?.ok_or("the drafts were removed while they were sent")?;
    let mut stored: StoredDrafts = serde_json::from_str(&text)
        .map_err(|e| format!("cannot read the drafts of {}: {e}", store::id_of(pr)))?;
    let mut input = stored.input()?;
    for d in input.drafts.iter_mut().filter(|d| d.sent_at.is_none()) {
        // The same draft is the same in everything, not the key and the body alone: the person may
        // have moved it to another line while it was on its way.
        if accepted.iter().any(|a| **a == *d) {
            d.sent_at = Some(stamp.to_string());
        }
    }
    stored.set_input(&input);
    let mut note = stored.pending()?.unwrap_or_else(|| PendingNote {
        review_id: String::new(),
        url: String::new(),
        at: String::new(),
        overall: None,
        extra: Map::new(),
    });
    // Another review than the one noted before: what was sent to that one is not in this one.
    if note.review_id != review.id {
        note.overall = None;
    }
    note.review_id.clone_from(&review.id);
    note.url.clone_from(&review.url);
    note.at = stamp.to_string();
    if let Some(body) = overall {
        note.overall = Some(body.to_string());
    }
    stored.set_pending(&note);
    stored.saved_at = stamp.to_string();
    store::write_drafts(root, &stem, &stored)?;
    let drafts = serde_json::to_value(&stored).map_err(|e| e.to_string())?;
    record.pending_review_at = Some(stamp.to_string());
    let warning = store::save(root, &mut record).err();
    Ok(WrittenBack {
        drafts,
        record,
        warning,
    })
}
