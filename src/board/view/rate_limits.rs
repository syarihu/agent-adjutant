//! The rate limits of each account the agents run under, from the agent session ledger.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::registry::AgentSession;
use crate::task::{LedgerUsage, newest_usage};

/// The rate limit windows of the accounts the ledger has a figure for. Every poll carries it,
/// with no `accounts` when no session has said one, and `error` when the ledger could not be
/// listed: not the same as no figure.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitsState {
    pub accounts: Vec<AccountRateLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One account — an agent and the config directory it runs under — as its newest session with a
/// figure last drew it. `lastEventAt` is when that session was last heard from, which is not
/// when the figures were drawn, so the page judges how old they may be.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRateLimits {
    pub agent: String,
    pub config_dir: String,
    pub session_id: String,
    pub last_event_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<RateLimitWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<RateLimitWindow>,
}

/// One window of an account: the share used and when it resets, epoch seconds.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitWindow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

fn window(w: &crate::registry::RateWindow) -> RateLimitWindow {
    RateLimitWindow {
        used_percent: w.used_percent,
        resets_at: w.resets_at,
    }
}

/// The accounts of `rows`, by config directory, each from the row `newest_usage` picks for it,
/// ordered by directory. Only Claude draws these figures today, so no other agent has an entry.
pub fn rate_limits_of(rows: Result<Vec<AgentSession>, String>) -> RateLimitsState {
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            return RateLimitsState {
                accounts: Vec::new(),
                error: Some(error),
            };
        }
    };
    let mut dirs: Vec<&str> = Vec::new();
    for row in &rows {
        let Some(dir) = row.config_dir.as_deref() else {
            continue;
        };
        if row.agent.as_deref() == Some("claude")
            && !dirs
                .iter()
                .any(|seen| PathBuf::from(seen) == Path::new(dir))
        {
            dirs.push(dir);
        }
    }
    dirs.sort_unstable();
    let accounts = dirs
        .into_iter()
        .filter_map(|dir| {
            let LedgerUsage {
                session_id,
                last_event_at,
                five_hour,
                seven_day,
            } = newest_usage(&rows, Path::new(dir))?;
            Some(AccountRateLimits {
                agent: "claude".to_string(),
                config_dir: dir.to_string(),
                session_id,
                last_event_at,
                five_hour: five_hour.as_ref().map(window),
                seven_day: seven_day.as_ref().map(window),
            })
        })
        .collect();
    RateLimitsState {
        accounts,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{RateLimits, RateWindow};

    fn row(
        id: &str,
        agent: &str,
        dir: &str,
        last_event_at: i64,
        five: Option<f64>,
    ) -> AgentSession {
        AgentSession {
            session_id: id.to_string(),
            agent: Some(agent.to_string()),
            config_dir: Some(dir.to_string()),
            last_event_at: Some(last_event_at),
            rate_limits: five.map(|used| RateLimits {
                five_hour: Some(RateWindow {
                    used_percent: Some(used),
                    resets_at: Some(900),
                    ..RateWindow::default()
                }),
                ..RateLimits::default()
            }),
            ..AgentSession::default()
        }
    }

    #[test]
    fn two_sessions_of_one_account_make_one_entry_from_the_newer_row_with_a_figure() {
        let state = rate_limits_of(Ok(vec![
            row("old", "claude", "/cfg/a", 100, Some(10.0)),
            row("new", "claude", "/cfg/a", 200, Some(30.0)),
            // Newest, but it has drawn no figure yet.
            row("bare", "claude", "/cfg/a", 300, None),
        ]));
        assert_eq!(state.error, None);
        assert_eq!(state.accounts.len(), 1);
        let account = &state.accounts[0];
        assert_eq!(
            (account.agent.as_str(), account.session_id.as_str()),
            ("claude", "new")
        );
        assert_eq!(account.config_dir, "/cfg/a");
        assert_eq!(account.last_event_at, 200);
        assert_eq!(
            account.five_hour,
            Some(RateLimitWindow {
                used_percent: Some(30.0),
                resets_at: Some(900)
            })
        );
        assert_eq!(account.seven_day, None);
    }

    #[test]
    fn two_accounts_make_two_entries_ordered_by_directory() {
        let state = rate_limits_of(Ok(vec![
            row("b", "claude", "/cfg/b", 100, Some(1.0)),
            row("a", "claude", "/cfg/a", 100, Some(2.0)),
        ]));
        let dirs: Vec<&str> = state
            .accounts
            .iter()
            .map(|a| a.config_dir.as_str())
            .collect();
        assert_eq!(dirs, ["/cfg/a", "/cfg/b"]);
    }

    #[test]
    fn a_ledger_that_cannot_be_listed_is_an_error_not_an_empty_list() {
        let state = rate_limits_of(Err("agent-sessions: denied".to_string()));
        assert!(state.accounts.is_empty());
        assert_eq!(state.error.as_deref(), Some("agent-sessions: denied"));
        let json = serde_json::to_value(rate_limits_of(Ok(Vec::new()))).unwrap();
        assert_eq!(json, serde_json::json!({"accounts": []}));
    }

    #[test]
    fn a_row_of_another_agent_is_not_an_account() {
        let state = rate_limits_of(Ok(vec![row("x", "codex", "/cfg/a", 100, Some(5.0))]));
        assert!(state.accounts.is_empty());
    }
}
