//! `adj review-engine` — which engine reads the diff in this self-review round.
//!
//! Every round used to redo the same reading and arithmetic by hand. This command
//! runs it the same way every time and leaves the procedures.

use serde_json::{Value, json};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::kernel::config;

/// The 5-hour window trips at or above this.
const FIVE_HOUR_LIMIT: f64 = 50.0;
/// The 7-day window trips above this.
const SEVEN_DAY_LIMIT: f64 = 70.0;
/// A cache older than this says nothing about now.
const STALE_AFTER_SECS: i64 = 15 * 60;
/// Written by the status line on every draw, per account, directly under the config directory.
const CACHE_FILE: &str = "rate-limit-cache.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    Claude,
    Codex,
}

impl Engine {
    fn as_str(self) -> &'static str {
        match self {
            Engine::Claude => "claude",
            Engine::Codex => "codex",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Engine::Claude => "Claude",
            Engine::Codex => "codex",
        }
    }
}

/// One window that tripped.
#[derive(Debug, Clone, PartialEq)]
struct Window {
    /// "5h" or "7d"
    name: &'static str,
    used: f64,
    resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
enum Reason {
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
    fn code(&self) -> &'static str {
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
enum Usage {
    Missing,
    Broken,
    Read(Value),
}

/// `${CLAUDE_CONFIG_DIR:-$HOME/.claude}/rate-limit-cache.json`. Takes both inputs as arguments so a
/// test answers for values it chose rather than for the machine it runs on.
fn cache_path(claude_config_dir: Option<&OsStr>, home: &Path) -> PathBuf {
    let mut dir = claude_config_dir
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    dir.push(CACHE_FILE);
    dir
}

/// NotFound -> Missing; any other read error, invalid JSON -> Broken; otherwise Read.
fn read_cache(path: &Path) -> Usage {
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
/// Err only for a `reviewEngine` value that is not auto / claude / codex.
fn decide(
    setting: &str,
    usage: &Usage,
    now: i64,
    codex_on_path: impl FnOnce() -> bool,
) -> Result<(Engine, Reason), String> {
    match setting {
        "claude" => Ok((Engine::Claude, Reason::Pinned)),
        "codex" => Ok((Engine::Codex, Reason::Pinned)),
        "auto" => match usage {
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
        other => Err(format!(
            "reviewEngine is {other:?}; it takes \"auto\", \"claude\" or \"codex\""
        )),
    }
}

/// The one line the worker tells the user.
fn message(engine: Engine, reason: &Reason, setting: &str, cache: &Path, now: i64) -> String {
    let cache_disp = cache.display();
    match reason {
        Reason::Pinned => format!(
            "reviewEngine is {setting}, so {} reviews this round",
            engine.display_name()
        ),
        Reason::Tripped(w) => {
            if let Some(r) = w.resets_at {
                format!(
                    "{} is at {}%, so the review switches to codex (resets in {})",
                    w.name,
                    w.used,
                    super::ago((r - now).max(0))
                )
            } else {
                format!(
                    "{} is at {}%, so the review switches to codex",
                    w.name, w.used
                )
            }
        }
        Reason::CodexMissing(w) => format!(
            "{} is at {}%, but codex is not on PATH, so Claude reviews this round",
            w.name, w.used
        ),
        Reason::WithinLimits {
            five_hour,
            seven_day,
        } => {
            let format_pct = |v: Option<f64>| {
                v.map(|f| format!("{}%", f))
                    .unwrap_or_else(|| "unknown".to_string())
            };
            format!(
                "5h at {}, 7d at {}, so Claude reviews this round",
                format_pct(*five_hour),
                format_pct(*seven_day)
            )
        }
        Reason::CacheMissing => format!(
            "The usage check was skipped ({} does not exist), so Claude reviews this round",
            cache_disp
        ),
        Reason::CacheBroken => format!(
            "The usage check was skipped ({} cannot be read as a rate-limit cache), so Claude reviews this round",
            cache_disp
        ),
        Reason::CacheStale { age_secs } => format!(
            "The usage check was skipped ({} is {} old), so Claude reviews this round",
            cache_disp,
            super::ago(*age_secs)
        ),
        Reason::CacheFuture { ahead_secs } => format!(
            "The usage check was skipped ({} is stamped {} in the future), so Claude reviews this round",
            cache_disp,
            super::ago(*ahead_secs)
        ),
        Reason::NoUsage => format!(
            "The usage check was skipped ({} has neither five_hour nor seven_day), so Claude reviews this round",
            cache_disp
        ),
    }
}

/// `adj review-engine`.
pub fn run(repo_arg: Option<&str>, as_json: bool) -> Result<(), String> {
    let ctx = super::context_without_hub(repo_arg)?;
    let setting = match ctx
        .resolved
        .config
        .as_ref()
        .and_then(|c| c.get("reviewEngine"))
    {
        Some(Value::String(s)) => s.clone(),
        Some(other) => return Err(format!("reviewEngine must be a string, not {other}")),
        None => config::builtin_defaults()["reviewEngine"]
            .as_str()
            .unwrap()
            .to_string(),
    };

    let cache = cache_path(
        std::env::var_os("CLAUDE_CONFIG_DIR").as_deref(),
        &crate::infra::paths::home_dir(),
    );
    let usage = read_cache(&cache);
    let now = crate::infra::clock::now_secs();
    let (engine, reason) = decide(&setting, &usage, now, || {
        crate::infra::shell::on_path("codex")
    })?;
    let text = message(engine, &reason, &setting, &cache, now);

    if as_json {
        let (window, used_percentage, resets_at) = match &reason {
            Reason::Tripped(w) | Reason::CodexMissing(w) => {
                (Some(w.name), Some(w.used), w.resets_at)
            }
            _ => (None, None, None),
        };

        let out = json!({
            "engine": engine.as_str(),
            "setting": setting,
            "reason": reason.code(),
            "window": window,
            "usedPercentage": used_percentage,
            "resetsAt": resets_at,
            "cache": cache.to_string_lossy(),
            "message": text
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("{text}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
