//! What the config resolves to: the flat shorthand, the dashboard setting at each level, the
//! worktree fallback and the shipped example.

mod common;

use common::*;

/// The machine says collect and this repository says don't.
const DASHBOARD_PER_REPO: &str = r#"{"notification": "true", "startupDashboard": true,
    "defaults": {"ide": "code"},
    "repos": {"acme/widget": {"startupDashboard": false, "taskSource": "github",
              "issueRepo": "acme/widget", "issueKeys": {"acme/widget": "WID"}}}}"#;

/// The same, with nothing said about the repository.
const DASHBOARD_PER_MACHINE: &str = r#"{"notification": "true", "startupDashboard": false,
    "defaults": {"ide": "code"},
    "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
              "issueKeys": {"acme/widget": "WID"}}}}"#;

/// The middle rung: `defaults` says don't, the top level says do, and the entry says nothing.
/// Both neighbours disagree with it, so the answer can only have come from `defaults` itself.
const DASHBOARD_PER_DEFAULTS: &str = r#"{"notification": "true", "startupDashboard": true,
    "defaults": {"ide": "code", "startupDashboard": false},
    "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget",
              "issueKeys": {"acme/widget": "WID"}}}}"#;

#[test]
fn config_resolves_the_flat_shorthand_and_the_machine_settings() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.json(&["config"]);
    assert_eq!(out["registered"], true);
    assert_eq!(out["repo"], "acme/widget");
    assert_eq!(out["config"]["taskSources"][0]["type"], "github");
    assert_eq!(
        out["config"]["taskSources"][0]["worktreeName"],
        "{issuekey-lowercase}-{issue}"
    );
    assert_eq!(out["settings"]["ide"], "code");
    assert_eq!(out["config"]["verify"][0], "cargo test");
    assert_eq!(out["warnings"].as_array().unwrap().len(), 0, "{out}");
}

#[test]
fn an_unregistered_repo_says_so_instead_of_failing() {
    let fixture = Fixture::new(r#"{"repos": {}}"#);
    let out = fixture.json(&["config"]);
    assert_eq!(out["registered"], false);
    assert!(out["config"].is_null());
}

/// Every answer `settings.startupDashboard` can give, read back out of the binary.
///
/// `adj config` is what the hub's procedure actually reads, so this is the only place the
/// whole trip is visible: the file, the level that won it, and the flag that outranks both.
///
/// The unit tests take that decision apart — which level wins, and what the flag does to the
/// winner — and check the halves separately. Nothing checked that the resolver still joins
/// them: hand the deciding function `None` for the configured value instead of the level it
/// picked, and every one of those tests stays green while a repository that turned the
/// collection off gets it anyway.
///
/// Both directions are asserted, because a resolver that ignored the file would be right
/// half the time by accident — `true` is also the default, so a fixture that only ever says
/// `false` would not tell "read it" from "never read it and defaulted".
#[test]
fn what_the_config_file_says_about_the_dashboard_reaches_the_resolved_settings() {
    // A machine that has never heard of the setting collects, which is what every existing
    // config has to keep doing.
    assert_eq!(
        Fixture::new(QUIET).json(&["config"])["settings"]["startupDashboard"],
        true
    );

    // The most specific level wins, as it does for every other machine setting — and the
    // two levels are made to disagree so that the answer names which one was read.
    let per_repo = Fixture::new(DASHBOARD_PER_REPO);
    assert_eq!(
        per_repo.json(&["config"])["settings"]["startupDashboard"],
        false
    );

    // And a machine that says it once, for every repository it holds, is read the same way.
    // A resolver that only ever looked at the entry would pass the case above.
    let per_machine = Fixture::new(DASHBOARD_PER_MACHINE);
    assert_eq!(
        per_machine.json(&["config"])["settings"]["startupDashboard"],
        false
    );

    // The rung between those two. `defaults` is the level the other two cases step over
    // without touching, so a break in it — `pick` skipping the middle, or `defaults` losing
    // to the top level — passes everything above and is found by nobody. With the entry
    // silent and the two outer levels disagreeing, `false` here names `defaults` and only
    // `defaults`, which makes this one assertion pin the whole order: entry > defaults > root.
    let per_defaults = Fixture::new(DASHBOARD_PER_DEFAULTS);
    assert_eq!(
        per_defaults.json(&["config"])["settings"]["startupDashboard"],
        false
    );

    // The flag still outranks the file it disagrees with, asked of the same fixture that
    // says `false` so that `true` coming back can only have come from the flag.
    let overridden = per_repo
        .command(["config"])
        .env("ADJUTANT_STARTUP_DASHBOARD", "1")
        .output()
        .unwrap();
    let answer: serde_json::Value =
        serde_json::from_slice(&overridden.stdout).expect("config printed no JSON");
    assert_eq!(answer["settings"]["startupDashboard"], true);
}

#[test]
fn the_worktree_fallback_answers_without_any_other_tool_installed() {
    let fixture = Fixture::new(QUIET);
    let path = fixture.ok(&["worktree-path", "--branch", "someone/WID-1"]);
    assert!(
        path.trim().ends_with(".claude/worktrees/someone-WID-1"),
        "{path}"
    );

    // From a task name it answers all three at once: branch, path, and what to branch from.
    // Two commands for two halves of one decision is how the halves drift apart.
    let out: serde_json::Value = serde_json::from_str(&fixture.ok(&[
        "worktree-path",
        "--name",
        "wid-1",
        "--user",
        "someone",
    ]))
    .unwrap();
    assert_eq!(out["branch"], "someone/wid-1");
    assert_eq!(
        out["path"].as_str().unwrap(),
        format!("{}/.claude/worktrees/someone-wid-1", fixture.repo.display())
    );
    // The checkout to run `git worktree add` *in* — not what to branch from. Called `base`
    // once, and the procedure passed it where git wants a commit-ish.
    assert_eq!(
        out["main"].as_str().unwrap(),
        fixture.repo.to_string_lossy()
    );
    assert!(out.get("base").is_none(), "{out}");
}

#[test]
fn a_task_source_with_its_own_branch_shape_overrides_the_fallback() {
    let fixture = Fixture::new(QUIET);
    let out: serde_json::Value = serde_json::from_str(&fixture.ok(&[
        "worktree-path",
        "--name",
        "wid-1",
        "--user",
        "someone",
        "--pattern",
        "feature/{name}",
    ]))
    .unwrap();
    assert_eq!(out["branch"], "feature/wid-1");
}

#[test]
fn asking_for_neither_a_branch_nor_a_name_is_an_error_not_a_guess() {
    let fixture = Fixture::new(QUIET);
    assert!(!fixture.cmd(&["worktree-path"]).status.success());
}

#[test]
fn the_shipped_example_config_resolves_without_a_single_warning() {
    // The example is the schema documentation. A warning in it means the documentation
    // describes a config the resolver disagrees with.
    let fixture = Fixture::new(&std::fs::read_to_string("config.example.json").unwrap());
    for repo in [
        "example/android-app",
        "example/web",
        "example/service",
        "example/legacy",
    ] {
        let out = fixture.json(&["config", "--repo", repo]);
        assert_eq!(out["registered"], true, "{repo}");
        assert_eq!(
            out["warnings"].as_array().unwrap().len(),
            0,
            "{repo}: {}",
            out["warnings"]
        );
        assert!(
            !out["config"]["taskSources"].as_array().unwrap().is_empty(),
            "{repo}"
        );
    }
}
