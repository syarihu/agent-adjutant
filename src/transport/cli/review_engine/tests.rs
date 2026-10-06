use super::*;
use crate::kernel::config::ReviewEngine;

use crate::task::{NOW, Usage, Window};

#[test]
fn a_cache_stamped_in_the_future_skips_the_check() {
    let usage =
        Usage::Read(json!({ "captured_at": NOW + 1, "five_hour": { "used_percentage": 90.0 } }));
    let (engine, reason) = decide(&ReviewEngine::Auto, &usage, NOW, || {
        panic!("PATH was searched although the cache is not trusted")
    })
    .unwrap();
    assert_eq!(engine, Engine::Claude);
    assert_eq!(reason, Reason::CacheFuture { ahead_secs: 1 });
    assert_eq!(reason.code(), "cache-future");

    let text = message(engine, &reason, "auto", Path::new("/cache"), NOW);
    assert_eq!(
        text,
        "The usage check was skipped (/cache is stamped less than a minute in the future), so Claude reviews this round"
    );

    let usage =
        Usage::Read(json!({ "captured_at": NOW, "five_hour": { "used_percentage": 90.0 } }));
    let (engine, _) = decide(&ReviewEngine::Auto, &usage, NOW, || true).unwrap();
    assert_eq!(engine, Engine::Codex);
}

#[test]
fn the_message_names_the_window_and_when_it_resets() {
    let cache = Path::new("/cache");
    let w = Window {
        name: "5h",
        used: 62.0,
        resets_at: Some(NOW + 3900),
    };
    let text = message(
        Engine::Codex,
        &Reason::Tripped(w.clone()),
        "auto",
        cache,
        NOW,
    );
    assert_eq!(
        text,
        "5h is at 62%, so the review switches to codex (resets in 1 h 5 min)"
    );

    let w_no_reset = Window {
        name: "5h",
        used: 62.0,
        resets_at: None,
    };
    let text = message(
        Engine::Codex,
        &Reason::Tripped(w_no_reset),
        "auto",
        cache,
        NOW,
    );
    assert_eq!(text, "5h is at 62%, so the review switches to codex");

    let text = message(
        Engine::Claude,
        &Reason::CacheStale { age_secs: 900 },
        "auto",
        cache,
        NOW,
    );
    assert!(text.contains("was skipped"));
    assert!(text.contains("/cache"));
}

#[test]
fn a_missing_cache_says_no_session_reported_either() {
    let text = message(
        Engine::Claude,
        &Reason::CacheMissing,
        "auto",
        Path::new("/cache"),
        NOW,
    );
    assert_eq!(
        text,
        "The usage check was skipped (no session reported rate limits in the last 15 minutes and /cache does not exist), so Claude reviews this round"
    );
}
