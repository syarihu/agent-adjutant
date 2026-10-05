//! `adj review-engine` — which engine reads the diff in this self-review round.
//!
//! Every round used to redo the same reading and arithmetic by hand. This command
//! runs it the same way every time and leaves the procedures.

use serde_json::json;
use std::path::Path;

use super::args::ReviewEngineArgs;
use crate::task::{Engine, Reason, cache_path, decide, read_cache};

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
pub fn run(args: &ReviewEngineArgs) -> Result<(), String> {
    let ctx = crate::registry::context_without_hub(args.repo.as_deref())?;
    let setting = &ctx.settings.review_engine;

    let cache = cache_path(
        std::env::var_os("CLAUDE_CONFIG_DIR").as_deref(),
        &crate::infra::paths::home_dir(),
    );
    let usage = read_cache(&cache);
    let now = crate::infra::clock::now_secs();
    let (engine, reason) = decide(setting, &usage, now, || {
        crate::infra::shell::on_path("codex")
    })?;
    let text = message(engine, &reason, setting.as_str(), &cache, now);

    if args.json {
        let (window, used_percentage, resets_at) = match &reason {
            Reason::Tripped(w) | Reason::CodexMissing(w) => {
                (Some(w.name), Some(w.used), w.resets_at)
            }
            _ => (None, None, None),
        };

        let out = json!({
            "engine": engine.as_str(),
            "setting": setting.as_str(),
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
