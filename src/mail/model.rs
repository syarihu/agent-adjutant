use super::*;
use serde::{Deserialize, Serialize};

// ── messages ─────────────────────────────────────────────────────────
#[derive(Debug, Clone, Default)]
pub struct Message {
    /// Who is speaking. A worker's session name, or whatever the sending agent calls itself.
    pub from: String,
    /// Where it is speaking *from*: the absolute path of the sender's own worktree.
    ///
    /// `from` cannot carry this. It is free text, chosen by the sender, and names a session
    /// at best — so a hub acting on a request to close a tab and remove a worktree was
    /// acting on a path typed into the body. This is derived from where the sender actually
    /// is, and it is filled in for every kind: which worktree a report came from is worth
    /// the same line as which worktree a finished task is in, and a format whose headers
    /// depend on the kind is one every reader eventually mis-parses.
    ///
    /// `None` when the sender could not be placed in a worktree at all, and then the header
    /// is left out rather than written empty.
    pub worktree: Option<String>,
    /// `report`, `question`, `answer`, `ack`, `done`, or anything the two sides agree on.
    pub kind: String,
    /// The one line a human will actually read.
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Delivery {
    pub path: PathBuf,
    pub present: bool,
}

/// Newlines in a header value would let a body forge extra headers, and a colon in a value
/// is harmless but confusing. Collapsing whitespace handles both.
pub(super) fn one_line(text: &str) -> String {
    let collapsed: Vec<&str> = text.split_whitespace().collect();
    collapsed.join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub subject: String,
    pub from: String,
    /// The sender's worktree, for the messages that carry one. `None` covers both "sent
    /// from outside a worktree" and "written before this header existed": an inbox outlives
    /// an upgrade, and a listing that failed on the messages already in it would strand
    /// them.
    pub worktree: Option<String>,
    pub kind: String,
    /// When it was sent: the `at` header, or the stamp its file name starts with for a
    /// message that has none. `None` when neither reads as a stamp.
    pub at: Option<String>,
}

pub fn header_value(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return None;
    }
    for line in lines {
        if line == "---" {
            return None;
        }
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// The prefix a message wears while it is being acked. A dotfile, so `list` does not offer
/// it and `safe_join` will not open it — it is mid-move, not waiting — and it carries the
/// name it came from so that a move interrupted half way can be undone.
pub(super) const HOLDING: &str = ".acking-";

/// How long a message may be held before `list` decides nobody is coming back for it. An
/// ack holds one across two syscalls, so anything this old is from a process that died.
pub(super) const HELD_STALE_SECS: u64 = 60;

/// A message name comes from an agent, so it is untrusted input used as a path. Anything
/// with a separator in it is rejected outright rather than sanitised — a name that needed
/// sanitising was not one of ours.
pub(super) fn safe_join(dir: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return Err(format!("invalid message name: {name}"));
    }
    Ok(dir.join(name))
}

/// `report.md` with `seq` worked into it: `report-2.md`. Suffixing the stem rather than the
/// whole name keeps the extension where a reader (and an editor) expects it.
pub(super) fn numbered(name: &str, seq: usize) -> String {
    if seq == 0 {
        return name.to_string();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}-{seq}.{ext}"),
        _ => format!("{name}-{seq}"),
    }
}

/// One message waiting in a hub's inbox, as hubs[].inbox lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub name: String,
    pub subject: String,
    pub kind: String,
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    /// UTC timestamp string from the message header, or its file name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// A hub of the repository, as reported in hubs[].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoHub {
    pub id: String,
    /// Whether this is a parent-task hub rather than the repository's own one. Said apart from
    /// `key` because a parent-task hub started before its record carried the key can have
    /// none that can be told.
    pub parent: bool,
    /// null for the repository hub, the key string for parent-task hubs, and null for a
    /// parent-task hub whose key cannot be told (`parent` says which).
    pub key: Option<String>,
    pub name: String,
    /// The parent task's title, from the tracker cache; None for the repository hub and until
    /// the title is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub slug: String,
    pub state: RepoHubState,
    pub inbox_count: usize,
    /// The newest messages waiting, newest first and capped: `inbox_count` is the full number.
    #[serde(default)]
    pub inbox: Vec<InboxItem>,
    /// How many checkouts have a worker that reports to this hub, running or ended.
    #[serde(default)]
    pub children: usize,
}

/// The status / state of a hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoHubState {
    pub present: bool,
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
}

fn has_bracketed_tag(first_line: &str, tag: &str) -> bool {
    let lower = first_line.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix(tag) {
        rest.starts_with(' ') || rest.starts_with(':') || rest.starts_with(']')
    } else {
        false
    }
}

/// Whether a message left for a worker requires waking it.
///
/// A worker tab is woken only when there is something it has to act on: a question asked
/// back by the hub, a decision on a gate it opened, or a link to a task. The last is woken
/// because a session that was started with no task is idle at its prompt and would never
/// look at its outbox on its own. Notices (`[ack]`, issue filed, etc.)
/// are left in the outbox for the worker to read the next time it checks; waking on a notice
/// risks typing the wake line into an interactive prompt or question the person is looking at.
pub fn should_wake_worker(subject: &str) -> bool {
    let first_line = subject.lines().next().unwrap_or("").trim();
    has_bracketed_tag(first_line, "[question")
        || first_line.starts_with("[質問")
        || has_bracketed_tag(first_line, "[gate")
        || has_bracketed_tag(first_line, "[linked")
}

/// Whether a message delivered to the hub requires waking it.
///
/// A hub tab is woken for reports, answers, done notices, task requests, next triggers,
/// gate decisions, Jules updates, and any actionable custom message kinds. Messages a hub
/// leaves for itself (`kind: question`, `kind: needs-user`, or any message where `from`
/// is the hub itself), plain notices, and acknowledgements do not wake the hub.
pub fn should_wake_hub(from: &str, hub_name: &str, kind: &str, subject: &str) -> bool {
    if !hub_name.is_empty() && from == hub_name {
        return false;
    }
    let kind = if kind.trim().is_empty() {
        "report"
    } else {
        kind.trim()
    };
    if kind == "question" || kind == "needs-user" || kind == "ack" || kind == "notice" {
        return false;
    }
    let first_line = subject.lines().next().unwrap_or("").trim();
    if has_bracketed_tag(first_line, "[ack") {
        return false;
    }
    true
}

/// What became of a message left for a hub or a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryOutcome {
    pub path: PathBuf,
    pub reached: Reached,
}

impl DeliveryOutcome {
    /// Whether anyone was running to read it.
    pub fn is_present(&self) -> bool {
        !matches!(self.reached, Reached::NotRunning)
    }

    /// Whether the receiver was woken to read it.
    pub fn was_woken(&self) -> bool {
        matches!(self.reached, Reached::Woken)
    }
}

/// Whether the message reached a running receiver, and if so whether it was woken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reached {
    Woken,
    Running { wake: NotWoken },
    NotRunning,
}

/// Why a running receiver was not woken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotWoken {
    /// The message did not call for a wake.
    NotNeeded,
    /// A wake was called for and did not happen. `why` is what the receiver's screen was
    /// showing, or what went wrong; `None` when there was nothing to say about it.
    Held { why: Option<String> },
}

/// The agent a wake will find on the other end, which decides how its screen is read.
///
/// Decided from the receiver's runner alone. `ADJUTANT_AGENT` is the setting of whichever
/// process is sending, and it says nothing about the session being woken: an agy worker
/// sending to a Claude hub would read the hub's screen with agy's table. And stricter than
/// `prompts::resolve_agent`, which falls back to Claude for anything it does not know: a
/// procedure in the wrong dialect is a wording problem, but a screen read with the wrong
/// agent's table is never recognised and the wake would never be typed. So only a runner that
/// is Claude Code or agy is read; any other custom runner is typed into without looking.
pub(crate) fn wake_agent(runner: Option<&str>) -> crate::infra::agent::Agent {
    use crate::infra::agent::Agent;
    let Some(runner) = runner else {
        return Agent::Claude;
    };
    // The program the line runs, past `env` and `KEY=VALUE` words: a runner is often written
    // `env CLAUDE_CONFIG_DIR=… claude --resume {sessionId}`.
    match crate::kernel::runner::agent_from_runner(runner).as_str() {
        "claude" => Agent::Claude,
        "agy" => Agent::Agy,
        _ => Agent::Generic,
    }
}
