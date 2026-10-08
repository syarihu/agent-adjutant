//! The description of an agent session and its execution context.
//!
//! A session represents an agent process running either as a hub or as a worker in a
//! worktree. It captures who is running (agent, kind, id), where it runs (terminal backend
//! and tmux details), its task and git context, and its lifecycle status.
//!
//! Pure data structures with serde serialization, and the two board-wide refusals to resume
//! a session or a hub, which read only the settings. It also holds `is_window_id` and
//! `target_of`, which name the tmux window a session is opened in.

use serde::{Deserialize, Serialize};

use crate::infra::terminal::SessionTerminal;
use crate::kernel::runner;
use crate::lifecycle::hub::{hub_startable, own_hub_runner_refusal};
use crate::lifecycle::resume_template;

/// An agent session running under adjutant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// Names a slot on one board, unique within that board's state and derived when it is
    /// read, never stored: "hub" for the repo hub, "hub-{key}" for a parent-task hub, and
    /// "worker-{worktree_name}" for a worker — with a short digest of the path added when
    /// another worktree has the same name. Across boards it is addressed as
    /// "{board slug}/{id}".
    pub id: String,
    /// "hub" or "worker". A session with no task is a worker whose `task` is null, not a kind
    /// of its own: linking it to a task changes that field and nothing else about it.
    pub kind: String,
    /// Agent binary / harness name (e.g. "claude", "agy"), derived from the runner template.
    pub agent: String,
    /// Where the session runs.
    pub terminal: SessionTerminal,
    /// The hub this session belongs to (e.g. "hub", "hub-ALPHA-233"). None for hubs themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub: Option<String>,
    /// The parent-task key for a parent-task hub (None for the repository hub and workers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Worktree or checkout directory where the session runs.
    pub worktree: String,
    /// Git branch of the worktree or checkout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The task record ID when one is linked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Session title or task title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The title of the linked task's record, when `task` names one this board can read. Apart
    /// from `title`, which is the tab's own and may say anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_title: Option<String>,
    /// The agent's own session id (its conversation) saved for this session, when there is
    /// one: what a resume would reopen. Not the `id` above, which names a slot on the board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
    /// Process is alive and matches recorded identity.
    pub present: bool,
    /// Recorded process is no longer running.
    pub stale: bool,
    /// PID of the session process, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// UTC timestamp string when the process started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Current worker phase (e.g. "plan", "implement", "verify"), if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Epoch timestamp when the phase was entered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_at: Option<i64>,
    /// Every phase the worker has entered, oldest first, as `[phase, epoch seconds]`: what
    /// `phase` and `phaseAt` were before they were overwritten. Capped by the record.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<(String, i64)>,
    /// Epoch seconds of the last activity in the session's tmux window (`#{window_activity}`).
    /// Absent when the session is not in tmux or the window is not found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<i64>,
    /// The last line the session's pane shows above its input box, as the agent drew it. Only
    /// when the page asked for it (`GET /api/state?lines=1`), and only for a session that runs
    /// in a tmux window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_line: Option<String>,
    /// How many clients are attached to the session's window, not counting the board's own
    /// `adjboard-*` sessions. Absent when the window is not found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached: Option<u32>,
    /// The open gate this session is waiting on, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<SessionWaiting>,
    /// What the agent's hooks last said about this session, from the agent session ledger.
    /// Only for a session that runs; `error` alone when the ledger could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<SessionAgentState>,
    /// The files and lines the worktree holds that no commit has, as last read in the
    /// background. Only for a worker, and only once it has been read: a count of none is
    /// `files: 0`, not a missing key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncommitted: Option<crate::kernel::worktree_state::Uncommitted>,
    /// Why the last read of `uncommitted` failed; the counts of an earlier read stay beside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncommitted_error: Option<String>,
    /// The pull request of the session's branch, for a session that has no task (a task's own
    /// is on its card). Only where the resident server polls, and as old as its last round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_pr: Option<SessionPr>,
    /// Why the last lookup of `branchPr` failed (GitHub could not be asked, or the repository
    /// could not be read); the pull request of an earlier lookup stays beside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_pr_error: Option<String>,
}

/// The pull request a branch has, as a session row shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPr {
    pub number: u64,
    pub url: String,
    /// `open`, `draft`, `merged` or `closed`, as a card's PR status says it.
    pub state: String,
}

/// What the agent's hooks last said about a session: its row in the agent session ledger, as
/// much of it as the page shows. Times are epoch seconds, never ages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAgentState {
    /// The ledger row's own session id, which is not the session's `conversation` after `/clear`.
    /// With `updatedAt` it names one wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The ledger's own word: idle, running, waiting, done, failed, or one a newer binary wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// A `done` or `failed` held back while sub-agents run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<String>,
    /// When the status last changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// When the hooks were last heard from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<i64>,
    /// The tool the agent is running, first line, cut short.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// What the agent asks permission for, first line, cut short.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    /// The model the status line last showed, as its display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How much of the context window is in use, a whole number of percent from 0 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_percent: Option<u8>,
    /// The sub-agents that are running, oldest first.
    #[serde(default)]
    pub subagents: Vec<SessionSubagent>,
    /// Set instead of the rest when the ledger could not be read: not the same as no row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A sub-agent of a session that is running, as the ledger row has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSubagent {
    pub id: String,
    /// What the agent calls it (`Explore`, `general-purpose`).
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// When it started, epoch seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    /// The tool it is running, first line, cut short.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
}

/// A session that has waited on a permission prompt or a question long enough to be listed, for
/// as long as it still waits. The page lists it in 要対応, rings its own desktop notification
/// from it unless `quiet`, once per `(agentSessionId, since)`, and opens `session` when that is
/// clicked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitNotice {
    /// The ledger row's session id, which with `since` names the wait.
    pub agent_session_id: String,
    /// When the row turned `waiting`, epoch seconds.
    pub since: i64,
    /// The board's id of the session that waits (`hub`, `worker-x`), which opens its terminal.
    pub session: String,
    /// `hub` or `worker`.
    pub kind: String,
    /// What the person knows it by: its title, else its id.
    pub name: String,
    /// What it asks, as the ledger has it. Left out when the agent said nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    /// Listed for 要対応 but not announced: the person was at its terminal, or it was already
    /// waiting when the watch started. The page does not ring a desktop notification for it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quiet: bool,
}

/// The oldest open gate a session waits on: the one a worker opened and is waiting to have
/// answered, or, for a hub, the one it opened for a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionWaiting {
    pub id: String,
    pub kind: String,
    /// The `hubs[].id` whose gate directory holds it.
    pub hub: String,
    pub slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// UTC timestamp string when the gate was opened.
    pub opened_at: String,
    /// How many gates the session waits on in all, this one included.
    pub count: usize,
    /// The decisions the gate offers, so a banner can draw the buttons without the gate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<WaitingChoice>,
    /// What the person has to decide, cut short: the banner is not the gate's review page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

/// One choice of a gate, as much of it as fits on a button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingChoice {
    pub id: String,
    pub label: String,
}

/// What the board asks a hub for when a person starts a session without a task: the session
/// has no record to carry these, so the message body is all there is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRequest {
    pub agent: String,
    pub worktree_name: String,
    pub instruction: String,
}

impl SessionRequest {
    /// The body of the `session` message, in the plain `## ` lines `task::render_request`
    /// uses so the hub reads both the same way. The instruction comes last and verbatim: it is
    /// free text, and anything after it would read as part of it.
    pub fn render_request(&self) -> String {
        format!(
            "## Session       no task\n## Agent         {}\n## Worktree name {}\n## Instruction\n{}\n",
            self.agent,
            self.worktree_name,
            // `-` for none, as the other lines of a request say it: the hub starts the worker
            // with nothing to do and the worker waits for the person.
            match self.instruction.trim_end_matches('\n') {
                "" => "-",
                text => text,
            }
        )
    }
}

// ── resume ───────────────────────────────────────────────────────────

/// Why no session can be resumed from the board with these settings, or `None` when one can.
/// The board-wide half of `resume`'s refusals, known without looking at a session.
pub fn resume_refusal(settings: &crate::kernel::config::Settings) -> Option<String> {
    if !hub_startable(&settings.terminal) {
        return Some(
            "resuming a session from the board needs terminal.preset \"tmux\" and no terminal.spawn"
                .to_string(),
        );
    }
    // Only the built-in resume line knows how to reopen a Claude conversation. Another agent
    // given it would start something unrelated in the worktree and look like a resumed worker.
    let agent = runner::agent_from_runner(
        settings
            .agent_runner
            .as_deref()
            .unwrap_or(runner::DEFAULT_AGENT_RUNNER),
    );
    if settings.agent_resume_runner.is_none() && agent != "claude" {
        return Some(format!(
            "{agent} has no agentResumeRunner, so it cannot be resumed"
        ));
    }
    // The same refusal the resume itself would end in, so the page never offers a button that
    // can only fail.
    if let Err(refusal) =
        resume_template(settings.agent_resume_runner.as_deref(), "agentResumeRunner")
    {
        return Some(refusal);
    }
    None
}

/// Why no hub can be resumed from the board with these settings, or `None` when one can: what
/// `hub_startable` and `hubResumeRunner` say, known without looking at a hub. The per-hub half
/// (a saved conversation, a parent key that is known) is checked by the restart itself.
pub fn hub_resume_refusal(settings: &crate::kernel::config::Settings) -> Option<String> {
    if !hub_startable(&settings.terminal) {
        return Some(
            "restarting a hub from the board needs terminal.preset \"tmux\" and no terminal.spawn"
                .to_string(),
        );
    }
    if let Err(refusal) = resume_template(settings.hub_resume_runner.as_deref(), "hubResumeRunner")
    {
        return Some(refusal);
    }
    own_hub_runner_refusal(settings)
}

/// A tmux window id: `@` and digits, the only thing a record's `window` is allowed to be.
pub fn is_window_id(window: &str) -> bool {
    window
        .strip_prefix('@')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The socket and window of `session`, if it is one a terminal can be opened on: it runs in
/// tmux, was recorded with a window, and is running. Shared with the board's action that opens
/// the session in the person's own terminal.
pub fn target_of(session: &crate::board::Session) -> Option<(Option<String>, String)> {
    let terminal = &session.terminal;
    let window = terminal.window.as_deref().filter(|w| is_window_id(w))?;
    (terminal.backend == "tmux" && session.present)
        .then(|| (terminal.socket.clone(), window.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_says_what_it_knows_in_camel_case_and_leaves_out_what_it_does_not() {
        let mut session = Session {
            id: "worker-a".to_string(),
            kind: "worker".to_string(),
            agent: "claude".to_string(),
            terminal: SessionTerminal {
                backend: "tmux".to_string(),
                socket: None,
                session: None,
                window: None,
                pane: None,
            },
            hub: None,
            key: None,
            worktree: "/w".to_string(),
            branch: None,
            task: None,
            title: None,
            task_title: None,
            conversation: None,
            present: true,
            stale: false,
            pid: None,
            started_at: None,
            phase: None,
            phase_at: None,
            phases: Vec::new(),
            last_activity_at: None,
            last_line: None,
            attached: None,
            waiting: None,
            agent_session: None,
            uncommitted: None,
            uncommitted_error: None,
            branch_pr: None,
            branch_pr_error: None,
        };
        let bare = serde_json::to_value(&session).unwrap();
        for key in [
            "phases",
            "lastActivityAt",
            "lastLine",
            "attached",
            "waiting",
            "agentSession",
            "uncommitted",
            "uncommittedError",
            "branchPr",
            "branchPrError",
        ] {
            assert!(bare.get(key).is_none(), "{key} should be left out");
        }
        session.phases = vec![("plan".to_string(), 123), ("verify".to_string(), 456)];
        session.last_activity_at = Some(99);
        session.last_line = Some("Running tests".to_string());
        session.attached = Some(0);
        session.waiting = Some(SessionWaiting {
            id: "g1".to_string(),
            kind: "plan".to_string(),
            hub: "hub".to_string(),
            slug: "o-r".to_string(),
            title: None,
            opened_at: "20260101T000000Z".to_string(),
            count: 2,
            options: vec!["answer".to_string()],
            choices: vec![WaitingChoice {
                id: "a".to_string(),
                label: "A".to_string(),
            }],
            focus: None,
        });
        session.agent_session = Some(SessionAgentState {
            session_id: Some("agent-1".to_string()),
            status: Some("waiting".to_string()),
            pending: None,
            updated_at: Some(7),
            last_event_at: Some(8),
            activity: None,
            request: Some("Bash: ls".to_string()),
            model: Some("Opus 5".to_string()),
            context_percent: Some(43),
            subagents: vec![
                SessionSubagent {
                    id: "a1".to_string(),
                    kind: Some("Explore".to_string()),
                    started_at: Some(5),
                    activity: Some("Grep: x".to_string()),
                },
                SessionSubagent {
                    id: "a2".to_string(),
                    kind: None,
                    started_at: None,
                    activity: None,
                },
            ],
            error: None,
        });
        session.uncommitted = Some(crate::kernel::worktree_state::Uncommitted {
            files: 2,
            untracked: 1,
            insertions: 10,
            deletions: 3,
            binary: 1,
        });
        session.uncommitted_error = Some("git could not read HEAD".to_string());
        session.branch_pr = Some(SessionPr {
            number: 12,
            url: "https://github.com/o/r/pull/12".to_string(),
            state: "draft".to_string(),
        });
        let full = serde_json::to_value(&session).unwrap();
        assert_eq!(full["agentSession"]["sessionId"], "agent-1");
        assert_eq!(full["agentSession"]["status"], "waiting");
        assert_eq!(full["agentSession"]["updatedAt"], 7);
        assert_eq!(full["agentSession"]["lastEventAt"], 8);
        assert_eq!(full["agentSession"]["request"], "Bash: ls");
        assert_eq!(full["agentSession"]["model"], "Opus 5");
        assert_eq!(full["agentSession"]["contextPercent"], 43);
        assert_eq!(
            full["agentSession"]["subagents"],
            serde_json::json!([
                {"id": "a1", "type": "Explore", "startedAt": 5, "activity": "Grep: x"},
                {"id": "a2"}
            ])
        );
        assert_eq!(full["uncommitted"]["files"], 2);
        assert_eq!(full["uncommitted"]["binary"], 1);
        assert_eq!(full["uncommittedError"], "git could not read HEAD");
        assert_eq!(full["branchPr"]["number"], 12);
        assert_eq!(full["branchPr"]["state"], "draft");
        session.branch_pr_error = Some("API rate limit exceeded".to_string());
        let errored = serde_json::to_value(&session).unwrap();
        assert_eq!(errored["branchPrError"], "API rate limit exceeded");
        assert!(full["agentSession"].get("pending").is_none());
        assert!(full["agentSession"].get("error").is_none());
        assert_eq!(
            full["phases"],
            serde_json::json!([["plan", 123], ["verify", 456]])
        );
        assert_eq!(full["lastActivityAt"], 99);
        assert_eq!(full["lastLine"], "Running tests");
        assert_eq!(full["attached"], 0);
        assert_eq!(full["waiting"]["openedAt"], "20260101T000000Z");
        assert!(full["waiting"].get("title").is_none());
        assert!(full["waiting"].get("focus").is_none());
        assert_eq!(full["waiting"]["options"], serde_json::json!(["answer"]));
        assert_eq!(full["waiting"]["choices"][0]["label"], "A");
    }

    #[test]
    fn a_session_request_renders_its_fields_and_the_instruction_verbatim() {
        let request = SessionRequest {
            agent: "claude".to_string(),
            worktree_name: "try-retry".to_string(),
            instruction: "## not a header\n  keep 'quotes' and $vars\n".to_string(),
        };
        assert_eq!(
            request.render_request(),
            "## Session       no task\n## Agent         claude\n## Worktree name try-retry\n\
             ## Instruction\n## not a header\n  keep 'quotes' and $vars\n"
        );
        let none = SessionRequest {
            instruction: String::new(),
            ..request
        };
        assert!(none.render_request().ends_with("## Instruction\n-\n"));
    }
}
