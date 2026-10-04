//! A task the dashboard was handed, as a record that outlives the message announcing it.
//!
//! The message in the inbox is the *notification* — it is read once and acked, and after
//! that the hub has no way to say what became of the thing. The record is what the board
//! reads: one file per task, rewritten in place as the work moves. Both are derived from
//! this module so that only one of them is authored: `render_request` builds the message
//! body out of the same struct the file holds, rather than a second description of a task
//! kept in step by hand.
//!
//! A leaf: it is handed the directory to work in rather than deriving it, so it never has
//! to know where this machine keeps its state.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the human is asking for. The four shapes the hub's own procedure already sorts a
/// request into ("人間に話しかけられたら" 2/3/4/5) — named here so the form can ask once
/// instead of the hub inferring it from prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// An issue that already exists. Start on it.
    Start,
    /// No issue yet. File one, then start.
    FileAndStart,
    /// Report back; file nothing, move no board.
    Investigate,
    /// A postscript for a worker that is already running.
    TellWorker,
}

/// Where the worker stops. The vocabulary the brief's 完了条件 line already uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DoneWhen {
    ReportOnly,
    Verify,
    Pr,
    Review,
}

impl DoneWhen {
    /// How the request and the brief say it. The worker branches on the first three
    /// phrases, so the brief maps `Review` to `Pr` before asking.
    pub fn as_prose(self) -> &'static str {
        match self {
            DoneWhen::ReportOnly => "investigation only (report and stop)",
            DoneWhen::Verify => "up to handing over for verification",
            DoneWhen::Pr => "up to a PR",
            DoneWhen::Review => "up to handling review",
        }
    }
}

/// Which gates wait on a person. The rest are kept as records the worker leaves and carries
/// on past. Chosen by whoever hands the task over, because whether anybody wants to look at
/// the diff or the check depends on the task, and the worker has no way to tell.
///
/// Each step includes the one before it: nobody asks to look at the check but not the diff.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopAt {
    /// The plan only. The default, and what every task did before there was a choice.
    #[default]
    Plan,
    /// The plan and the diff.
    Diff,
    /// The plan, the diff and the check.
    All,
}

impl StopAt {
    pub fn as_str(self) -> &'static str {
        match self {
            StopAt::Plan => "plan",
            StopAt::Diff => "diff",
            StopAt::All => "all",
        }
    }
}

/// Who writes the code once the plan is approved.
///
/// A worker plans every task either way: the plan is where a strong model earns its keep,
/// and an agent that implements well from a detailed design does not need to write one.
/// What changes is what the worker does after the plan gate — implement it, or hand the
/// approved plan to Jules and stop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Executor {
    /// The worker implements, reviews and opens the pull request itself.
    #[default]
    Worker,
    /// The worker hands the approved plan to a Jules session, which implements it and opens
    /// the pull request.
    Jules,
}

impl Executor {
    pub fn parse(text: &str) -> Option<Executor> {
        Some(match text {
            "worker" => Executor::Worker,
            "jules" => Executor::Jules,
            _ => return None,
        })
    }

    fn is_worker(&self) -> bool {
        *self == Executor::Worker
    }
}

/// The start of the note the hub writes when it could not start a task ("4. Start the worker",
/// "A request from the dashboard"). `next` skips a task whose note starts with it until a
/// person has looked at the reason, so the hub's wording and this check have to be one string.
pub const COULD_NOT_START: &str = "Could not start:";

/// What a free worker slot should take, and what is waiting on a confirmation nobody has been
/// asked for yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Next {
    /// The first queued task that can be started, or `None`.
    pub task: Option<Task>,
    /// Queued tasks with `autoStart: false` and no open `dispatch` gate.
    pub needs_dispatch_gate: Vec<Task>,
}

/// Pick the next task to start from `tasks`, which is expected in queue order (as `list`
/// returns it). `gated` holds the ids of tasks that already have an open `dispatch` gate.
///
/// A held task at the head must not keep everything behind it from starting, so a task
/// with `autoStart: false` is skipped until its gate is open, and a task with a note
/// starting with "Could not start:" is skipped until a person has looked at it.
pub fn next(tasks: Vec<Task>, gated: &std::collections::HashSet<String>) -> Next {
    let mut picked = None;
    let mut needs_dispatch_gate = Vec::new();
    for task in tasks {
        if task.status != Status::Queued {
            continue;
        }
        if task
            .note
            .as_deref()
            .is_some_and(|n| n.starts_with(COULD_NOT_START))
        {
            continue;
        }
        if !task.auto_start {
            if !gated.contains(&task.id) {
                needs_dispatch_gate.push(task);
            }
            continue;
        }
        if picked.is_none() {
            picked = Some(task);
        }
    }
    Next {
        task: picked,
        needs_dispatch_gate,
    }
}

/// Six states, and no more.
///
/// There is deliberately no `gate` here. Whether a task is waiting on a human is answered
/// by whether an open gate exists for it, and adding a seventh state would mean the worker
/// has to remember to write it on the way in *and* on the way out — two writes that can
/// disagree with the gate directory, which is the thing actually being described.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Written down, not handed over. Nothing is in the inbox yet.
    Backlog,
    /// Handed to the hub. Waiting for it to pick the task up.
    Queued,
    /// A worker is on it.
    Dispatched,
    /// A pull request is open and the work is out of the worker's hands.
    Pr,
    Done,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Backlog => "backlog",
            Status::Queued => "queued",
            Status::Dispatched => "dispatched",
            Status::Pr => "pr",
            Status::Done => "done",
            Status::Cancelled => "cancelled",
        }
    }

    pub fn parse(text: &str) -> Option<Status> {
        Some(match text {
            "backlog" => Status::Backlog,
            "queued" => Status::Queued,
            "dispatched" => Status::Dispatched,
            "pr" => Status::Pr,
            "done" => Status::Done,
            "cancelled" => Status::Cancelled,
            _ => return None,
        })
    }
}

/// The issue's own text as it was when the task started, kept on the record so the board can
/// show what was asked without a trip to the tracker on every poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSnapshot {
    /// The issue this was read from. A record whose URL has since changed is stale by this.
    pub url: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// The body was longer than `ISSUE_BODY_CAP` and only its start is kept.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// When it was read, in the stamp format `createdAt` uses.
    pub fetched_at: String,
}

/// How much of an issue is kept. The record is rewritten whole on every change and the board
/// sends every record on each poll, so an issue with a pasted log must not ride along in full.
pub const ISSUE_TITLE_CAP: usize = 256;
pub const ISSUE_BODY_CAP: usize = 16 * 1024;

/// Whether `url` is an issue this tool knows how to read: an http(s) URL whose path is
/// `/<owner>/<repo>/issues/<number>`, on github.com or an enterprise host alike. Other
/// trackers, and pull request URLs, are left alone rather than guessed at.
pub fn fetchable_issue(url: &str) -> bool {
    issue_parts(url).is_some()
}

/// `owner/repo#N` for an issue `fetchable_issue` accepts: the name a task made from the issue
/// carries until the issue's own title has been read.
pub fn issue_ref(url: &str) -> Option<String> {
    let (owner, repo, number) = issue_parts(url)?;
    Some(format!("{owner}/{repo}#{number}"))
}

/// The owner, repository and number of an issue URL `fetchable_issue` accepts.
fn issue_parts(url: &str) -> Option<(&str, &str, &str)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    // Cut at the first `?` or `#` before looking at the path, so a '/' inside a query or
    // fragment can never pass for one in the path.
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (host, path) = rest.split_once('/')?;
    // One trailing slash after the number is the same issue.
    let path = path.strip_suffix('/').unwrap_or(path);
    let parts: Vec<&str> = path.split('/').collect();
    let ok = !host.is_empty()
        && parts.len() == 4
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && parts[2] == "issues"
        && !parts[3].is_empty()
        && parts[3].chars().all(|c| c.is_ascii_digit());
    ok.then(|| (parts[0], parts[1], parts[3]))
}

/// The issue a parent-task key names, from the config's `issueKeys` (`owner/repo` -> key): the
/// key's prefix (`ALPHA` of `ALPHA-233`) picks the repository, ignoring case, and the number
/// is the issue's. Only on GitHub, and only when exactly one repository has that prefix: two
/// would be a guess, and a title of the wrong issue is worse than none.
pub fn parent_issue_url(
    key: &str,
    issue_keys: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let (prefix, number) = key.trim().rsplit_once('-')?;
    if prefix.is_empty() || number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut matching = issue_keys
        .iter()
        .filter(|(_, v)| v.as_str().is_some_and(|v| v.eq_ignore_ascii_case(prefix)))
        .map(|(repo, _)| repo);
    let repo = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    Some(format!("https://github.com/{repo}/issues/{number}"))
}

/// The issue to read for this task: the URL it was created with, else the one the worker
/// recorded, and only if it is one `fetchable_issue` accepts.
pub fn issue_to_fetch(task: &Task) -> Option<&str> {
    task.issue_url
        .as_deref()
        .or(task.issue.as_deref())
        .filter(|u| fetchable_issue(u))
}

/// The issue to read now, if there is one and it has not been read yet: the task is started
/// and has no snapshot of that URL. A snapshot of another URL is as good as none.
pub fn needs_snapshot(task: &Task) -> Option<&str> {
    if !matches!(task.status, Status::Dispatched | Status::Pr) {
        return None;
    }
    let url = issue_to_fetch(task)?;
    match &task.issue_snapshot {
        Some(s) if s.url == url => None,
        _ => Some(url),
    }
}

/// `text` cut to at most `cap` bytes, at a character boundary. `true` when something was cut.
fn cut_at(text: &str, cap: usize) -> (&str, bool) {
    if text.len() <= cap {
        return (text, false);
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

/// Build a snapshot from what `gh issue view --json title,body` printed.
pub fn snapshot_from_gh(json: &str, url: &str, stamp: &str) -> Result<IssueSnapshot, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("cannot read gh's answer: {e}"))?;
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or("gh's answer has no title")?;
    // An issue with no description comes back as null, not as an empty string.
    let body = value.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let title: String = title.chars().take(ISSUE_TITLE_CAP).collect();
    let (body, truncated) = cut_at(body, ISSUE_BODY_CAP);
    Ok(IssueSnapshot {
        url: url.to_string(),
        title,
        body: body.to_string(),
        truncated,
        fetched_at: stamp.to_string(),
    })
}

/// How a pull request's checks stand, counted rather than listed: the card shows one word.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckCounts {
    pub pass: u32,
    pub fail: u32,
    pub pending: u32,
}

/// What the last PR refresh read about a record's pull request, kept so the board can show it
/// without asking `gh` on every poll (#191). Holds nothing that changes by itself with time:
/// the record is only rewritten when GitHub's answer differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrStatus {
    /// `open`, `draft`, `merged` or `closed`.
    pub state: String,
    pub title: String,
    /// `approved`, `changes`, `required` (a review is still owed, whether GitHub says so or
    /// someone has been asked) or `none`.
    pub review: String,
    pub ci: CheckCounts,
}

/// A pull request as GitHub names it. Owner and repository are kept in lower case, which is
/// how GitHub itself compares them, so two spellings of one PR are one key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrRef {
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

impl PrRef {
    /// `owner/repo`, the name a notification gives a repository by.
    pub fn nwo(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

/// Owner and repository names as GitHub allows them. Checked because they travel to `gh` as
/// values, and a name that starts with `-` would be read as a flag.
fn is_name(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('-')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn is_number(text: &str) -> Option<u64> {
    (!text.is_empty() && text.chars().all(|c| c.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn pr_ref_of(host: &str, owner: &str, repo: &str, number: &str) -> Option<PrRef> {
    let host_ok = !host.is_empty()
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    (host_ok && is_name(owner) && is_name(repo)).then_some(())?;
    Some(PrRef {
        host: host.to_ascii_lowercase(),
        owner: owner.to_ascii_lowercase(),
        repo: repo.to_ascii_lowercase(),
        number: is_number(number)?,
    })
}

/// The pull request a record's `pr` names: `https://HOST/OWNER/REPO/pull/N`, with whatever
/// follows the number (`/files`, a query, a fragment) ignored, or a bare `N` / `#N` when the
/// repository the board belongs to is known (`default`: the host its origin is on, and its
/// `owner/repo`). Anything else is not guessed at, and a value that starts with `-` is
/// refused before it can reach `gh`.
pub fn pr_ref(pr: &str, default: Option<(&str, &str)>) -> Option<PrRef> {
    let pr = pr.trim();
    if pr.starts_with('-') {
        return None;
    }
    if let Some(rest) = pr
        .strip_prefix("https://")
        .or_else(|| pr.strip_prefix("http://"))
    {
        let rest = rest.split(['?', '#']).next().unwrap_or("");
        let (host, path) = rest.split_once('/')?;
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() < 4 || parts[2] != "pull" {
            return None;
        }
        return pr_ref_of(host, parts[0], parts[1], parts[3]);
    }
    let number = pr.strip_prefix('#').unwrap_or(pr);
    let (host, nwo) = default?;
    let (owner, repo) = nwo.split_once('/')?;
    pr_ref_of(host, owner, repo, number)
}

/// The pull request an API URL of a notification's subject names:
/// `https://api.github.com/repos/O/R/pulls/N`, or `https://HOST/api/v3/repos/O/R/pulls/N` on
/// an enterprise host.
pub fn pr_ref_from_api(url: &str) -> Option<PrRef> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    let (host, path) = if host == "api.github.com" {
        ("github.com", path)
    } else {
        (host, path.strip_prefix("api/v3/")?)
    };
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 5 || parts[0] != "repos" || parts[3] != "pulls" {
        return None;
    }
    pr_ref_of(host, parts[1], parts[2], parts[4])
}

/// Whose turn a pull request is, read from what GitHub said about it. Derived on every read
/// and never stored: the record keeps the facts (`PrStatus`) and the rule can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrTurn {
    /// Still being prepared.
    Draft,
    /// Ready, and nobody has been asked or has answered: today's rule decides.
    Unrequested,
    /// Ready, and another reviewer has been asked and has not decided.
    OtherReviewer,
    /// Waiting on review bots or CI.
    Checks,
    /// A reviewer asked for changes.
    Changes,
    /// Approved: only the merge is left.
    Merge,
    CiFailed,
    Merged,
    /// Closed without being merged.
    Closed,
}

impl PrTurn {
    /// Whether the person is the one to act: they answer a review, merge, look at a failure,
    /// or decide what a closed PR means.
    pub fn persons(self) -> bool {
        matches!(
            self,
            PrTurn::Changes | PrTurn::Merge | PrTurn::CiFailed | PrTurn::Closed
        )
    }
}

/// The turn a summary stands for. A failed check outranks one still running, and a request
/// for changes outranks both: what the person can act on now comes first.
pub fn pr_turn(status: &PrStatus) -> Option<PrTurn> {
    match status.state.as_str() {
        "merged" => return Some(PrTurn::Merged),
        "closed" => return Some(PrTurn::Closed),
        "draft" => return Some(PrTurn::Draft),
        "open" => {}
        _ => return None,
    }
    Some(if status.review == "changes" {
        PrTurn::Changes
    } else if status.ci.fail > 0 {
        PrTurn::CiFailed
    } else if status.ci.pending > 0 {
        PrTurn::Checks
    } else if status.review == "approved" {
        PrTurn::Merge
    } else if status.review == "required" {
        PrTurn::OtherReviewer
    } else {
        PrTurn::Unrequested
    })
}

/// Whether a Jules task's pull request is the person's ball, once Jules is not working on it
/// (which only the board's own poll knows, so this is asked after that). The same turn as
/// `pr_waits_on_person`, with no worker phase to consult: the person's turn waits; another
/// reviewer's, the bots' or a merge does not; a turn that says nothing (a draft, one nobody was
/// asked, one not read yet) waits as it always did. `humanColOf` in `src/ui/core.js` mirrors it.
pub fn jules_pr_waits_on_person(pr_status: Option<&PrStatus>) -> bool {
    match pr_status.and_then(pr_turn) {
        Some(turn) if turn.persons() => true,
        Some(PrTurn::Checks | PrTurn::OtherReviewer | PrTurn::Merged) => false,
        _ => true,
    }
}

/// Whether a task's pull request is the person's ball: the one rule the board's column and
/// the sidebar's count both follow, and which `humanColOf` in `src/ui/core.js` mirrors.
///
/// `phase` is what the task's worker record says: `None` when there is no record, `Some(None)`
/// when there is one with no phase. A worker still in a phase other than `pr` / `pr-bots` is
/// at work, so the card stays on the agent board. Otherwise a turn that is the person's puts
/// it in their column, even while the worker's phase says `pr-bots`; a turn that is somebody
/// else's, or nobody's, keeps it out; and a PR whose turn says nothing (a draft, one nobody
/// was asked to review, one not read yet) is decided the way it was before the turn was
/// known: by the phase, or by the status when there is no record.
pub fn pr_waits_on_person(
    status: Status,
    has_pr: bool,
    pr_status: Option<&PrStatus>,
    phase: Option<Option<&str>>,
) -> bool {
    if !has_pr || matches!(status, Status::Done | Status::Cancelled) {
        return false;
    }
    if let Some(phase) = phase
        && !matches!(phase, Some("pr" | "pr-bots"))
    {
        return false;
    }
    match pr_status.and_then(pr_turn) {
        Some(turn) if turn.persons() => true,
        Some(PrTurn::Checks | PrTurn::OtherReviewer | PrTurn::Merged) => false,
        _ => match phase {
            Some(phase) => phase == Some("pr"),
            None => status == Status::Pr,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_url: Option<String>,
    pub done_when: DoneWhen,
    /// Absent in a record written before there was a choice, which is what `Plan` means.
    #[serde(default)]
    pub stop_at: StopAt,
    /// Absent means the worker, which is what every record written before there was a
    /// choice did. Left out when it is the worker, so those records read back unchanged.
    #[serde(default, skip_serializing_if = "Executor::is_worker")]
    pub executor: Executor,
    /// What this one dispatch should branch from. `None` = the repository's `baseBranch`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The parent task's URL. A key would make the worker look the tracker up; a URL says
    /// which tracker it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Needed only when there is no issue to take a name from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
    /// Whether the hub may dispatch this without asking. `false` becomes a question for the
    /// human rather than a halt: the hub is not allowed to block on one.
    pub auto_start: bool,
    /// Position in the queue. The human owns this; the hub reads it to pick what is next.
    #[serde(default)]
    pub order: u32,
    pub status: Status,
    // ── filled in as the work moves ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    /// The Jules session implementing this task, once the worker has handed it over. The id
    /// alone: the board asks the API for the state, which changes long after the worker has
    /// gone, rather than keeping a copy here that would go stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jules_session: Option<String>,
    /// The GitHub account that started the session, as `gh` named it then. Jules answers that
    /// account's comments and nobody else's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jules_by: Option<String>,
    /// Review comments already passed on to Jules, by their GitHub id. Kept so the board can
    /// say which ones went, and so one comment is not handed over twice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relayed: Vec<String>,
    /// Review comments the board has told the hub about, so each one is brought up once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub announced: Vec<String>,
    /// How many times the board has told the hub about new review comments on this task's PR.
    /// Past a limit it stops, and passing comments on is left to a person: a reviewer and Jules
    /// answering each other's pushes can otherwise go round without end.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub relay_rounds: u32,
    /// Why the hub could not take it, when that is the answer. Written where the reply to
    /// the requester would have gone, because for a dashboard request there is no session
    /// to reply to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Handover instruction for the agent when queued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    /// When a gate for this task was last answered or closed. The board counts the worker's
    /// time in a phase from here when it is later than the phase's start: waiting on a person
    /// is not the worker being stuck. Kept on the record, written as the gate is answered, so
    /// the board does not have to read the whole archive of answered gates on every poll.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_answered_at: Option<String>,
    /// The issue's title and body as read when the task started; see `IssueSnapshot`. Written
    /// only by a fetch, never taken from a caller's JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_snapshot: Option<IssueSnapshot>,
    /// The title was made from the issue URL (`owner/repo#N`) because the issue could not be
    /// read when the task was created. The first successful read replaces it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub title_pending: bool,
    /// The pull request as the last refresh read it; see `PrStatus`. Written only by a
    /// refresh, never taken from a caller's JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_status: Option<PrStatus>,
    pub created_at: String,
    pub updated_at: String,
    /// Keys this binary does not know, kept from the file so that a record written by another
    /// version and saved by this one loses nothing. Only ever filled from disk: `create` empties it.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// Where this hub's tasks live. Beside the inbox rather than inside it: the inbox is a
/// queue that drains, and a task record has to still be there after its message is acked.
pub fn dir(state_dir: &Path, slug: &str) -> PathBuf {
    state_dir.join("tasks").join(slug)
}

/// `20260922T041233Z-login-retry`. The stamp comes from the caller so this stays a leaf —
/// and so a test can pin it.
pub fn new_id(stamp: &str, title: &str) -> String {
    let slug = slug(title);
    if slug.is_empty() {
        stamp.to_string()
    } else {
        format!("{stamp}-{slug}")
    }
}

/// Lower-cased ASCII words joined by hyphens, cut short. Anything else — Japanese, most
/// punctuation — is a separator rather than transliterated: a filename is not where a title
/// is preserved, and the title itself is right there in the record.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 32 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

/// Whether `id` can only name a file inside the task directory. Ids reach here from a request
/// body and a URL, and a `/` or a `..` in one would name a file somewhere else.
pub fn is_plain_id(id: &str) -> bool {
    !id.is_empty() && id != ".." && !id.contains(['/', '\\', '\0']) && !id.contains("..")
}

pub fn path_of(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// Take an id nobody else holds, and hold it.
///
/// The stamp has one-second resolution and a Japanese title slugs to nothing, so two tasks
/// written in the same second are not a rare case — it is what filling the form twice looks
/// like, and the second one would land on the first one's file and erase it.
///
/// Claimed with `create_new` rather than checked with `exists` first: the dashboard and the
/// command line can both be creating one, and check-then-write leaves a window where both
/// see the name free. `mail::send` names inbox files the same way, for the same reason.
pub fn claim_id(dir: &Path, stamp: &str, title: &str) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let base = new_id(stamp, title);
    for seq in 1..1000 {
        let id = if seq == 1 {
            base.clone()
        } else {
            format!("{base}-{seq}")
        };
        match crate::infra::fs::create_new(&path_of(dir, &id)) {
            Ok(true) => return Ok(id),
            Ok(false) => continue,
            Err(e) => {
                return Err(format!(
                    "cannot create a task file in {}: {e}",
                    dir.display()
                ));
            }
        }
    }
    Err(format!("no free task id for {base}"))
}

/// Write the record, creating the directory if this is the first one.
///
/// Written whole each time rather than patched: every caller already holds the struct it
/// wants on disk, and a partial write is how two writers end up with a record neither of
/// them would recognise.
///
/// Staged as a dotfile beside the record, synced and renamed over it, because the resident
/// server's PR poll writes records while `adj task show` and the board read them: a plain
/// write truncates first, and a reader in between sees an empty file. `list` skips the
/// staged name, so it never picks one up.
pub fn save(dir: &Path, task: &Task) -> Result<PathBuf, String> {
    let path = path_of(dir, &task.id);
    crate::infra::fs::write_json(&path, task)?;
    Ok(path)
}

pub fn load(dir: &Path, id: &str) -> Result<Task, String> {
    if !is_plain_id(id) {
        return Err(format!("no such task: {id}"));
    }
    let path = path_of(dir, id);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no such task: {id}"))?;
    serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Every task this hub knows about, queue order first.
///
/// A file that will not parse is skipped rather than fatal. The board is a view of a
/// directory somebody may have hand-edited, and one bad file must not blank the page.
pub fn list(dir: &Path) -> Vec<Task> {
    let mut tasks: Vec<Task> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|text| serde_json::from_str::<Task>(&text).ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    tasks.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });
    tasks
}

/// The body of the `request` message that tells the hub this task exists.
///
/// Derived from the record rather than typed beside it: the server writes the file and then
/// renders this, so the hub can work from `adj pending --read` alone without opening the
/// record, and the two can never drift.
pub fn render_request(task: &Task) -> String {
    let mut out = String::new();
    out.push_str(&format!("## task          {}\n", task.id));
    out.push_str(&format!(
        "## Kind          {}\n",
        match task.kind {
            Kind::Start => "start an issue",
            Kind::FileAndStart => "file and start",
            Kind::Investigate => "investigation only (report and stop)",
            Kind::TellWorker => "more instructions for an existing worktree",
        }
    ));
    out.push_str(&format!("## Done when     {}\n", task.done_when.as_prose()));
    // The value itself goes first: the hub passes it on as `--stop-at` and into the brief,
    // and the gloss is for whoever reads the message.
    out.push_str(&format!(
        "## Stop at       {} ({})\n",
        task.stop_at.as_str(),
        match task.stop_at {
            StopAt::Plan => "wait for plan approval only",
            StopAt::Diff => "wait for plan approval and the diff review",
            StopAt::All => "wait for plan approval, the diff review and verification",
        }
    ));
    // Only when it is not the worker, so a request reads the way it always has for the tasks
    // that did not choose.
    if task.executor == Executor::Jules {
        out.push_str("## Implementer   jules (handed to Jules once the plan is approved)\n");
    }
    let line = |label: &str, value: Option<&str>| format!("## {label}{}\n", value.unwrap_or("-"));
    out.push_str(&line("Issue         ", task.issue_url.as_deref()));
    out.push_str(&line("Base          ", task.base.as_deref()));
    out.push_str(&line("Parent task   ", task.parent.as_deref()));
    out.push_str(&line("Worktree name ", task.worktree_name.as_deref()));
    out.push_str(&format!(
        "## Start         {}\n",
        if task.auto_start {
            "start without asking"
        } else {
            "ask before starting"
        }
    ));
    if task.title_pending {
        out.push_str("## Title         not read yet (the issue could not be read; the title is its reference)\n");
    }
    out.push_str("\n## Body\n\n");
    out.push_str(task.body.trim_end());
    out.push('\n');
    if let Some(instruction) = task.instruction.as_deref().filter(|s| !s.trim().is_empty()) {
        out.push_str("\n## Handover note\n\n");
        out.push_str(instruction.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests;
