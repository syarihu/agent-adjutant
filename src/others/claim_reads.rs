//! Starting the readers that are next in line, at most `MAX_READING` at once.

use std::path::Path;

use crate::infra::clock::utc_stamp;
use crate::task::pr_ref;

use super::model::{MAX_READING, Record, State};
use super::store;

/// A read that may start now. The caller starts the reader, outside every lock, and calls
/// `fail_read` with `session_id` if it cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// `owner/repo#N`.
    pub id: String,
    pub url: String,
    /// The file stem of the record.
    pub stem: String,
    /// The reader's own directory, `<state>/review/<owner>+<repo>+<number>/` (the stem with `+` for `~`).
    pub dir: std::path::PathBuf,
    /// The session the reader runs as; a read-through is accepted from this one only.
    pub session_id: String,
}

/// Mark as `running` as many queued reads as there are free places, in the order they were asked
/// (`startedAt`, then `id`), and say which. Counting the running ones and marking the new ones is
/// one step under `others/review.lock`: the count spans records, and two callers that each saw
/// two running would otherwise start two more. A read that is running is never claimed again, and
/// one whose PR is done is failed, not claimed.
///
/// `new_session` makes the id of a reader's session. It is asked for before anything is marked,
/// so a failure leaves every record as it was.
pub fn claim_reads(
    root: &Path,
    now: i64,
    new_session: &dyn Fn() -> Result<String, String>,
) -> Result<Vec<Claim>, String> {
    let _lock = store::review_lock(root)?;
    // A PR that went done while it waited is not read: it is failed with the reason, and takes no
    // place from one that is open. (Failing takes the record's lock, after this one, as always.)
    for done in store::list(root)
        .into_iter()
        .filter(|r| r.state == State::Done && r.ai.as_ref().is_some_and(|ai| ai.status == "queued"))
    {
        if let Some(pr) = pr_ref(&done.url, None) {
            let _ = super::fail_read::fail_read(root, &pr, None, "the PR is no longer open", now);
        }
    }
    let records = store::list(root);
    let status = |r: &Record| r.ai.as_ref().map(|ai| ai.status.clone());
    let running = records
        .iter()
        .filter(|r| status(r).as_deref() == Some("running"))
        .count();
    let free = MAX_READING.saturating_sub(running);
    let mut queued: Vec<&Record> = records
        .iter()
        .filter(|r| status(r).as_deref() == Some("queued"))
        .collect();
    queued.sort_by_key(|r| {
        (
            r.ai.as_ref().and_then(|ai| ai.started_at.clone()),
            r.id.clone(),
        )
    });
    queued.truncate(free);
    let sessions: Vec<String> = queued
        .iter()
        .map(|_| new_session())
        .collect::<Result<_, _>>()?;

    let stamp = utc_stamp(now);
    let mut claims = Vec::new();
    let mut first_failure: Option<String> = None;
    for (listed, session_id) in queued.into_iter().zip(sessions) {
        let Some(pr) = pr_ref(&listed.url, None) else {
            continue;
        };
        let claimed = (|| {
            let stem = store::stem(&pr);
            let _record_lock = store::lock(root, &stem)?;
            // Asked again under the record's lock: it may have moved since the list was read.
            let Some(mut record) = store::load(root, &pr)? else {
                return Ok(None);
            };
            let Some(ai) = record.ai.as_mut().filter(|ai| ai.status == "queued") else {
                return Ok(None);
            };
            ai.status = "running".to_string();
            ai.launched_at = Some(stamp.clone());
            ai.session_id = Some(session_id.clone());
            store::save(root, &mut record)?;
            Ok::<_, String>(Some(Claim {
                id: record.id,
                url: record.url,
                stem,
                dir: super::reader_dir::reader_dir(root, &pr),
                session_id: session_id.clone(),
            }))
        })();
        match claimed {
            Ok(Some(claim)) => claims.push(claim),
            Ok(None) => {}
            Err(e) => {
                first_failure.get_or_insert(e);
            }
        }
    }
    match (claims.is_empty(), first_failure) {
        (true, Some(e)) => Err(e),
        _ => Ok(claims),
    }
}
