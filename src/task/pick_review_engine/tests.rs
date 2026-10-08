use super::*;
use crate::kernel::config::ReviewEngine;
use crate::registry::{AgentSession, RateLimits};

use serde_json::json;

pub(crate) const NOW: i64 = 1_800_000_000;

#[test]
fn a_pinned_engine_does_not_read_the_cache() {
    let (engine, reason) = decide(&ReviewEngine::Claude, &Usage::Missing, NOW, || {
        panic!("PATH was searched although nothing tripped")
    })
    .unwrap();
    assert_eq!(engine, Engine::Claude);
    assert_eq!(reason, Reason::Pinned);

    let (engine, reason) = decide(&ReviewEngine::Codex, &Usage::Missing, NOW, || {
        panic!("PATH was searched although nothing tripped")
    })
    .unwrap();
    assert_eq!(engine, Engine::Codex);
    assert_eq!(reason, Reason::Pinned);
}

#[test]
fn an_unknown_setting_is_an_error() {
    let err = decide(
        &ReviewEngine::Other("\"sometimes\"".into()),
        &Usage::Missing,
        NOW,
        || false,
    )
    .unwrap_err();
    assert_eq!(
        err,
        "reviewEngine is \"sometimes\"; it takes \"auto\", \"claude\" or \"codex\""
    );
}

#[test]
fn five_hour_trips_at_fifty() {
    let usage = Usage::Read(json!({
        "captured_at": NOW,
        "five_hour": { "used_percentage": 50.0 }
    }));
    let (engine, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || true).unwrap();
    assert_eq!(engine, Engine::Codex);
    assert_eq!(
        reason,
        Reason::Tripped(Window {
            name: "5h",
            used: 50.0,
            resets_at: None
        })
    );

    let usage = Usage::Read(json!({
        "captured_at": NOW,
        "five_hour": { "used_percentage": 49.9 }
    }));
    let (engine, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || {
        panic!("PATH was searched although nothing tripped")
    })
    .unwrap();
    assert_eq!(engine, Engine::Claude);
    assert_eq!(
        reason,
        Reason::WithinLimits {
            five_hour: Some(49.9),
            seven_day: None
        }
    );
}

#[test]
fn seven_day_trips_only_above_seventy() {
    let usage = Usage::Read(json!({
        "captured_at": NOW,
        "seven_day": { "used_percentage": 70.0 }
    }));
    let (engine, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || {
        panic!("PATH was searched although nothing tripped")
    })
    .unwrap();
    assert_eq!(engine, Engine::Claude);
    assert_eq!(
        reason,
        Reason::WithinLimits {
            five_hour: None,
            seven_day: Some(70.0)
        }
    );

    let usage = Usage::Read(json!({
        "captured_at": NOW,
        "seven_day": { "used_percentage": 70.1 }
    }));
    let (engine, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || true).unwrap();
    assert_eq!(engine, Engine::Codex);
    assert_eq!(
        reason,
        Reason::Tripped(Window {
            name: "7d",
            used: 70.1,
            resets_at: None
        })
    );
}

#[test]
fn when_both_trip_the_five_hour_window_is_named_with_its_own_reset() {
    let usage = Usage::Read(json!({
        "captured_at": NOW,
        "five_hour": { "used_percentage": 60.0, "resets_at": 100 },
        "seven_day": { "used_percentage": 90.0, "resets_at": 200 }
    }));
    let (_, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || true).unwrap();
    assert_eq!(
        reason,
        Reason::Tripped(Window {
            name: "5h",
            used: 60.0,
            resets_at: Some(100)
        })
    );
}

#[test]
fn each_window_is_judged_on_its_own() {
    let usage = Usage::Read(json!({
        "captured_at": NOW,
        "seven_day": { "used_percentage": 80.0 }
    }));
    let (_, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || true).unwrap();
    assert_eq!(
        reason,
        Reason::Tripped(Window {
            name: "7d",
            used: 80.0,
            resets_at: None
        })
    );

    let usage = Usage::Read(json!({
        "captured_at": NOW,
        "five_hour": { "used_percentage": 10.0 }
    }));
    let (_, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || false).unwrap();
    assert_eq!(
        reason,
        Reason::WithinLimits {
            five_hour: Some(10.0),
            seven_day: None
        }
    );
}

#[test]
fn a_cache_with_neither_window_skips_the_check() {
    let usage = Usage::Read(json!({ "captured_at": NOW }));
    let (engine, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || {
        panic!("PATH was searched although nothing tripped")
    })
    .unwrap();
    assert_eq!(engine, Engine::Claude);
    assert_eq!(reason, Reason::NoUsage);
}

#[test]
fn a_cache_older_than_fifteen_minutes_skips_the_check() {
    let usage =
        Usage::Read(json!({ "captured_at": NOW - 901, "five_hour": { "used_percentage": 90.0 } }));
    let (_, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || false).unwrap();
    assert_eq!(reason, Reason::CacheStale { age_secs: 901 });

    let usage =
        Usage::Read(json!({ "captured_at": NOW - 900, "five_hour": { "used_percentage": 90.0 } }));
    let (_, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || true).unwrap();
    assert_eq!(
        reason,
        Reason::Tripped(Window {
            name: "5h",
            used: 90.0,
            resets_at: None
        })
    );
}

#[test]
fn a_cache_without_captured_at_or_not_an_object_is_broken() {
    let usage = Usage::Read(json!({ "five_hour": { "used_percentage": 90.0 } }));
    let (_, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || false).unwrap();
    assert_eq!(reason, Reason::CacheBroken);

    let usage = Usage::Read(json!([1, 2]));
    let (_, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || false).unwrap();
    assert_eq!(reason, Reason::CacheBroken);
}

#[test]
fn a_tripped_window_without_codex_stays_with_claude() {
    let usage =
        Usage::Read(json!({ "captured_at": NOW, "five_hour": { "used_percentage": 62.0 } }));
    let (engine, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || false).unwrap();
    assert_eq!(engine, Engine::Claude);
    assert_eq!(
        reason,
        Reason::CodexMissing(Window {
            name: "5h",
            used: 62.0,
            resets_at: None
        })
    );
}

#[test]
fn the_cache_is_read_as_missing_broken_or_read() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("missing.json");
    assert!(matches!(read_cache(&path), Usage::Missing));

    let path = tmp.path().join("broken.json");
    std::fs::write(&path, "not json").unwrap();
    assert!(matches!(read_cache(&path), Usage::Broken));

    let path = tmp.path().join("valid.json");
    std::fs::write(&path, r#"{"captured_at": 1}"#).unwrap();
    assert!(matches!(read_cache(&path), Usage::Read(_)));
}

#[test]
fn the_cache_sits_under_claude_config_dir_or_home() {
    let home = Path::new("/home");
    assert_eq!(
        cache_path(Some(OsStr::new("/cfg/a")), home),
        PathBuf::from("/cfg/a/rate-limit-cache.json")
    );
    assert_eq!(
        cache_path(Some(OsStr::new("")), home),
        PathBuf::from("/home/.claude/rate-limit-cache.json")
    );
    assert_eq!(
        cache_path(None, home),
        PathBuf::from("/home/.claude/rate-limit-cache.json")
    );
}

#[test]
fn a_blank_claude_config_dir_is_the_default_account() {
    let home = Path::new("/home");
    assert_eq!(
        claude_config_dir(Some(OsStr::new("  ")), home),
        PathBuf::from("/home/.claude")
    );
    assert_eq!(
        claude_config_dir(Some(OsStr::new("")), home),
        PathBuf::from("/home/.claude")
    );
    assert_eq!(
        claude_config_dir(Some(OsStr::new("/cfg/a")), home),
        PathBuf::from("/cfg/a")
    );
    assert_eq!(
        cache_path(Some(OsStr::new(" ")), home),
        PathBuf::from("/home/.claude/rate-limit-cache.json")
    );
}

fn window(used: Option<f64>, resets_at: Option<i64>) -> RateWindow {
    RateWindow {
        used_percent: used,
        resets_at,
        ..Default::default()
    }
}

fn row(
    id: &str,
    agent: &str,
    config_dir: &str,
    last_event_at: i64,
    five: Option<RateWindow>,
    seven: Option<RateWindow>,
) -> AgentSession {
    AgentSession {
        session_id: id.to_string(),
        agent: Some(agent.to_string()),
        config_dir: Some(config_dir.to_string()),
        last_event_at: Some(last_event_at),
        rate_limits: Some(RateLimits {
            five_hour: five,
            seven_day: seven,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn claude_row(id: &str, last_event_at: i64, used: f64) -> AgentSession {
    row(
        id,
        "claude",
        "/cfg/a",
        last_event_at,
        Some(window(Some(used), None)),
        None,
    )
}

#[test]
fn the_newest_qualifying_row_of_the_account_is_picked() {
    let dir = Path::new("/cfg/a");
    let mut no_limits = claude_row("no-limits", NOW - 1, 1.0);
    no_limits.rate_limits = None;
    let rows = vec![
        no_limits,
        row(
            "other-dir",
            "claude",
            "/cfg/b",
            NOW - 2,
            Some(window(Some(1.0), None)),
            None,
        ),
        row(
            "codex",
            "codex",
            "/cfg/a",
            NOW - 3,
            Some(window(Some(1.0), None)),
            None,
        ),
        row(
            "no-percent",
            "claude",
            "/cfg/a",
            NOW - 4,
            Some(window(None, Some(NOW))),
            None,
        ),
        claude_row("older", NOW - 600, 20.0),
        claude_row("oldest", NOW - 700, 30.0),
    ];
    let got = ledger_usage(&rows, dir, NOW).unwrap();
    assert_eq!(got.session_id, "older");
    assert_eq!(got.last_event_at, NOW - 600);

    // Not relying on the order the rows come in.
    let rows = vec![
        claude_row("oldest", NOW - 700, 30.0),
        claude_row("newer", NOW - 10, 40.0),
    ];
    assert_eq!(ledger_usage(&rows, dir, NOW).unwrap().session_id, "newer");

    assert_eq!(ledger_usage(&[], dir, NOW), None);
}

#[test]
fn a_config_dir_with_a_trailing_slash_is_the_same_account() {
    let rows = vec![row(
        "s",
        "claude",
        "/cfg/a/",
        NOW,
        Some(window(Some(10.0), None)),
        None,
    )];
    assert!(ledger_usage(&rows, Path::new("/cfg/a"), NOW).is_some());
}

#[test]
fn a_row_is_fresh_for_fifteen_minutes() {
    let dir = Path::new("/cfg/a");
    let at = |last| vec![claude_row("s", last, 10.0)];
    assert!(ledger_usage(&at(NOW - 900), dir, NOW).is_some());
    assert_eq!(ledger_usage(&at(NOW - 901), dir, NOW), None);
    assert_eq!(ledger_usage(&at(NOW + 1), dir, NOW), None);
}

#[test]
fn the_newest_usage_is_returned_whatever_its_age() {
    let dir = Path::new("/cfg/a");
    let rows = vec![claude_row("old", NOW - 1200, 10.0)];
    assert_eq!(newest_usage(&rows, dir).unwrap().session_id, "old");
    assert_eq!(ledger_usage(&rows, dir, NOW), None);
}

fn ledger(five: Option<RateWindow>, seven: Option<RateWindow>) -> Usage {
    Usage::Ledger(LedgerUsage {
        session_id: "s".to_string(),
        last_event_at: NOW,
        five_hour: five,
        seven_day: seven,
    })
}

#[test]
fn ledger_figures_trip_by_the_same_rules_as_the_cache() {
    let usage = ledger(
        Some(window(Some(50.0), Some(NOW + 60))),
        Some(window(Some(10.0), None)),
    );
    let (engine, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || true).unwrap();
    assert_eq!(engine, Engine::Codex);
    assert_eq!(
        reason,
        Reason::Tripped(Window {
            name: "5h",
            used: 50.0,
            resets_at: Some(NOW + 60)
        })
    );

    let usage = ledger(None, Some(window(Some(71.0), Some(NOW + 5))));
    let (engine, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || true).unwrap();
    assert_eq!(engine, Engine::Codex);
    assert_eq!(
        reason,
        Reason::Tripped(Window {
            name: "7d",
            used: 71.0,
            resets_at: Some(NOW + 5)
        })
    );

    let usage = ledger(Some(window(Some(10.0), None)), None);
    let (engine, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || {
        panic!("PATH was searched although nothing tripped")
    })
    .unwrap();
    assert_eq!(engine, Engine::Claude);
    assert_eq!(
        reason,
        Reason::WithinLimits {
            five_hour: Some(10.0),
            seven_day: None
        }
    );

    let usage = ledger(Some(window(Some(60.0), None)), None);
    let (engine, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || false).unwrap();
    assert_eq!(engine, Engine::Claude);
    assert_eq!(reason.code(), "codex-missing");

    let (engine, reason) = decide(&ReviewEngine::Claude, &usage, NOW, || {
        panic!("PATH was searched although the engine is pinned")
    })
    .unwrap();
    assert_eq!(engine, Engine::Claude);
    assert_eq!(reason, Reason::Pinned);
}
