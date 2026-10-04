//! What a task is: the record, its vocabulary, and the pure rules over it.

use super::*;

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

impl Kind {
    /// The text a caller gives, refused in the words a bad value in a task's JSON gets.
    pub fn parse(text: &str) -> Result<Kind, String> {
        serde_json::from_value(serde_json::Value::String(text.to_string()))
            .map_err(|e| format!("bad task: {e}"))
    }
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
    /// The text a caller gives, refused in the words a bad value in a task's JSON gets.
    pub fn parse(text: &str) -> Result<DoneWhen, String> {
        serde_json::from_value(serde_json::Value::String(text.to_string()))
            .map_err(|e| format!("bad task: {e}"))
    }

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
    /// The text a caller gives, refused with the words the stop points are listed in. A blank
    /// one is the default, which is the caller's to say.
    pub fn parse(text: &str) -> Result<StopAt, String> {
        Ok(match text {
            "plan" => StopAt::Plan,
            "diff" => StopAt::Diff,
            "all" => StopAt::All,
            _ => return Err(format!("no such stop point: {text} (plan, diff or all)")),
        })
    }

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
    /// version and saved by this one loses nothing. Only ever filled from disk: `create` never fills it.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// What a caller may say about a task it creates. Every other field of the record is the
/// hub's: a caller that could set `relayed` or `julesBy` would change what Jules is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTask {
    /// As given; `create` trims it and falls back on the body or the issue.
    pub title: Option<String>,
    /// As given, and stored as given.
    pub body: String,
    pub kind: Kind,
    pub done_when: DoneWhen,
    pub stop_at: StopAt,
    pub executor: Executor,
    pub issue_url: Option<String>,
    pub base: Option<String>,
    pub parent: Option<String>,
    pub worktree_name: Option<String>,
    pub worktree: Option<String>,
    pub auto_start: bool,
    pub status: Status,
}

impl Default for NewTask {
    /// What a form that filled in nothing else means.
    fn default() -> Self {
        NewTask {
            title: None,
            body: String::new(),
            kind: Kind::Start,
            done_when: DoneWhen::Pr,
            stop_at: StopAt::default(),
            executor: Executor::default(),
            issue_url: None,
            base: None,
            parent: None,
            worktree_name: None,
            worktree: None,
            auto_start: true,
            status: Status::Backlog,
        }
    }
}

impl NewTask {
    /// Read a request's JSON. Keys this does not name are ignored, whatever they are. A value
    /// of the wrong type is refused here (a title aside), before anything is claimed or read
    /// from `gh`.
    pub fn from_json(input: &serde_json::Value) -> Result<NewTask, String> {
        use serde_json::Value;
        let fields = input.as_object().ok_or("expected an object")?;
        // A missing key is the default; one that is there is read as the record would read it,
        // so `null` where a string is needed is refused rather than taken for "missing".
        fn read<T: serde::de::DeserializeOwned>(
            fields: &serde_json::Map<String, Value>,
            key: &str,
        ) -> Result<Option<T>, String> {
            fields
                .get(key)
                .map(|v| serde_json::from_value(v.clone()).map_err(|e| format!("bad task: {e}")))
                .transpose()
        }
        // A form sends the stop point whether or not one was picked: nothing picked is the
        // default.
        let stop_at = match fields.get("stopAt") {
            None | Some(Value::Null) => StopAt::default(),
            Some(Value::String(s)) if s.is_empty() => StopAt::default(),
            Some(Value::String(s)) => StopAt::parse(s)?,
            Some(other) => return Err(format!("no such stop point: {other}")),
        };
        let executor = match fields.get("executor") {
            Some(Value::String(s)) if !s.is_empty() => Executor::parse(s)
                .ok_or_else(|| format!("no such executor: {s} (worker or jules)"))?,
            _ => read::<Executor>(fields, "executor")?.unwrap_or_default(),
        };
        let defaults = NewTask::default();
        Ok(NewTask {
            // The one exception to the types above: a title that is not a string has always
            // been passed over for one derived from the body rather than refused.
            title: fields
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string),
            body: read::<String>(fields, "body")?.unwrap_or_default(),
            kind: read(fields, "kind")?.unwrap_or(defaults.kind),
            done_when: read(fields, "doneWhen")?.unwrap_or(defaults.done_when),
            stop_at,
            executor,
            issue_url: read::<Option<String>>(fields, "issueUrl")?.flatten(),
            base: read::<Option<String>>(fields, "base")?.flatten(),
            parent: read::<Option<String>>(fields, "parent")?.flatten(),
            worktree_name: read::<Option<String>>(fields, "worktreeName")?.flatten(),
            worktree: read::<Option<String>>(fields, "worktree")?.flatten(),
            auto_start: read(fields, "autoStart")?.unwrap_or(defaults.auto_start),
            status: read(fields, "status")?.unwrap_or(defaults.status),
        })
    }
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

/// What GitHub says about a pull request a record points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    /// Closed without being merged. The work may have gone on in another PR, so this is not
    /// read as "done" or as "cancelled": a person says which.
    Closed,
    Merged,
    /// `gh` could not say: not installed, not signed in, no such PR, or an answer this does
    /// not recognise. Why, in `gh`'s own words where it gave any.
    Unreadable(String),
}

impl PrState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrState::Open => "open",
            PrState::Closed => "closed",
            PrState::Merged => "merged",
            PrState::Unreadable(_) => "unreadable",
        }
    }
}
