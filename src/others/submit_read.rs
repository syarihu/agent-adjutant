//! Handing in a read-through: checked, stored, and the record told it is ready.

use std::path::Path;

use crate::infra::clock::utc_stamp;
use crate::task::PrRef;

use super::model::{AiCounts, Record};
use super::read_through::{Level, MAX_READ_BYTES, ReadThrough, Stored, validate};
use super::store;

/// What a hand-in came to.
#[derive(Debug, Clone, PartialEq)]
pub struct Submitted {
    pub record: Record,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitError {
    /// The file breaks the contract: one problem per entry, all of them.
    Invalid(Vec<String>),
    /// adj has no record of this PR.
    NoRecord,
    /// Nobody asked for a read-through of this PR.
    NotAsked,
    /// The reader that handed it in is not the one the record names now: the read was asked for
    /// again, so a newer reader owns it.
    Superseded,
    /// The record or the file could not be read or written, and why.
    Failed(String),
}

/// Hand in `text` as the read-through of `pr`.
///
/// `session` is the reader's session when it says one (`ADJUTANT_REVIEW_SESSION`). One that is
/// not the record's current `sessionId` is refused, which includes a record that has none
/// because a re-read is waiting in the queue; only a person handing a file in by hand says
/// none, and is not held to it.
///
/// Everything that can refuse does so before the first write. The file is then written first and
/// the record second, and a record that cannot be saved takes the file back, so the two are
/// never half changed.
pub fn submit_read(
    root: &Path,
    pr: &PrRef,
    text: &str,
    session: Option<&str>,
    now: i64,
) -> Result<Submitted, SubmitError> {
    submit_read_with(root, pr, text, session, now, &|record| {
        store::save(root, record)
    })
}

/// `submit_read`, with the way the record is saved handed in.
pub(crate) fn submit_read_with(
    root: &Path,
    pr: &PrRef,
    text: &str,
    session: Option<&str>,
    now: i64,
    save: &dyn Fn(&mut Record) -> Result<(), String>,
) -> Result<Submitted, SubmitError> {
    if text.len() > MAX_READ_BYTES {
        return Err(SubmitError::Invalid(vec![format!(
            "the file is {} bytes, at most {MAX_READ_BYTES} are taken",
            text.len()
        )]));
    }
    let read: ReadThrough = serde_json::from_str(text)
        .map_err(|e| SubmitError::Invalid(vec![format!("cannot read the file: {e}")]))?;
    validate(&read).map_err(SubmitError::Invalid)?;
    // Before the lock is taken, so that a refused hand-in leaves not even a lock file.
    check_asked(store::load(root, pr).map_err(SubmitError::Failed)?)?;

    let stem = store::stem(pr);
    let _lock = store::lock(root, &stem).map_err(SubmitError::Failed)?;
    let mut record = check_asked(store::load(root, pr).map_err(SubmitError::Failed)?)?;
    if let Some(session) = session
        && record.ai.as_ref().and_then(|ai| ai.session_id.as_deref()) != Some(session)
    {
        return Err(SubmitError::Superseded);
    }

    // A PR you reviewed that has moved is a re-review, and a re-review says what became of your
    // comments. `sha` is the head the reader read, `commit` the one you reviewed.
    if let Some(mine) = &record.my_review
        && read.since_review.is_none()
        && mine.commit != read.sha
    {
        return Err(SubmitError::Invalid(vec![format!(
            "this is a re-review (you reviewed {}, and the head is {}): sinceReview is required, not null",
            mine.commit, read.sha
        )]));
    }
    // What `adj review context` wrote for this reader is what it must have read. Without those
    // files (a hand-in by a person) there is nothing to check against.
    if session.is_some()
        && let Some(head) = pr_json_head(root, pr)
        && head != read.sha
    {
        return Err(SubmitError::Invalid(vec![format!(
            "sha {} is not the headRefOid of pr.json ({head}): use the headRefOid of pr.json, and read the material again if the head moved",
            read.sha
        )]));
    }
    if let Some(since) = &read.since_review
        && let Some(known) = known_comments(root, pr)
    {
        let mut problems: Vec<String> = Vec::new();
        let unknown: Vec<String> = since
            .comments
            .iter()
            .filter(|c| !known.all.contains(&c.comment_id))
            .map(|c| c.comment_id.to_string())
            .collect();
        if !unknown.is_empty() {
            problems.push(format!(
                "sinceReview.comments has commentId {} which is not in my-last-review.json: use the ids of that file only",
                unknown.join(", ")
            ));
        }
        let times = |id: u64| since.comments.iter().filter(|c| c.comment_id == id).count();
        let missing: Vec<String> = known
            .top
            .iter()
            .filter(|id| times(**id) == 0)
            .map(u64::to_string)
            .collect();
        if !missing.is_empty() {
            problems.push(format!(
                "sinceReview.comments has no entry for the comment(s) {}: every comment of your last review (those of my-last-review.json with no in_reply_to_id) needs exactly one",
                missing.join(", ")
            ));
        }
        let twice: Vec<String> = known
            .top
            .iter()
            .filter(|id| times(**id) > 1)
            .map(u64::to_string)
            .collect();
        if !twice.is_empty() {
            problems.push(format!(
                "sinceReview.comments has more than one entry for the comment(s) {}: exactly one each",
                twice.join(", ")
            ));
        }
        if !problems.is_empty() {
            return Err(SubmitError::Invalid(problems));
        }
    }
    // The record's own word on which review of yours this is about, taken under the lock: a
    // `sinceReview` of another commit would put a comparison on the wrong review.
    if let Some(since) = &read.since_review {
        let wrong = match &record.my_review {
            Some(mine) if mine.commit == since.commit => None,
            Some(mine) => Some(format!(
                "sinceReview.commit is {}, but your last review is on {}: use myReview.commit of pr.json",
                since.commit, mine.commit
            )),
            None => Some(
                "sinceReview is only for a PR you have already reviewed (pr.json has no myReview): use null"
                    .to_string(),
            ),
        };
        if let Some(why) = wrong {
            return Err(SubmitError::Invalid(vec![why]));
        }
    }

    let stamp = utc_stamp(now);
    let counts = AiCounts {
        attention: count(&read, Level::Attention),
        check: count(&read, Level::Check),
        drafts: read.drafts.len() as u32,
    };
    let previous = store::load_read(root, &stem).map_err(SubmitError::Failed)?;
    store::write_read(root, &stem, &Stored::of(&record.id, &stamp, session, &read))
        .map_err(SubmitError::Failed)?;

    if let Some(ai) = record.ai.as_mut() {
        ai.sha = Some(read.sha.clone());
        ai.finished_at = Some(stamp.clone());
        // The reader fetched its material after it was launched, and before it handed in.
        ai.read_from = Some(ai.launched_at.clone().unwrap_or_else(|| stamp.clone()));
        ai.counts = Some(counts);
        ai.error = None;
        // A re-read waits in line behind this hand-in; it keeps its place.
        if ai.status != "queued" {
            ai.status = "ready".to_string();
        }
    }
    record.events.ai_ready = Some(stamp);
    if let Err(e) = save(&mut record) {
        return Err(SubmitError::Failed(
            match store::restore_read(root, &stem, previous.as_deref()) {
                Ok(()) => e,
                Err(undo) => format!("{e} (and the read-through could not be put back: {undo})"),
            },
        ));
    }
    Ok(Submitted { record })
}

/// The comments of the reader's `my-last-review.json`, when it is there and can be read.
struct KnownComments {
    /// Every id in the file, replies included.
    all: Vec<u64>,
    /// The comments of the review itself: those that answer none.
    top: Vec<u64>,
}

fn known_comments(root: &Path, pr: &PrRef) -> Option<KnownComments> {
    let text =
        std::fs::read_to_string(super::reader_dir(root, pr).join("my-last-review.json")).ok()?;
    let list: serde_json::Value = serde_json::from_str(&text).ok()?;
    let comments = list.as_array()?;
    let id = |c: &serde_json::Value| c.get("id").and_then(serde_json::Value::as_u64);
    Some(KnownComments {
        all: comments.iter().filter_map(id).collect(),
        top: comments
            .iter()
            .filter(|c| {
                c.get("in_reply_to_id")
                    .is_none_or(serde_json::Value::is_null)
            })
            .filter_map(id)
            .collect(),
    })
}

/// The head the reader's `pr.json` says, when the file is there and can be read.
fn pr_json_head(root: &Path, pr: &PrRef) -> Option<String> {
    let text = std::fs::read_to_string(super::reader_dir(root, pr).join("pr.json")).ok()?;
    let view: serde_json::Value = serde_json::from_str(&text).ok()?;
    view.get("headRefOid")?.as_str().map(str::to_string)
}

fn check_asked(record: Option<Record>) -> Result<Record, SubmitError> {
    let record = record.ok_or(SubmitError::NoRecord)?;
    match record.ai {
        Some(_) => Ok(record),
        None => Err(SubmitError::NotAsked),
    }
}

fn count(read: &ReadThrough, level: Level) -> u32 {
    read.points.iter().filter(|p| p.level == level).count() as u32
}
