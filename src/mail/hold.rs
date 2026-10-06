//! Holding a wake back by what the agent's session row says, where the screen cannot be read.
use std::time::Duration;

use crate::infra::terminal::{Performed, WAKE_READY_BUDGET, WAKE_READY_POLL};
use crate::registry::{AgentSession, AgentStatus};

/// A `running` row older than this is not believed. Claude Code sends no `Stop` when a turn is
/// interrupted with Esc, so a row can stay `running` long after its agent is back at the prompt.
const RUNNING_TRUSTED_SECS: i64 = 600;

/// Why a wake should not be typed now, by what `row` says its agent is doing: `None` when it is
/// safe, or when the row cannot be trusted to say so.
pub(crate) fn busy(row: &AgentSession, now: i64) -> Option<&'static str> {
    match row.status {
        Some(AgentStatus::Running)
            if row
                .last_event_at
                .is_some_and(|seen| now - seen < RUNNING_TRUSTED_SECS) =>
        {
            Some("its agent is in the middle of a turn")
        }
        Some(AgentStatus::Waiting) => {
            Some("its agent is waiting on a question or a permission prompt")
        }
        _ => None,
    }
}

/// Wait for the agent's row to stop saying it is busy, reading it every so often, within the
/// time a wake is given to wait for a prompt. `Ok` as soon as there is no row or it is not
/// busy; otherwise the wake as it is reported when it is held back.
pub(crate) fn wait_for_row(
    read: impl Fn() -> Option<AgentSession>,
    wait: impl Fn(Duration),
    now: impl Fn() -> i64,
    pid: u32,
) -> Result<(), Performed> {
    let mut waited = Duration::ZERO;
    loop {
        let Some(why) = read().and_then(|row| busy(&row, now())) else {
            return Ok(());
        };
        if waited >= WAKE_READY_BUDGET {
            return Err(Performed {
                description: format!("the wake was not typed into the session (pid {pid}): {why}"),
                script: String::new(),
                ran: false,
                screen: true,
            });
        }
        wait(WAKE_READY_POLL);
        waited += WAKE_READY_POLL;
    }
}
