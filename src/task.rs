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
/// see the name free. `messaging::send` names inbox files the same way, for the same reason.
pub fn claim_id(dir: &Path, stamp: &str, title: &str) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let base = new_id(stamp, title);
    for seq in 1..1000 {
        let id = if seq == 1 {
            base.clone()
        } else {
            format!("{base}-{seq}")
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path_of(dir, &id))
        {
            Ok(_) => return Ok(id),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
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
/// Written beside the record and renamed over it, because the resident server's PR poll
/// writes records while `adj task show` and the board read them: a plain write truncates
/// first, and a reader in between sees an empty file. The temporary name does not end in
/// `.json`, so `list` never picks it up.
pub fn save(dir: &Path, task: &Task) -> Result<PathBuf, String> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = path_of(dir, &task.id);
    let json = serde_json::to_string_pretty(task).map_err(|e| e.to_string())?;
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{}.{}-{seq}.tmp", task.id, std::process::id()));
    let written =
        std::fs::write(&tmp, format!("{json}\n")).and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("cannot write {}: {e}", path.display()));
    }
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
mod tests {
    use super::*;

    /// Production builds a task out of the form's JSON; this is the shorthand the tests
    /// need and nothing else does.
    impl Task {
        /// A task as the form hands it over. Everything the hub fills in later is absent.
        pub fn new(
            id: String,
            kind: Kind,
            title: String,
            done_when: DoneWhen,
            stamp: &str,
        ) -> Task {
            Task {
                id,
                kind,
                title,
                body: String::new(),
                issue_url: None,
                done_when,
                stop_at: StopAt::default(),
                executor: Executor::default(),
                base: None,
                parent: None,
                worktree_name: None,
                auto_start: true,
                order: 0,
                status: Status::Backlog,
                worktree: None,
                issue: None,
                pr: None,
                jules_session: None,
                jules_by: None,
                relayed: Vec::new(),
                announced: Vec::new(),
                relay_rounds: 0,
                note: None,
                instruction: None,
                gate_answered_at: None,
                issue_snapshot: None,
                title_pending: false,
                pr_status: None,
                created_at: stamp.to_string(),
                updated_at: stamp.to_string(),
            }
        }
    }

    fn sample() -> Task {
        let mut task = Task::new(
            new_id("20260922T041233Z", "Fix the login retry"),
            Kind::Investigate,
            "ログインのリトライを調べる".to_string(),
            DoneWhen::ReportOnly,
            "20260922T041233Z",
        );
        task.body = "The retry does not seem to take effect".to_string();
        task
    }

    fn snap(url: &str) -> IssueSnapshot {
        IssueSnapshot {
            url: url.to_string(),
            title: "T".to_string(),
            body: "B".to_string(),
            truncated: false,
            fetched_at: "20260922T041233Z".to_string(),
        }
    }

    #[test]
    fn a_snapshot_survives_a_save_and_load_and_an_absent_one_is_not_written() {
        let dir = std::env::temp_dir().join(format!("adj-snap-{}", std::process::id()));
        let mut task = sample();
        task.issue_snapshot = Some(snap("https://github.com/a/b/issues/1"));
        save(&dir, &task).unwrap();
        assert_eq!(load(&dir, &task.id).unwrap(), task);
        let text = std::fs::read_to_string(path_of(&dir, &task.id)).unwrap();
        assert!(text.contains("\"issueSnapshot\""), "{text}");
        assert!(text.contains("\"fetchedAt\""), "{text}");
        assert!(!text.contains("truncated"), "{text}");

        task.issue_snapshot = None;
        save(&dir, &task).unwrap();
        let text = std::fs::read_to_string(path_of(&dir, &task.id)).unwrap();
        assert!(!text.contains("issueSnapshot"), "{text}");
        assert_eq!(load(&dir, &task.id).unwrap().issue_snapshot, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_github_style_issue_url_is_fetchable() {
        for ok in [
            "https://github.com/a/b/issues/12",
            "https://github.com/a/b/issues/12#issuecomment-1",
            "https://github.com/a/b/issues/12?x=1",
            "https://github.com/a/b/issues/12/",
            "https://github.com/a/b/issues/12/#x",
            "https://ghe.example.com/a/b/issues/7",
        ] {
            assert!(fetchable_issue(ok), "{ok}");
        }
        for bad in [
            "https://github.com/a/b/pull/12",
            "https://github.com/a/b/issues/",
            "https://github.com/a/b/issues/x1",
            "https://github.com/a/b/issues/12x",
            "https://github.com/a/b/issues/12/anything",
            "https://github.com/a/b/issues/12//",
            "https://h/a?x/b/issues/1",
            "https://h/a#x/b/issues/1",
            "https://linear.app/team/issue/ABC-1/title",
            "https://example.atlassian.net/browse/ABC-1",
            "-x",
            "file:///a/b/issues/1",
            "",
        ] {
            assert!(!fetchable_issue(bad), "{bad}");
        }
    }

    #[test]
    fn an_issue_ref_names_owner_repo_and_number() {
        assert_eq!(
            issue_ref("https://github.com/a/b/issues/12#issuecomment-1").as_deref(),
            Some("a/b#12")
        );
        assert_eq!(
            issue_ref("https://ghe.example.com/a/b/issues/7/").as_deref(),
            Some("a/b#7")
        );
        assert_eq!(issue_ref("https://github.com/a/b/pull/12"), None);
        assert_eq!(issue_ref("https://linear.app/x/issue/ABC-1"), None);
    }

    #[test]
    fn the_issue_url_is_read_before_the_issue_the_worker_recorded() {
        let mut task = sample();
        task.issue = Some("https://github.com/a/b/issues/2".to_string());
        assert_eq!(
            issue_to_fetch(&task),
            Some("https://github.com/a/b/issues/2")
        );
        task.issue_url = Some("https://github.com/a/b/issues/1".to_string());
        assert_eq!(
            issue_to_fetch(&task),
            Some("https://github.com/a/b/issues/1")
        );
        task.issue_url = Some("https://linear.app/t/issue/A-1".to_string());
        assert_eq!(issue_to_fetch(&task), None);
    }

    #[test]
    fn a_snapshot_is_wanted_once_the_task_is_started_and_none_of_that_url_is_kept() {
        let url = "https://github.com/a/b/issues/1";
        let mut task = sample();
        task.issue_url = Some(url.to_string());
        for status in [
            Status::Backlog,
            Status::Queued,
            Status::Done,
            Status::Cancelled,
        ] {
            task.status = status;
            assert_eq!(needs_snapshot(&task), None, "{}", status.as_str());
        }
        for status in [Status::Dispatched, Status::Pr] {
            task.status = status;
            assert_eq!(needs_snapshot(&task), Some(url), "{}", status.as_str());
        }
        task.issue_snapshot = Some(snap(url));
        assert_eq!(needs_snapshot(&task), None);
        task.issue_url = Some("https://github.com/a/b/issues/2".to_string());
        assert_eq!(
            needs_snapshot(&task),
            Some("https://github.com/a/b/issues/2")
        );
    }

    #[test]
    fn a_snapshot_is_cut_to_the_caps_and_a_null_body_is_empty() {
        let url = "https://github.com/a/b/issues/1";
        let s = snapshot_from_gh(r#"{"title":"T","body":null}"#, url, "s").unwrap();
        assert_eq!(
            (s.title.as_str(), s.body.as_str(), s.truncated),
            ("T", "", false)
        );

        let big = "あ".repeat(ISSUE_BODY_CAP);
        let json = serde_json::json!({"title": "t".repeat(400), "body": big}).to_string();
        let s = snapshot_from_gh(&json, url, "s").unwrap();
        assert!(s.truncated);
        assert!(s.body.len() <= ISSUE_BODY_CAP && s.body.len() > ISSUE_BODY_CAP - 4);
        assert_eq!(s.title.chars().count(), ISSUE_TITLE_CAP);

        let json = serde_json::json!({"title": "t", "body": "x".repeat(ISSUE_BODY_CAP)});
        let s = snapshot_from_gh(&json.to_string(), url, "s").unwrap();
        assert!(!s.truncated);

        assert!(snapshot_from_gh("not json", url, "s").is_err());
        assert!(snapshot_from_gh(r#"{"body":"x"}"#, url, "s").is_err());
    }

    #[test]
    fn a_parent_key_names_the_issue_of_the_one_repository_with_that_prefix() {
        let keys = serde_json::json!({"acme/team-app": "ALPHA", "acme/api": "BETA"});
        let keys = keys.as_object().unwrap();
        assert_eq!(
            parent_issue_url("ALPHA-233", keys).as_deref(),
            Some("https://github.com/acme/team-app/issues/233")
        );
        assert_eq!(
            parent_issue_url("alpha-7", keys).as_deref(),
            Some("https://github.com/acme/team-app/issues/7")
        );
        assert_eq!(parent_issue_url("GAMMA-1", keys), None);
        assert_eq!(parent_issue_url("ALPHA", keys), None);
        assert_eq!(parent_issue_url("ALPHA-x", keys), None);
        assert_eq!(parent_issue_url("-12", keys), None);
        let twice = serde_json::json!({"acme/a": "ALPHA", "acme/b": "alpha"});
        assert_eq!(
            parent_issue_url("ALPHA-1", twice.as_object().unwrap()),
            None
        );
    }

    #[test]
    fn next_is_the_first_queued_task_and_ignores_every_other_status() {
        let mut t1 = sample();
        t1.id = "1".to_string();
        t1.status = Status::Backlog;
        let mut t2 = sample();
        t2.id = "2".to_string();
        t2.status = Status::Dispatched;
        let mut t3 = sample();
        t3.id = "3".to_string();
        t3.status = Status::Queued;
        let mut t4 = sample();
        t4.id = "4".to_string();
        t4.status = Status::Queued;

        let gated = std::collections::HashSet::new();
        let next_result = next(vec![t1, t2, t3.clone(), t4], &gated);
        assert_eq!(next_result.task, Some(t3));
        assert!(next_result.needs_dispatch_gate.is_empty());
    }

    #[test]
    fn a_task_that_asks_first_is_passed_over_and_listed_until_its_gate_is_open() {
        let mut t1 = sample();
        t1.id = "1".to_string();
        t1.status = Status::Queued;
        t1.auto_start = false;
        let mut t2 = sample();
        t2.id = "2".to_string();
        t2.status = Status::Queued;
        t2.auto_start = true;

        let mut gated = std::collections::HashSet::new();
        let next_result = next(vec![t1.clone(), t2.clone()], &gated);
        assert_eq!(next_result.task, Some(t2.clone()));
        assert_eq!(next_result.needs_dispatch_gate, vec![t1.clone()]);

        gated.insert("1".to_string());
        let next_result_gated = next(vec![t1, t2.clone()], &gated);
        assert_eq!(next_result_gated.task, Some(t2));
        assert!(next_result_gated.needs_dispatch_gate.is_empty());
    }

    #[test]
    fn a_task_that_could_not_start_waits_for_a_person() {
        let mut t1 = sample();
        t1.id = "1".to_string();
        t1.status = Status::Queued;
        t1.note = Some(format!("{COULD_NOT_START} fatal: bad base"));
        let mut t2 = sample();
        t2.id = "2".to_string();
        t2.status = Status::Queued;
        t2.note = Some("Waiting for a worker slot (resume with --resume)".to_string());
        let mut t3 = sample();
        t3.id = "3".to_string();
        t3.status = Status::Queued;
        t3.note = Some("Waiting for confirmation to start".to_string());

        let gated = std::collections::HashSet::new();
        let next_result = next(vec![t1, t2.clone(), t3], &gated);
        assert_eq!(next_result.task, Some(t2));
    }

    #[test]
    fn a_task_that_could_not_start_is_not_asked_about_either() {
        let mut t1 = sample();
        t1.id = "1".to_string();
        t1.status = Status::Queued;
        t1.auto_start = false;
        t1.note = Some(format!("{COULD_NOT_START} fatal: bad base"));
        let mut t2 = sample();
        t2.id = "2".to_string();
        t2.status = Status::Queued;

        let gated = std::collections::HashSet::new();
        let next_result = next(vec![t1, t2.clone()], &gated);
        assert_eq!(next_result.task, Some(t2));
        assert!(next_result.needs_dispatch_gate.is_empty());
    }

    #[test]
    fn the_whole_queue_is_searched_for_tasks_that_need_a_gate() {
        let mut t1 = sample();
        t1.id = "1".to_string();
        t1.status = Status::Queued;
        t1.auto_start = true;
        let mut t2 = sample();
        t2.id = "2".to_string();
        t2.status = Status::Queued;
        t2.auto_start = false;

        let gated = std::collections::HashSet::new();
        let next_result = next(vec![t1.clone(), t2.clone()], &gated);
        assert_eq!(next_result.task, Some(t1));
        assert_eq!(next_result.needs_dispatch_gate, vec![t2]);
    }

    #[test]
    fn nothing_is_next_when_every_queued_task_is_held() {
        let mut t1 = sample();
        t1.id = "1".to_string();
        t1.status = Status::Queued;
        t1.auto_start = false;
        let mut t2 = sample();
        t2.id = "2".to_string();
        t2.status = Status::Queued;
        t2.note = Some(format!("{COULD_NOT_START} fatal: bad base"));

        let gated = std::collections::HashSet::new();
        let next_result = next(vec![t1.clone(), t2], &gated);
        assert_eq!(next_result.task, None);
        assert_eq!(next_result.needs_dispatch_gate, vec![t1]);
    }

    #[test]
    fn id_carries_a_readable_tail_when_the_title_has_one() {
        assert_eq!(
            new_id("20260922T041233Z", "Fix the login retry"),
            "20260922T041233Z-fix-the-login-retry"
        );
    }

    /// A Japanese title leaves nothing to slugify, and the stamp alone is still a usable
    /// id. The alternative — refusing the task — would reject the common case here.
    #[test]
    fn id_is_the_stamp_alone_when_nothing_survives_slugging() {
        assert_eq!(
            new_id("20260922T041233Z", "ログインのリトライ"),
            "20260922T041233Z"
        );
    }

    #[test]
    fn slug_does_not_run_past_its_cap_or_end_on_a_separator() {
        let slug = slug("a very long english title that keeps going and going and going");
        assert!(slug.len() <= 32, "{slug}");
        assert!(!slug.ends_with('-'), "{slug}");
    }

    /// Two tasks written in the same second, with titles that slug to nothing, must not
    /// land on the same file — which is what filling the form twice in a row looks like.
    #[test]
    fn ids_claimed_in_the_same_second_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<String> = (0..3)
            .map(|_| claim_id(dir.path(), "20260922T041233Z", "ログインのリトライ").unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "20260922T041233Z",
                "20260922T041233Z-2",
                "20260922T041233Z-3"
            ]
        );
    }

    /// The claim leaves the file behind, so `save` has somewhere to land and no second
    /// caller can take the name in between.
    #[test]
    fn a_claimed_id_is_held_before_anything_is_written_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let id = claim_id(dir.path(), "20260922T041233Z", "x").unwrap();
        assert!(path_of(dir.path(), &id).exists());
    }

    #[test]
    fn a_saved_task_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let task = sample();
        save(dir.path(), &task).unwrap();
        assert_eq!(load(dir.path(), &task.id).unwrap(), task);
    }

    #[test]
    fn listing_is_in_queue_order() {
        let dir = tempfile::tempdir().unwrap();
        for (id, order) in [("a", 3u32), ("b", 1), ("c", 2)] {
            let mut task = sample();
            task.id = id.to_string();
            task.order = order;
            save(dir.path(), &task).unwrap();
        }
        let ids: Vec<String> = list(dir.path()).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["b", "c", "a"]);
    }

    /// One unreadable file must not blank the board.
    #[test]
    fn a_file_that_will_not_parse_is_skipped_rather_than_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let task = sample();
        save(dir.path(), &task).unwrap();
        std::fs::write(dir.path().join("broken.json"), "{ not json").unwrap();
        assert_eq!(list(dir.path()).len(), 1);
    }

    #[test]
    fn listing_a_directory_that_is_not_there_is_empty_rather_than_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(&dir.path().join("nope")).is_empty());
    }

    /// The message the hub reads has to carry everything the form asked for; otherwise the
    /// hub goes back to asking the questions the form existed to answer.
    #[test]
    fn the_request_body_carries_what_the_form_collected() {
        let mut task = sample();
        task.base = Some("origin/release/1.2".to_string());
        task.worktree_name = Some("login-retry".to_string());
        task.auto_start = false;
        let body = render_request(&task);
        assert!(body.contains(&task.id), "{body}");
        assert!(body.contains("origin/release/1.2"), "{body}");
        assert!(body.contains("login-retry"), "{body}");
        assert!(body.contains("ask before starting"), "{body}");
        assert!(
            body.contains("investigation only (report and stop)"),
            "{body}"
        );
        assert!(
            body.contains("The retry does not seem to take effect"),
            "{body}"
        );
    }

    #[test]
    fn a_title_not_read_yet_is_said_in_the_request() {
        let mut task = sample();
        assert!(!render_request(&task).contains("## Title"));
        task.title_pending = true;
        assert!(render_request(&task).contains("## Title         not read yet"));
    }

    /// Absent fields are written as `-` rather than left out: the hub reads this as prose,
    /// and a missing line reads as "nobody said" while an empty one reads as "said nothing".
    #[test]
    fn unset_fields_say_so_rather_than_vanishing() {
        let body = render_request(&sample());
        assert!(body.contains("## Base          -"), "{body}");
        assert!(body.contains("## Parent task   -"), "{body}");
    }

    /// The hub copies the stop point into the brief, so the message has to say it — and say
    /// the default out loud rather than leave the line off.
    #[test]
    fn the_request_body_says_where_the_task_stops() {
        let mut task = sample();
        assert!(
            render_request(&task).contains("## Stop at       plan ("),
            "{}",
            render_request(&task)
        );
        task.stop_at = StopAt::All;
        assert!(
            render_request(&task).contains("## Stop at       all ("),
            "{}",
            render_request(&task)
        );
    }

    /// The brief and the request say it in these words, and the worker matches on them.
    #[test]
    fn done_when_prose_is_what_the_request_says() {
        let mut task = sample();
        for done_when in [
            DoneWhen::ReportOnly,
            DoneWhen::Verify,
            DoneWhen::Pr,
            DoneWhen::Review,
        ] {
            task.done_when = done_when;
            let line = format!("## Done when     {}\n", done_when.as_prose());
            assert!(render_request(&task).contains(&line));
        }
        assert_eq!(DoneWhen::Pr.as_prose(), "up to a PR");
    }

    /// A record written before the field existed stopped at the plan, and still does.
    #[test]
    fn a_record_without_a_stop_point_stops_at_the_plan() {
        let mut value = serde_json::to_value(sample()).unwrap();
        value.as_object_mut().unwrap().remove("stopAt");
        let task: Task = serde_json::from_value(value).unwrap();
        assert_eq!(task.stop_at, StopAt::Plan);
    }

    #[test]
    fn the_request_body_includes_instruction_when_present() {
        let mut task = sample();
        assert!(!render_request(&task).contains("## Handover note"));

        task.instruction = Some("Look into how the existing code behaves first".to_string());
        let body = render_request(&task);
        assert!(
            body.contains("## Handover note\n\nLook into how the existing code behaves first\n")
        );
    }

    #[test]
    fn a_record_without_an_instruction_deserializes_with_none() {
        let value = serde_json::to_value(sample()).unwrap();
        let task: Task = serde_json::from_value(value).unwrap();
        assert_eq!(task.instruction, None);
    }

    fn at(owner: &str, repo: &str, number: u64) -> PrRef {
        PrRef {
            host: "github.com".to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
            number,
        }
    }

    #[test]
    fn a_pr_url_is_read_whatever_follows_the_number() {
        for url in [
            "https://github.com/acme/widget/pull/12",
            "https://github.com/acme/widget/pull/12/",
            "https://github.com/acme/widget/pull/12/files",
            "https://github.com/acme/widget/pull/12?diff=split",
            "https://github.com/acme/widget/pull/12#issuecomment-1",
            "https://GitHub.com/Acme/Widget/pull/12",
        ] {
            assert_eq!(pr_ref(url, None), Some(at("acme", "widget", 12)), "{url}");
        }
        let enterprise = pr_ref("https://git.example.com/acme/widget/pull/3", None).unwrap();
        assert_eq!(enterprise.host, "git.example.com");
        assert_eq!(enterprise.nwo(), "acme/widget");
    }

    #[test]
    fn a_bare_number_needs_the_repository_to_be_known() {
        let here = Some(("github.com", "acme/widget"));
        assert_eq!(pr_ref("12", here), Some(at("acme", "widget", 12)));
        assert_eq!(pr_ref("#12", here), Some(at("acme", "widget", 12)));
        assert_eq!(pr_ref("12", None), None);
        assert_eq!(pr_ref("#12", None), None);
        // A directory name is not a repository.
        assert_eq!(pr_ref("12", Some(("github.com", "widget"))), None);
        // It is read on the host the repository's origin is on, not on github.com.
        let enterprise = pr_ref("12", Some(("git.example.com", "acme/widget"))).unwrap();
        assert_eq!(enterprise.host, "git.example.com");
    }

    #[test]
    fn what_is_not_a_pull_request_is_not_guessed_at() {
        for odd in [
            "",
            "-x",
            "--web",
            "-1",
            "https://github.com/acme/widget/issues/12",
            "https://github.com/acme/widget/pull/",
            "https://github.com/acme/widget/pull/x",
            "https://github.com/acme/pull/12",
            "https://github.com/-acme/widget/pull/12",
            "ftp://github.com/acme/widget/pull/12",
            "https://github.com/acme/widget?x=/pull/12",
            "twelve",
        ] {
            assert_eq!(
                pr_ref(odd, Some(("github.com", "acme/widget"))),
                None,
                "{odd:?}"
            );
        }
    }

    #[test]
    fn a_notification_names_its_pull_request_by_an_api_url() {
        assert_eq!(
            pr_ref_from_api("https://api.github.com/repos/acme/widget/pulls/7"),
            Some(at("acme", "widget", 7))
        );
        let enterprise =
            pr_ref_from_api("https://git.example.com/api/v3/repos/acme/widget/pulls/7").unwrap();
        assert_eq!(
            (enterprise.host.as_str(), enterprise.number),
            ("git.example.com", 7)
        );
        for odd in [
            "https://api.github.com/repos/acme/widget/issues/7",
            "https://api.github.com/repos/acme/widget/pulls/x",
            "https://git.example.com/repos/acme/widget/pulls/7",
            "https://github.com/acme/widget/pull/7",
            "",
        ] {
            assert_eq!(pr_ref_from_api(odd), None, "{odd:?}");
        }
    }

    fn pr(state: &str, review: &str, fail: u32, pending: u32) -> PrStatus {
        PrStatus {
            state: state.to_string(),
            title: String::new(),
            review: review.to_string(),
            ci: CheckCounts {
                pass: 1,
                fail,
                pending,
            },
        }
    }

    #[test]
    fn a_turn_is_read_from_the_state_the_review_and_the_checks() {
        let turn = |s: PrStatus| pr_turn(&s);
        assert_eq!(turn(pr("draft", "none", 0, 0)), Some(PrTurn::Draft));
        assert_eq!(turn(pr("open", "none", 0, 0)), Some(PrTurn::Unrequested));
        assert_eq!(
            turn(pr("open", "required", 0, 0)),
            Some(PrTurn::OtherReviewer)
        );
        assert_eq!(turn(pr("open", "required", 0, 2)), Some(PrTurn::Checks));
        assert_eq!(turn(pr("open", "changes", 0, 0)), Some(PrTurn::Changes));
        assert_eq!(turn(pr("open", "approved", 0, 0)), Some(PrTurn::Merge));
        assert_eq!(turn(pr("open", "none", 1, 0)), Some(PrTurn::CiFailed));
        assert_eq!(turn(pr("merged", "approved", 0, 0)), Some(PrTurn::Merged));
        assert_eq!(turn(pr("closed", "none", 0, 0)), Some(PrTurn::Closed));
        assert_eq!(turn(pr("something-new", "none", 0, 0)), None);
    }

    #[test]
    fn what_the_person_can_act_on_comes_first() {
        let turn = |s: PrStatus| pr_turn(&s);
        assert_eq!(turn(pr("open", "changes", 0, 3)), Some(PrTurn::Changes));
        assert_eq!(turn(pr("open", "changes", 1, 0)), Some(PrTurn::Changes));
        assert_eq!(turn(pr("open", "approved", 0, 3)), Some(PrTurn::Checks));
        assert_eq!(turn(pr("open", "approved", 1, 3)), Some(PrTurn::CiFailed));
        // A draft is not asked about its reviews or its checks.
        assert_eq!(turn(pr("draft", "approved", 1, 1)), Some(PrTurn::Draft));
    }

    #[test]
    fn a_turn_serializes_as_a_word() {
        assert_eq!(
            serde_json::to_value(PrTurn::OtherReviewer).unwrap(),
            serde_json::json!("other-reviewer")
        );
        assert_eq!(
            serde_json::to_value(PrTurn::CiFailed).unwrap(),
            serde_json::json!("ci-failed")
        );
    }

    #[test]
    fn a_pr_waits_on_a_person_by_its_turn_and_then_by_the_worker_phase() {
        let waits = |status: &PrStatus, phase: Option<Option<&str>>| {
            pr_waits_on_person(Status::Pr, true, Some(status), phase)
        };
        let changes = pr("open", "changes", 0, 0);
        let approved = pr("open", "approved", 0, 0);
        let failed = pr("open", "none", 1, 0);
        let closed = pr("closed", "none", 0, 0);
        for person in [&changes, &approved, &failed, &closed] {
            for phase in [None, Some(Some("pr")), Some(Some("pr-bots")), Some(None)] {
                // A worker with no phase is not handing over anything.
                let expected = phase != Some(None);
                assert_eq!(waits(person, phase), expected, "{person:?} {phase:?}");
            }
            // The worker is still at work: it stays on the agent board.
            assert!(!waits(person, Some(Some("review"))), "{person:?}");
            assert!(!waits(person, Some(Some("implement"))), "{person:?}");
        }
        for other in [
            pr("open", "required", 0, 0),
            pr("open", "none", 0, 2),
            pr("merged", "approved", 0, 0),
        ] {
            for phase in [None, Some(Some("pr")), Some(Some("pr-bots"))] {
                assert!(!waits(&other, phase), "{other:?} {phase:?}");
            }
        }
        // A draft, or a PR nobody was asked to review, is decided as it was before.
        for undecided in [pr("draft", "none", 0, 0), pr("open", "none", 0, 0)] {
            assert!(waits(&undecided, Some(Some("pr"))));
            assert!(!waits(&undecided, Some(Some("pr-bots"))));
            assert!(waits(&undecided, None));
        }
        assert!(pr_waits_on_person(Status::Pr, true, None, Some(Some("pr"))));
        assert!(!pr_waits_on_person(
            Status::Pr,
            true,
            None,
            Some(Some("pr-bots"))
        ));
        assert!(!pr_waits_on_person(Status::Dispatched, true, None, None));
    }

    #[test]
    fn a_jules_pr_follows_the_turn_and_waits_when_the_turn_says_nothing() {
        for waits in [
            pr("open", "changes", 0, 0),
            pr("open", "approved", 0, 0),
            pr("open", "none", 1, 0),
            pr("closed", "none", 0, 0),
            pr("draft", "none", 0, 0),
            pr("open", "none", 0, 0),
        ] {
            assert!(jules_pr_waits_on_person(Some(&waits)), "{waits:?}");
        }
        for off in [
            pr("open", "required", 0, 0),
            pr("open", "none", 0, 2),
            pr("merged", "approved", 0, 0),
        ] {
            assert!(!jules_pr_waits_on_person(Some(&off)), "{off:?}");
        }
        assert!(jules_pr_waits_on_person(None));
    }

    #[test]
    fn nothing_waits_on_a_person_without_a_pr_or_after_the_task_ended() {
        let closed = pr("closed", "none", 0, 0);
        assert!(!pr_waits_on_person(Status::Pr, false, Some(&closed), None));
        for status in [Status::Done, Status::Cancelled] {
            assert!(!pr_waits_on_person(status, true, Some(&closed), None));
        }
    }
}
