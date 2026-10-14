use std::path::Path;

use serde::Serialize;

use crate::board::SessionAgentState;
use crate::board::view::agent_state_of;
use crate::infra::clock::now_secs;
use crate::kernel::config::resolve_config;
use crate::kernel::identity::AIDE_CONFIG_KEY;
use crate::lifecycle::hub::hub_startable;
use crate::registry::{
    self, AgentIdentity, AideRecord, AideState, Liveness, ProcessTable, Recorded, aide_state_with,
};

/// What `GET /api/aide` answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AideView {
    /// `none`, `starting`, `running`, `stopped` or `unknown`.
    pub state: &'static str,
    /// Only for `starting` and `running`: a stopped one has nothing to attach to.
    pub session_id: Option<String>,
    pub launched_at: Option<i64>,
    /// The ledger row of the agent, for context use and what it is doing.
    pub agent_session: Option<SessionAgentState>,
    /// Whether opening it from here can work: the terminal is tmux and this server has a tmux.
    pub startable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Where the aide is now. Writes nothing: a stopped aide is reported as stopped and the next
/// open replaces its record. `tmux` is the version this server found, `None` without one.
pub fn find(root: &Path, tmux: Option<(u32, u32)>) -> AideView {
    let startable = tmux.is_some()
        && resolve_config(AIDE_CONFIG_KEY)
            .map(|resolved| hub_startable(&resolved.settings.terminal))
            .unwrap_or(false);
    find_with(
        root,
        startable,
        now_secs(),
        &registry::hub_process_liveness,
        &ProcessTable::each(),
    )
}

/// `find`, with `startable`, the clock, the process check and the process table handed in.
pub(crate) fn find_with(
    root: &Path,
    startable: bool,
    now: i64,
    liveness: &dyn Fn(u32, Option<&str>) -> Liveness,
    table: &ProcessTable,
) -> AideView {
    let mut view = AideView {
        state: "none",
        session_id: None,
        launched_at: None,
        agent_session: None,
        startable,
        error: None,
    };
    let record = match registry::aide_record(root) {
        Recorded::Absent => return view,
        Recorded::Unreadable => {
            view.state = "unknown";
            view.error = Some("the aide record cannot be read".to_string());
            return view;
        }
        Recorded::Found(record) => record,
    };
    view.launched_at = Some(record.launched_at);
    view.state = match aide_state_with(&record, now, liveness) {
        AideState::Starting => "starting",
        AideState::Running => "running",
        AideState::Gone => "stopped",
        AideState::CannotTell => {
            view.error = Some("cannot tell whether the aide is still running".to_string());
            "unknown"
        }
    };
    if matches!(view.state, "starting" | "running") {
        view.session_id = Some(record.session_id.clone());
        view.agent_session = aide_row(root, table, &record);
    }
    view
}

/// The ledger row of the aide's agent as the board carries it, found by the session and the
/// process the record names.
pub(crate) fn aide_row(
    root: &Path,
    table: &ProcessTable,
    record: &AideRecord,
) -> Option<SessionAgentState> {
    let identity = AgentIdentity {
        session_id: Some(&record.session_id),
        pid: record.pid,
        ps_started: record.ps_started.as_deref(),
    };
    registry::agent_session_of(root, table, &identity)
        .ok()
        .flatten()
        .map(|row| agent_state_of(&row))
}

/// Whether a turn is in progress: the agent is running, or its end is held back while
/// sub-agents run.
///
/// A `running` row is believed only while the hooks were heard from recently: Claude Code sends
/// no `Stop` when a turn is interrupted with Esc. The bound mirrors `RUNNING_TRUSTED_SECS` in
/// `mail/hold.rs`, which is private to `mail`.
pub(crate) fn is_busy(agent: &SessionAgentState, now: i64) -> bool {
    const RUNNING_TRUSTED_SECS: i64 = 600;
    let running = agent.status.as_deref() == Some("running")
        && agent
            .last_event_at
            .is_some_and(|seen| now - seen < RUNNING_TRUSTED_SECS);
    running || agent.pending.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, text: &str) {
        std::fs::write(root.join("aide.json"), text).unwrap();
    }

    fn state(root: &Path, liveness: &dyn Fn(u32, Option<&str>) -> Liveness) -> AideView {
        find_with(
            root,
            true,
            1000,
            liveness,
            &ProcessTable::fixed(Some(Default::default())),
        )
    }

    #[test]
    fn each_record_state_is_named_and_only_a_live_one_carries_its_session() {
        let dir = tempfile::tempdir().unwrap();
        let alive = |_: u32, _: Option<&str>| Liveness::Alive;
        let gone = |_: u32, _: Option<&str>| Liveness::Gone;
        let unsure = |_: u32, _: Option<&str>| Liveness::CannotTell;
        let none = state(dir.path(), &alive);
        assert_eq!((none.state, none.session_id), ("none", None));
        assert!(none.startable);

        put(dir.path(), r#"{"sessionId":"s1","launchedAt":990}"#);
        let starting = state(dir.path(), &gone);
        assert_eq!(starting.state, "starting");
        assert_eq!(starting.session_id.as_deref(), Some("s1"));
        assert_eq!(starting.launched_at, Some(990));

        put(dir.path(), r#"{"sessionId":"s1","launchedAt":1,"pid":9}"#);
        assert_eq!(state(dir.path(), &alive).state, "running");
        assert_eq!(state(dir.path(), &alive).session_id.as_deref(), Some("s1"));
        let stopped = state(dir.path(), &gone);
        assert_eq!((stopped.state, stopped.session_id), ("stopped", None));
        let unknown = state(dir.path(), &unsure);
        assert_eq!(unknown.state, "unknown");
        assert!(unknown.error.is_some());

        put(dir.path(), "{");
        assert_eq!(state(dir.path(), &alive).state, "unknown");
    }

    #[test]
    fn a_turn_in_progress_is_busy_and_a_held_back_end_is_too() {
        let mut agent: SessionAgentState = serde_json::from_str("{}").unwrap();
        assert!(!is_busy(&agent, 1000));
        agent.status = Some("running".to_string());
        agent.last_event_at = Some(990);
        assert!(is_busy(&agent, 1000));
        agent.status = Some("idle".to_string());
        agent.pending = Some("done".to_string());
        assert!(is_busy(&agent, 1000));
    }

    #[test]
    fn a_running_row_that_has_gone_quiet_is_not_believed() {
        let mut agent: SessionAgentState = serde_json::from_str("{}").unwrap();
        agent.status = Some("running".to_string());
        agent.last_event_at = Some(100);
        assert!(!is_busy(&agent, 100 + 600));
        agent.last_event_at = None;
        assert!(!is_busy(&agent, 1000));
    }
}
