//! `adj review-engine` — which engine reads the diff in this self-review round.
//!
//! The cache and `codex` are machine-local, so every test sets `CLAUDE_CONFIG_DIR` (or `HOME`)
//! and `PATH` itself.

mod common;

use common::*;
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

fn write_cache(dir: &Path, body: serde_json::Value) {
    std::fs::write(dir.join("rate-limit-cache.json"), body.to_string()).unwrap();
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn with_codex(root: &Path) -> OsString {
    let bin = root.join("stub-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let codex = bin.join("codex");
    std::fs::write(&codex, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut paths = vec![bin];
    if let Some(real_path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&real_path));
    }
    std::env::join_paths(paths).unwrap()
}

fn without_codex(root: &Path) -> OsString {
    let bin = root.join("git-only");
    std::fs::create_dir_all(&bin).unwrap();

    // Find git on real PATH
    let git_path = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join("git"))
        .find(|p| p.exists())
        .expect("git not found on PATH");

    std::os::unix::fs::symlink(git_path, bin.join("git")).unwrap();

    std::env::join_paths([bin]).unwrap()
}

fn review_engine(fixture: &Fixture, config_dir: &Path, path: &OsStr, json: bool) -> Output {
    let mut cmd = fixture.command(["review-engine"]);
    if json {
        cmd.arg("--json");
    }
    cmd.env("CLAUDE_CONFIG_DIR", config_dir)
        .env("PATH", path)
        .output()
        .unwrap()
}

#[test]
fn a_tripped_window_sends_the_review_to_codex() {
    let fixture = Fixture::new(QUIET);
    let tmp = tempfile::tempdir().unwrap();
    let n = now();
    write_cache(
        tmp.path(),
        serde_json::json!({
            "captured_at": n,
            "five_hour": {"used_percentage": 62, "resets_at": n + 3600},
            "seven_day": {"used_percentage": 10}
        }),
    );
    let path = with_codex(tmp.path());

    let out = review_engine(&fixture, tmp.path(), &path, true);
    assert!(out.status.success());
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    assert_eq!(j["engine"], "codex");
    assert_eq!(j["reason"], "tripped");
    assert_eq!(j["window"], "5h");
    assert_eq!(j["resetsAt"], n + 3600);
    assert!(j["message"].as_str().unwrap().contains("5h is at 62%"));
}

#[test]
fn without_codex_on_path_the_review_stays_with_claude() {
    let fixture = Fixture::new(QUIET);
    let tmp = tempfile::tempdir().unwrap();
    let n = now();
    write_cache(
        tmp.path(),
        serde_json::json!({
            "captured_at": n,
            "five_hour": {"used_percentage": 62, "resets_at": n + 3600},
            "seven_day": {"used_percentage": 10}
        }),
    );
    let path = without_codex(tmp.path());

    let out = review_engine(&fixture, tmp.path(), &path, true);
    assert!(out.status.success());
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    assert_eq!(j["engine"], "claude");
    assert_eq!(j["reason"], "codex-missing");
}

#[test]
fn a_missing_cache_skips_the_check() {
    let fixture = Fixture::new(QUIET);
    let tmp = tempfile::tempdir().unwrap();
    let path = with_codex(tmp.path());

    let out = review_engine(&fixture, tmp.path(), &path, true);
    assert!(out.status.success());
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    assert_eq!(j["engine"], "claude");
    assert_eq!(j["reason"], "cache-missing");
    assert!(
        j["cache"]
            .as_str()
            .unwrap()
            .ends_with("rate-limit-cache.json")
    );
    assert!(
        j["cache"]
            .as_str()
            .unwrap()
            .starts_with(tmp.path().to_str().unwrap())
    );
    assert!(j["message"].as_str().unwrap().contains("was skipped"));
}

#[test]
fn a_stale_cache_skips_the_check() {
    let fixture = Fixture::new(QUIET);
    let tmp = tempfile::tempdir().unwrap();
    let n = now();
    write_cache(
        tmp.path(),
        serde_json::json!({
            "captured_at": n - 3600,
            "five_hour": {"used_percentage": 90}
        }),
    );
    let path = with_codex(tmp.path());

    let out = review_engine(&fixture, tmp.path(), &path, true);
    assert!(out.status.success());
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    assert_eq!(j["engine"], "claude");
    assert_eq!(j["reason"], "cache-stale");
}

#[test]
fn without_claude_config_dir_the_cache_is_read_from_home() {
    let fixture = Fixture::new(QUIET);
    let tmp = tempfile::tempdir().unwrap();
    let claude_dir = tmp.path().join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    let n = now();
    write_cache(
        &claude_dir,
        serde_json::json!({
            "captured_at": n,
            "seven_day": {"used_percentage": 80}
        }),
    );
    let path = with_codex(tmp.path());

    let out = fixture
        .command(["review-engine", "--json"])
        .env_remove("CLAUDE_CONFIG_DIR")
        .env("HOME", tmp.path())
        .env("PATH", &path)
        .output()
        .unwrap();

    assert!(out.status.success());
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    assert_eq!(j["engine"], "codex");
    assert_eq!(j["window"], "7d");
}

#[test]
fn a_pinned_engine_ignores_the_cache() {
    let mut config = QUIET.to_string();
    config = config.replace(
        "\"acme/widget\": {",
        "\"acme/widget\": {\n      \"reviewEngine\": \"claude\",",
    );
    let fixture = Fixture::new(&config);
    let tmp = tempfile::tempdir().unwrap();
    let n = now();
    write_cache(
        tmp.path(),
        serde_json::json!({
            "captured_at": n,
            "five_hour": {"used_percentage": 90}
        }),
    );
    let path = with_codex(tmp.path());

    let out = review_engine(&fixture, tmp.path(), &path, true);
    assert!(out.status.success());
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    assert_eq!(j["engine"], "claude");
    assert_eq!(j["reason"], "pinned");
}

#[test]
fn a_bad_review_engine_value_fails() {
    let mut config = QUIET.to_string();
    config = config.replace(
        "\"acme/widget\": {",
        "\"acme/widget\": {\n      \"reviewEngine\": \"sometimes\",",
    );
    let fixture = Fixture::new(&config);
    let tmp = tempfile::tempdir().unwrap();
    let path = with_codex(tmp.path());

    let out = review_engine(&fixture, tmp.path(), &path, true);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("reviewEngine"));
}

#[test]
fn without_json_it_prints_the_message_alone() {
    let fixture = Fixture::new(QUIET);
    let tmp = tempfile::tempdir().unwrap();
    let n = now();
    write_cache(
        tmp.path(),
        serde_json::json!({
            "captured_at": n,
            "five_hour": {"used_percentage": 62, "resets_at": n + 3600},
            "seven_day": {"used_percentage": 10}
        }),
    );
    let path = with_codex(tmp.path());

    let out_json = review_engine(&fixture, tmp.path(), &path, true);
    let j: serde_json::Value = serde_json::from_slice(&out_json.stdout).unwrap();
    let expected_message = j["message"].as_str().unwrap();

    let out_plain = review_engine(&fixture, tmp.path(), &path, false);
    assert!(out_plain.status.success());
    let plain_message = String::from_utf8_lossy(&out_plain.stdout)
        .trim()
        .to_string();

    assert_eq!(plain_message, expected_message);
}
