//! Which engine reads the diff in a self-review round, decided from the rate-limit cache.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::kernel::config::ReviewEngine;

/// The 5-hour window trips at or above this.
const FIVE_HOUR_LIMIT: f64 = 50.0;
/// The 7-day window trips above this.
const SEVEN_DAY_LIMIT: f64 = 70.0;
/// A cache older than this says nothing about now.
const STALE_AFTER_SECS: i64 = 15 * 60;
/// Written by the status line on every draw, per account, directly under the config directory.
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

/// The cache as read, not yet judged.
#[derive(Debug)]
pub enum Usage {
    Missing,
    Broken,
    Read(Value),
}

/// `${CLAUDE_CONFIG_DIR:-$HOME/.claude}/rate-limit-cache.json`. Takes both inputs as arguments so a
/// test answers for values it chose rather than for the machine it runs on.
pub fn cache_path(claude_config_dir: Option<&OsStr>, home: &Path) -> PathBuf {
    let mut dir = claude_config_dir
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    dir.push(CACHE_FILE);
    dir
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

                let used = |key: &str| {
                    v.get(key)
                        .and_then(|w| w.get("used_percentage"))
                        .and_then(|u| u.as_f64())
                };
                let resets_at = |key: &str| {
                    v.get(key)
                        .and_then(|w| w.get("resets_at"))
                        .and_then(|u| u.as_f64())
                        .map(|u| u as i64)
                };

                let five = used("five_hour");
                let seven = used("seven_day");

                if five.is_none() && seven.is_none() {
                    return Ok((Engine::Claude, Reason::NoUsage));
                }

                let tripped = if let Some(f) = five {
                    if f >= FIVE_HOUR_LIMIT {
                        Some(Window {
                            name: "5h",
                            used: f,
                            resets_at: resets_at("five_hour"),
                        })
                    } else if let Some(s) = seven {
                        if s > SEVEN_DAY_LIMIT {
                            Some(Window {
                                name: "7d",
                                used: s,
                                resets_at: resets_at("seven_day"),
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else if let Some(s) = seven {
                    if s > SEVEN_DAY_LIMIT {
                        Some(Window {
                            name: "7d",
                            used: s,
                            resets_at: resets_at("seven_day"),
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                match tripped {
                    None => Ok((
                        Engine::Claude,
                        Reason::WithinLimits {
                            five_hour: five,
                            seven_day: seven,
                        },
                    )),
                    Some(w) => {
                        if codex_on_path() {
                            Ok((Engine::Codex, Reason::Tripped(w)))
                        } else {
                            Ok((Engine::Claude, Reason::CodexMissing(w)))
                        }
                    }
                }
            }
        },
        ReviewEngine::Other(text) => Err(format!(
            "reviewEngine is {text}; it takes \"auto\", \"claude\" or \"codex\""
        )),
    }
}

#[cfg(test)]
mod tests;
// The message tests left in src/cmd/review_engine/tests.rs read the same clock.
#[cfg(test)]
pub(crate) use tests::NOW;
