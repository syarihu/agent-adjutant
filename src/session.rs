//! The description of an agent session and its execution context.
//!
//! A session represents an agent process running either as a hub or as a worker in a
//! worktree. It captures who is running (agent, kind, id), where it runs (terminal backend
//! and tmux details), its task and git context, and its lifecycle status.
//!
//! Pure data structures with serde serialization. The only thing it names in the crate is
//! `SessionTerminal`, which lives in `infra::terminal` and is re-exported here.

use serde::{Deserialize, Serialize};

pub use crate::infra::terminal::SessionTerminal;

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
        };
        let bare = serde_json::to_value(&session).unwrap();
        for key in [
            "phases",
            "lastActivityAt",
            "lastLine",
            "attached",
            "waiting",
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
        let full = serde_json::to_value(&session).unwrap();
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
