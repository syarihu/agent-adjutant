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
    /// The AI read-through. This module only reads it.
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

/// The AI read-through of a PR. `status` is text, not an enum, so a status a later binary adds
/// does not make this one drop the record: `queued`, `running`, `ready` or `failed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRead {
    pub status: String,
    /// The head SHA the read-through was made at.
    #[serde(default)]
    pub sha: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
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
