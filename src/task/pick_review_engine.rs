//! Which engine reads the diff in a self-review round, decided from the rate limits of the agent
//! session ledger (`adj hook claude --status-line` fills it) or, failing that, the rate-limit cache.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::kernel::config::ReviewEngine;
use crate::registry::{AgentSession, RateWindow};

/// The 5-hour window trips at or above this.
const FIVE_HOUR_LIMIT: f64 = 50.0;
/// The 7-day window trips above this.
const SEVEN_DAY_LIMIT: f64 = 70.0;
/// A cache older than this says nothing about now.
const STALE_AFTER_SECS: i64 = 15 * 60;
/// The fallback for a user whose status line script writes it on every draw, per account,
/// directly under the config directory.
const CACHE_FILE: &str = "rate-limit-cache.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Claude,
    Codex,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Claude => "claude",
            Engine::Codex => "codex",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Engine::Claude => "Claude",
            Engine::Codex => "codex",
        }
    }
}

/// One window that tripped.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// "5h" or "7d"
    pub name: &'static str,
    pub used: f64,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reason {
    Pinned,
    Tripped(Window),
    CodexMissing(Window),
    WithinLimits {
        five_hour: Option<f64>,
        seven_day: Option<f64>,
    },
    CacheMissing,
    CacheBroken,
    CacheStale {
        age_secs: i64,
    },
    /// `captured_at` is later than now: clock skew or a broken cache, so it says nothing about now.
    CacheFuture {
        ahead_secs: i64,
    },
    NoUsage,
}

impl Reason {
    pub fn code(&self) -> &'static str {
        match self {
            Reason::Pinned => "pinned",
            Reason::Tripped(_) => "tripped",
            Reason::CodexMissing(_) => "codex-missing",
            Reason::WithinLimits { .. } => "within-limits",
            Reason::CacheMissing => "cache-missing",
            Reason::CacheBroken => "cache-broken",
            Reason::CacheStale { .. } => "cache-stale",
            Reason::CacheFuture { .. } => "cache-future",
            Reason::NoUsage => "no-usage",
        }
    }
}

/// The usage as read, not yet judged.
#[derive(Debug)]
pub enum Usage {
    Missing,
    Broken,
    Read(Value),
    /// A session row of the agent session ledger that is fresh enough to say something.
    Ledger(LedgerUsage),
}

/// The rate limit windows of one ledger row, as `ledger_usage` picked it.
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerUsage {
    pub session_id: String,
    pub last_event_at: i64,
    pub five_hour: Option<RateWindow>,
    pub seven_day: Option<RateWindow>,
}

/// `${CLAUDE_CONFIG_DIR:-$HOME/.claude}`: the variable when it says something, else the default
/// account's directory. Takes both inputs as arguments so a test answers for values it chose
/// rather than for the machine it runs on. The rule mirrors the one `adj hook claude` stores a
/// row's `configDir` by, so the two name the same directory for the same account.
pub fn claude_config_dir(var: Option<&OsStr>, home: &Path) -> PathBuf {
    var.and_then(OsStr::to_str)
        .filter(|dir| !dir.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"))
}

/// `rate-limit-cache.json` under `claude_config_dir`.
pub fn cache_path(claude_config_dir_var: Option<&OsStr>, home: &Path) -> PathBuf {
    claude_config_dir(claude_config_dir_var, home).join(CACHE_FILE)
}

/// NotFound -> Missing; any other read error, invalid JSON -> Broken; otherwise Read.
pub fn read_cache(path: &Path) -> Usage {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(v) => Usage::Read(v),
            Err(_) => Usage::Broken,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Usage::Missing,
        Err(_) => Usage::Broken,
    }
}

/// The figures of the Claude row under `config_dir` with the newest `lastEventAt` that has at
/// least one window with a `usedPercent`, however old it is.
///
/// `lastEventAt` also moves for hooks, so it is when the session was last seen alive, not when
/// the figures were drawn; the figures may be older than the age says. The row is picked by
/// `lastEventAt` here, whatever order `rows` come in; of equal times the first wins.
pub fn newest_usage(rows: &[AgentSession], config_dir: &Path) -> Option<LedgerUsage> {
    let mut best: Option<(&AgentSession, i64)> = None;
    for row in rows {
        let Some(at) = row.last_event_at else {
            continue;
        };
        if row.agent.as_deref() != Some("claude")
            || row.config_dir.as_deref().map(Path::new) != Some(config_dir)
        {
            continue;
        }
        let Some(limits) = &row.rate_limits else {
            continue;
        };
        let has_figure = [&limits.five_hour, &limits.seven_day]
            .into_iter()
            .any(|w| w.as_ref().is_some_and(|w| w.used_percent.is_some()));
        if has_figure && best.is_none_or(|(_, newest)| at > newest) {
            best = Some((row, at));
        }
    }
    let (row, at) = best?;
    let limits = row.rate_limits.as_ref()?;
    Some(LedgerUsage {
        session_id: row.session_id.clone(),
        last_event_at: at,
        five_hour: limits.five_hour.clone(),
        seven_day: limits.seven_day.clone(),
    })
}

/// `newest_usage`, if its row is no older than the cache may be.
pub fn ledger_usage(rows: &[AgentSession], config_dir: &Path, now: i64) -> Option<LedgerUsage> {
    let usage = newest_usage(rows, config_dir)?;
    (0..=STALE_AFTER_SECS)
        .contains(&(now - usage.last_event_at))
        .then_some(usage)
}

/// The trip rules, over figures already known to be fresh. The 5-hour window is judged first; a
/// window that is missing is judged alone. `codex_on_path` is only called when a window trips.
fn judge(
    five: Option<(f64, Option<i64>)>,
    seven: Option<(f64, Option<i64>)>,
    codex_on_path: impl FnOnce() -> bool,
) -> (Engine, Reason) {
    if five.is_none() && seven.is_none() {
        return (Engine::Claude, Reason::NoUsage);
    }
    let tripped = match (five, seven) {
        (Some((f, resets_at)), _) if f >= FIVE_HOUR_LIMIT => Some(Window {
            name: "5h",
            used: f,
            resets_at,
        }),
        (_, Some((s, resets_at))) if s > SEVEN_DAY_LIMIT => Some(Window {
            name: "7d",
            used: s,
            resets_at,
        }),
        _ => None,
    };
    match tripped {
        None => (
            Engine::Claude,
            Reason::WithinLimits {
                five_hour: five.map(|(f, _)| f),
                seven_day: seven.map(|(s, _)| s),
            },
        ),
        Some(w) if codex_on_path() => (Engine::Codex, Reason::Tripped(w)),
        Some(w) => (Engine::Claude, Reason::CodexMissing(w)),
    }
}

/// The decision, as a function of its inputs. `codex_on_path` is only called when a window trips.
/// Err only for a `reviewEngine` value that is not auto / claude / codex; the message shows
/// the value as it was configured.
pub fn decide(
    setting: &ReviewEngine,
    usage: &Usage,
    now: i64,
    codex_on_path: impl FnOnce() -> bool,
) -> Result<(Engine, Reason), String> {
    match setting {
        ReviewEngine::Claude => Ok((Engine::Claude, Reason::Pinned)),
        ReviewEngine::Codex => Ok((Engine::Codex, Reason::Pinned)),
        ReviewEngine::Auto => match usage {
            Usage::Missing => Ok((Engine::Claude, Reason::CacheMissing)),
            Usage::Broken => Ok((Engine::Claude, Reason::CacheBroken)),
            // Its age was judged when it was picked.
            Usage::Ledger(l) => {
                let window = |w: &Option<RateWindow>| {
                    w.as_ref()
                        .and_then(|w| w.used_percent.map(|used| (used, w.resets_at)))
                };
                Ok(judge(
                    window(&l.five_hour),
                    window(&l.seven_day),
                    codex_on_path,
                ))
            }
            Usage::Read(v) => {
                if !v.is_object() {
                    return Ok((Engine::Claude, Reason::CacheBroken));
                }
                let captured_at = match v.get("captured_at").and_then(|c| c.as_f64()) {
                    Some(c) => c as i64,
                    None => return Ok((Engine::Claude, Reason::CacheBroken)),
                };
                if captured_at > now {
                    return Ok((
                        Engine::Claude,
                        Reason::CacheFuture {
                            ahead_secs: captured_at - now,
                        },
                    ));
                }
                let age = now - captured_at;
                if age > STALE_AFTER_SECS {
                    return Ok((Engine::Claude, Reason::CacheStale { age_secs: age }));
                }

                let window = |key: &str| {
                    let w = v.get(key)?;
                    let used = w.get("used_percentage").and_then(|u| u.as_f64())?;
                    let resets_at = w
                        .get("resets_at")
                        .and_then(|u| u.as_f64())
                        .map(|u| u as i64);
                    Some((used, resets_at))
                };
                Ok(judge(
                    window("five_hour"),
                    window("seven_day"),
                    codex_on_path,
                ))
            }
        },
        ReviewEngine::Other(text) => Err(format!(
            "reviewEngine is {text}; it takes \"auto\", \"claude\" or \"codex\""
        )),
    }
}

#[cfg(test)]
mod tests;
// The message tests left in src/transport/cli/review_engine/tests.rs read the same clock.
#[cfg(test)]
pub(crate) use tests::NOW;
