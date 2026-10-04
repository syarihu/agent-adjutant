use super::*;
use crate::kernel::config::ReviewEngine;

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
