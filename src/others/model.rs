//! A PR that asked for your review, as the record holds it, and the state derived from it.
//!
//! The JSON of these types is what the board page and the review hub read, so the names are a
//! contract: see "PRs others asked you to review" in `docs/architecture.md`.

use serde::{Deserialize, Serialize};

/// One PR that asked for your review, as `others/<owner>~<repo>~<number>.json` holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// `owner/repo#N`, owner and repo in lower case: the key. Never changes.
    pub id: String,
    /// `Owner/Repo` as GitHub spells it.
    pub repo: String,
    pub number: u64,
    pub url: String,
    /// Whether `owner/repo` (lower case) is in the address book at the last sync.
    pub registered: bool,
    pub title: String,
    /// The author's login; `None` for a deleted account.
    #[serde(default)]
    pub author: Option<String>,
    pub base: String,
    pub head: String,
    pub head_sha: String,
    /// Stamp of the sync that set `head_sha` or changed it. A read-through made after it, of a
    /// head the sync has not seen yet, is not stale for that reason; see `is_stale`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_seen_at: Option<String>,
    #[serde(default)]
    pub draft: bool,
    pub pr_state: PrOpenState,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    #[serde(default)]
    pub files: Vec<FileStat>,
    /// `false` when `files` holds fewer than `changed_files` entries (more than the pages read).
    #[serde(default)]
    pub files_complete: bool,
    #[serde(default)]
    pub ci: crate::task::CheckCounts,
    #[serde(default)]
    pub reviewers: Vec<Reviewer>,
    /// Your last submitted review; a pending one never counts.
    #[serde(default)]
    pub my_review: Option<MyReview>,
    /// Commits on the head since `my_review.commit`. `None` when unknown: no review, or the
    /// reviewed commit is no longer among the last 100 (a force-push). Never 0 for unknown.
    #[serde(default)]
    pub commits_since_review: Option<u32>,
    /// You (not a team of yours) are a requested reviewer right now.
    #[serde(default)]
    pub requested: bool,
    /// Stamp of the latest request that named you: the current one.
    #[serde(default)]
    pub requested_at: Option<String>,
    /// The current request came after a review of yours.
    #[serde(default)]
    pub rerequest: bool,
    /// Derived. It is read from disk only so the key is not caught by `extra`: `derive`
    /// overwrites it on every load and every save.
    #[serde(default)]
    pub state: State,
    /// Derived like `state`; set only while `state` is done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_reason: Option<DoneReason>,
    /// Stamp of the sync that first derived done; cleared when the record leaves done.
    #[serde(default)]
    pub done_at: Option<String>,
    #[serde(default)]
    pub events: Events,
    /// The AI read-through: where it stands, and the numbers the row shows. The read-through
    /// itself is in `others/reads/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiRead>,
    pub first_seen_at: String,
    /// Stamp of the last sync that read this PR.
    pub read_at: String,
    /// Keys this binary does not know (architecture rule 10).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Where a PR stands for you. Never trusted from disk: `derive` sets it on every load and save.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    #[default]
    Requested,
    AiReading,
    AiReady,
    Pushed,
    WaitingOnAuthor,
    Done,
    /// A value a newer binary wrote. `derive` never produces it, so this binary never writes it.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DoneReason {
    Approved,
    Merged,
    Closed,
    /// The request was taken away before you reviewed, or after your review.
    Withdrawn,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrOpenState {
    #[default]
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStat {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
    /// GitHub's change type in lower case: `added`, `deleted`, `modified`, `renamed`, `copied`
    /// or `changed`.
    pub change: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reviewer {
    /// A user's login, or a team's `org/slug`.
    pub login: String,
    #[serde(default)]
    pub team: bool,
    pub state: ReviewState,
    /// Stamp of the review; `None` for a request.
    #[serde(default)]
    pub at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    /// GitHub's dismissed maps here too.
    Commented,
    Requested,
    /// A value a newer binary wrote; never produced here.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MyReview {
    pub state: ReviewState,
    pub submitted_at: String,
    /// The commit the review was made on.
    pub commit: String,
}

/// What happened to the PR, as stamps: what a list counts as new.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Events {
    #[serde(default)]
    pub requested: Option<String>,
    #[serde(default)]
    pub rerequested: Option<String>,
    /// Written by whoever runs the read-through.
    #[serde(default)]
    pub ai_ready: Option<String>,
    #[serde(default)]
    pub pushed: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// How many readers run at once; the rest wait their turn as `queued`.
pub const MAX_READING: usize = 3;

/// The AI read-through of a PR. `status` is text, not an enum, so a status a later binary adds
/// does not make this one drop the record: `queued`, `running`, `ready` or `failed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRead {
    pub status: String,
    /// The head SHA the read-through was made at.
    #[serde(default)]
    pub sha: Option<String>,
    /// When the person asked (the latest request), which is also the place in the queue.
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    /// When the reader was started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launched_at: Option<String>,
    /// When the stored read-through's material was taken, at the latest: the launch of the reader
    /// that handed it in, or the time of a hand-in with no reader. What `stale` is judged against,
    /// not the hand-in time, which comes after the reader read the head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_from: Option<String>,
    /// The session the current reader runs as. A read-through is accepted from this one only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ps_started: Option<String>,
    /// Where the reader's window is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<crate::infra::terminal::SessionTerminal>,
    /// Why the last read failed; set with `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What the stored read-through holds, for the row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<AiCounts>,
    /// Derived: the read-through was made at another head than the PR has now. Read from disk
    /// only so the key is not caught by `extra`; every load and save overwrites it.
    #[serde(default)]
    pub stale: bool,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The counts of a read-through that the row shows, taken when it was handed in. Stored in the
/// record, so a count a newer binary adds is kept, and one it leaves out is read as zero rather
/// than making the record unreadable (architecture rule 10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiCounts {
    /// Points of level `attention`.
    #[serde(default)]
    pub attention: u32,
    /// Points of level `check`.
    #[serde(default)]
    pub check: u32,
    #[serde(default)]
    pub drafts: u32,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// `others/sync.json`: how the last sync went.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSync {
    /// Stamp of the last sync whose search succeeded. `None` until the first one.
    #[serde(default)]
    pub at: Option<String>,
    /// `false` when some PR could not be read (listed in `failed`) or the deadline cut it short.
    #[serde(default)]
    pub complete: bool,
    /// The owners searched, lower case, sorted.
    #[serde(default)]
    pub owners: Vec<String>,
    /// The search returned its limit; there may be more.
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub failed: Vec<SyncFailure>,
    /// The last sync that failed as a whole, and why. Cleared by the next successful one.
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_at: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncFailure {
    pub id: String,
    pub why: String,
}

/// Whether the read-through was made at a head the PR no longer has. Pure, like `derive`.
///
/// `sha` is the head the reader read, which is the PR's live head and can be newer than the
/// `head_sha` the last sync saw. So a read is stale only when the sync saw its head at or after
/// the time the reader took its material (`head_seen_at` not before `read_from`), or when either
/// time is missing (a record from before the stamps): then the heads that differ are all there is
/// to go on. The hand-in time is not used: a reader that read A, a push of B that a sync stored,
/// and then a hand-in of A, is stale. The next sync that brings the newer head makes the two equal.
pub fn is_stale(r: &Record) -> bool {
    let Some(ai) = r.ai.as_ref() else {
        return false;
    };
    let Some(sha) = ai.sha.as_deref() else {
        return false;
    };
    if sha == r.head_sha {
        return false;
    }
    match (r.head_seen_at.as_deref(), ai.read_from.as_deref()) {
        (Some(seen), Some(from)) => seen >= from,
        _ => true,
    }
}

/// The state of a record and, when done, why. Pure: it reads the facts and no clock, so the
/// same record derives the same on every load.
pub fn derive(r: &Record) -> (State, Option<DoneReason>) {
    match r.pr_state {
        PrOpenState::Merged => return (State::Done, Some(DoneReason::Merged)),
        PrOpenState::Closed => return (State::Done, Some(DoneReason::Closed)),
        PrOpenState::Open => {}
    }
    let pushed = r.my_review.as_ref().is_some_and(|m| m.commit != r.head_sha);
    let base = if r.requested {
        if pushed {
            State::Pushed
        } else {
            State::Requested
        }
    } else {
        match &r.my_review {
            // Asked, and the request was taken away before you reviewed.
            None => return (State::Done, Some(DoneReason::Withdrawn)),
            // Asked again after your review, and that request went away too.
            Some(m)
                if r.requested_at
                    .as_deref()
                    .is_some_and(|at| at > m.submitted_at.as_str()) =>
            {
                return (State::Done, Some(DoneReason::Withdrawn));
            }
            Some(m) if m.state == ReviewState::Approved => {
                return (State::Done, Some(DoneReason::Approved));
            }
            Some(_) if pushed => State::Pushed,
            Some(_) => State::WaitingOnAuthor,
        }
    };
    // The read-through shows only on a PR waiting for its first review; on a pushed one it is
    // beside the state, not instead of it.
    let state = match (base, r.ai.as_ref().map(|a| a.status.as_str())) {
        (State::Requested, Some("queued" | "running")) => State::AiReading,
        (State::Requested, Some("ready")) => State::AiReady,
        (state, _) => state,
    };
    (state, None)
}
