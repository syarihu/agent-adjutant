//! The description of an agent session and its execution context.
//!
//! A session represents an agent process running either as a hub or as a worker in a
//! worktree. It captures who is running (agent, kind, id), where it runs (terminal backend
//! and tmux details), its task and git context, and its lifecycle status.
//!
//! A leaf: defines pure data structures with serde serialization and no internal crate
//! dependencies.

use serde::{Deserialize, Serialize};

/// Where the session runs: its terminal backend, and for tmux the socket, session and window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTerminal {
    /// Terminal backend name: "tmux", "iterm2" (the built-in one), or "custom" for a
    /// `terminal.spawn` template.
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

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
    pub slug: String,
    pub state: RepoHubState,
    pub inbox_count: usize,
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
            self.instruction.trim_end_matches('\n')
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
