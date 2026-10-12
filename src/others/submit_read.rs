//! Handing in a read-through: checked, stored, and the record told it is ready.

use std::path::Path;

use crate::infra::clock::utc_stamp;
use crate::task::PrRef;

use super::diff_get::{MAX_DIFF_BYTES, StoredDiff};
use super::model::{AiCounts, ReadMode, Record};
use super::read_through::{Level, MAX_READ_BYTES, ReadThrough, Stored, validate, validate_split};
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
/// Everything that can refuse does so before the first write. The diff and the file are then
/// written first and the record second, and a record that cannot be saved takes both back, so
/// they are never half changed.
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

    // What was asked for, as the record has it under the lock, not what the file says it is.
    let mode = ReadMode::of(record.ai.as_ref());
    if mode == ReadMode::Split {
        validate_split(&read).map_err(SubmitError::Invalid)?;
    }

    // A PR you reviewed that has moved is a re-review, and a re-review says what became of your
    // comments. `sha` is the head the reader read, `commit` the one you reviewed. A split
    // proposal compares nothing with it.
    if mode == ReadMode::Read
        && let Some(mine) = &record.my_review
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
        extra: Default::default(),
    };
    // Read before the first write, so that an unreadable `diff.patch` refuses the hand-in whole.
    // A split proposal keeps no diff: the reader did not read one, so the 差分 tab has nothing
    // of this head to show and an earlier diff is removed below.
    let diff = match mode {
        ReadMode::Read => reader_diff(root, pr, &read.sha, &stamp).map_err(SubmitError::Failed)?,
        ReadMode::Split => None,
    };
    let previous = store::load_read(root, &stem).map_err(SubmitError::Failed)?;
    let previous_diff = store::load_diff(root, &stem).map_err(SubmitError::Failed)?;
    // The diff goes with the read-through or not at all: one of another head is never left to be
    // served beside this read.
    match &diff {
        Some(diff) => store::write_diff(root, &stem, diff),
        None => store::remove_diff(root, &stem),
    }
    .map_err(SubmitError::Failed)?;
    if let Err(e) = store::write_read(
        root,
        &stem,
        &Stored::of(&record.id, &stamp, session, &read, mode),
    ) {
        return Err(SubmitError::Failed(
            match store::restore_diff(root, &stem, previous_diff.as_deref()) {
                Ok(()) => e,
                Err(undo) => format!("{e} (and the diff could not be put back: {undo})"),
            },
        ));
    }

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
        // Newest first.
        let mut message = e;
        if let Err(undo) = store::restore_read(root, &stem, previous.as_deref()) {
            message = format!("{message} (and the read-through could not be put back: {undo})");
        }
        if let Err(undo) = store::restore_diff(root, &stem, previous_diff.as_deref()) {
            message = format!("{message} (and the diff could not be put back: {undo})");
        }
        return Err(SubmitError::Failed(message));
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

/// The diff to keep with the read-through of head `sha`: the reader's `diff.patch`, when
/// `pr.json` says the reader read that head. `None` when the diff is not known to be of `sha`
/// (no `pr.json`, or another head: a hand-in by a person with no session is let through that way)
/// or when `diff.patch` is not there, which is how the material looks when GitHub would not give
/// the diff. A `diff.patch` that is there and cannot be read is an error, not an absence, and an
/// empty one is an answer: a PR with no changes.
fn reader_diff(
    root: &Path,
    pr: &PrRef,
    sha: &str,
    stamp: &str,
) -> Result<Option<StoredDiff>, String> {
    if pr_json_head(root, pr).as_deref() != Some(sha) {
        return Ok(None);
    }
    let path = super::reader_dir(root, pr).join("diff.patch");
    // Not followed: `metadata` and `read` follow a link, so a link would be a way to put another file's text in the diff.
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    if !meta.file_type().is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    let size = meta.len();
    // Over the cap only the size is kept, so the file is not read at all.
    let diff = if size > MAX_DIFF_BYTES as u64 {
        None
    } else {
        // A diff can carry bytes that are not UTF-8 (a file of another encoding): they are shown
        // as the replacement character rather than refusing the hand-in.
        let bytes =
            std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    };
    Ok(Some(StoredDiff {
        sha: sha.to_string(),
        saved_at: stamp.to_string(),
        bytes: size,
        diff,
    }))
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
