use super::*;

#[test]
fn a_relative_value_is_anchored_to_the_directory_it_was_given_in() {
    let cwd = Path::new("/work/repo");
    assert_eq!(
        anchored("my-config.json", cwd),
        Some(PathBuf::from("/work/repo/my-config.json"))
    );
    assert_eq!(
        anchored("./sub/config.json", cwd),
        Some(PathBuf::from("/work/repo/sub/config.json"))
    );
    let up = anchored("../config.json", cwd).unwrap();
    assert!(up.is_absolute());
    assert_eq!(up, PathBuf::from("/work/repo/../config.json"));
}

#[test]
fn an_absolute_empty_or_home_relative_value_is_left_as_it_was() {
    let cwd = Path::new("/work/repo");
    assert_eq!(anchored("/etc/adjutant/config.json", cwd), None);
    assert_eq!(anchored("", cwd), None);
    assert_eq!(anchored("~", cwd), None);
    assert_eq!(anchored("~/cfg/config.json", cwd), None);
}

/// Nothing from the environment, always and explicitly: no startup flag, no tmux session or
/// socket. Those a hub exports are `resolve_config`'s to read, and a test that picked them
/// up from the terminal would be reporting on the tab it was run in.
fn resolve(raw: Value, nwo: &str) -> (Option<Value>, Settings, Vec<String>) {
    let (_, config, settings, warnings) = resolve_from_value(&raw, nwo, &ResolveEnv::default());
    (config, settings, warnings)
}

#[test]
fn an_unregistered_repo_still_gets_settings() {
    let (config, settings, _) = resolve(
        json!({"terminal": {"spawn": "tmux new-window {command}"}, "repos": {}}),
        "acme/widget",
    );
    assert!(config.is_none());
    assert_eq!(
        settings.terminal.spawn.as_deref(),
        Some("tmux new-window {command}")
    );
}

#[test]
fn the_flat_shorthand_becomes_one_source() {
    let (config, _, _) = resolve(
        json!({"repos": {"acme/web": {
            "taskSource": "github",
            "issueRepo": "acme/web",
            "issueKeys": {"acme/web": "WEB"},
            "ide": "code"
        }}}),
        "acme/web",
    );
    let config = config.unwrap();
    let sources = config["taskSources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["type"], "github");
    assert_eq!(sources[0]["issueRepo"], "acme/web");
    // The source keys are lifted, not copied: two places to read `issueRepo` from is how
    // a prompt ends up reading the stale one.
    assert!(config.get("issueRepo").is_none());
    assert!(config.get("taskSource").is_none());
}

#[test]
fn every_source_gets_a_worktree_name_so_two_trackers_cannot_collide() {
    let (config, _, _) = resolve(
        json!({"repos": {"acme/app": {"taskSources": [
            {"type": "github-project", "projectOwner": "acme", "projectNumber": 9}
        ], "issueKeys": {"acme/app": "WID"}, "ide": "studio"}}}),
        "acme/app",
    );
    assert_eq!(
        config.unwrap()["taskSources"][0]["worktreeName"],
        DEFAULT_WORKTREE_NAME
    );
}

#[test]
fn defaults_merge_under_the_entry_but_never_supply_sources() {
    let (config, _, warnings) = resolve(
        json!({
            "defaults": {"selfReviewRounds": 3, "draftPr": true, "taskSource": "github"},
            "repos": {"acme/app": {
                "selfReviewRounds": 9,
                "taskSources": [{"type": "github", "issueRepo": "acme/app"}],
                "issueKeys": {"acme/app": "WID"},
                "ide": "code"
            }}
        }),
        "acme/app",
    );
    let config = config.unwrap();
    assert_eq!(config["selfReviewRounds"], 9);
    assert_eq!(config["draftPr"], true);
    assert_eq!(config["taskSources"].as_array().unwrap().len(), 1);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("taskSource in defaults"))
    );
}

#[test]
fn builtin_defaults_fill_the_gaps() {
    let (config, _, _) = resolve(
        json!({"repos": {"acme/app": {"taskSource": "github", "issueRepo": "acme/app",
               "issueKeys": {"acme/app": "WID"}, "ide": "code"}}}),
        "acme/app",
    );
    let config = config.unwrap();
    assert_eq!(config["reviewEffort"], "high");
    assert_eq!(config["selfReviewRounds"], 5);
    assert_eq!(config["baseBranch"], "auto");
    assert_eq!(config["copilotReview"], "ask");
}

#[test]
fn copilot_review_is_set_per_repo_over_defaults() {
    let (config, _, warnings) = resolve(
        json!({
            "defaults": {"copilotReview": "never"},
            "repos": {
                "acme/app": {"taskSource": "github", "issueRepo": "acme/app",
                    "issueKeys": {"acme/app": "WID"}, "ide": "code",
                    "copilotReview": "always"},
                "acme/lib": {"taskSource": "github", "issueRepo": "acme/lib",
                    "issueKeys": {"acme/lib": "XYZ"}, "ide": "code"}
            }
        }),
        "acme/app",
    );
    assert_eq!(config.unwrap()["copilotReview"], "always");
    assert!(!warnings.iter().any(|w| w.contains("copilotReview")));

    let (config, _, _) = resolve(
        json!({
            "defaults": {"copilotReview": "never"},
            "repos": {"acme/lib": {"taskSource": "github", "issueRepo": "acme/lib",
                "issueKeys": {"acme/lib": "XYZ"}, "ide": "code"}}
        }),
        "acme/lib",
    );
    assert_eq!(config.unwrap()["copilotReview"], "never");
}

#[test]
fn an_unknown_copilot_review_value_is_reported() {
    for value in [json!("alway"), json!(true)] {
        let (_, _, warnings) = resolve(
            json!({"repos": {"acme/app": {"taskSource": "github", "issueRepo": "acme/app",
                   "issueKeys": {"acme/app": "WID"}, "ide": "code",
                   "copilotReview": value}}}),
            "acme/app",
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("copilotReview") && w.contains("treated as ask")),
            "no warning for {value}: {warnings:?}"
        );
    }
}

#[test]
fn the_resolved_settings_are_spelled_the_way_the_config_file_spells_them() {
    let (_, settings, _) = resolve(
        // Every optional key is set, because several are skipped when absent and a
        // skipped key would pass an "is it spelled right" check for free.
        json!({"hubWake": "poke", "workerWake": "poke2", "agentRunner": "run {prompt}",
               "hubRunner": "start {name}", "worktreePattern": ".wt/{name}",
               "agentResumeRunner": "again {sessionId}", "hubAutoResumeHours": 1,
               "maxWorkers": 3, "stuckAfterMinutes": 30, "julesKey": "print-key", "language": "ja",
               "hubResumeRunner": "again {name} {sessionId}",
               "agentEnv": {"K": "v"}, "ide": "code", "startupDashboard": false, "hubServe": false,
               "terminal": {"spawn": "s", "focus": "f", "close": "c", "title": "t"},
               "repos": {}}),
        "acme/app",
    );
    let text = serde_json::to_value(settings).unwrap();
    for key in [
        "hubWake",
        "workerWake",
        "agentRunner",
        "hubRunner",
        "agentResumeRunner",
        "hubResumeRunner",
        "worktreePattern",
        "agentEnv",
        "startupDashboard",
        "hubServe",
        "hubAutoResumeHours",
        "maxWorkers",
        "stuckAfterMinutes",
        "julesKey",
        "language",
    ] {
        assert!(text.get(key).is_some(), "{key} is missing from {text}");
    }
    for key in [
        "hub_wake",
        "worker_wake",
        "agent_runner",
        "hub_runner",
        "agent_resume_runner",
        "hub_resume_runner",
        "hub_auto_resume_hours",
        "worktree_pattern",
        "agent_env",
        "startup_dashboard",
        "hub_serve",
        "max_workers",
        "stuck_after_minutes",
        "jules_key",
    ] {
        assert!(
            text.get(key).is_none(),
            "{key} is still snake_case in {text}"
        );
    }
}

#[test]
fn per_key_documentation_is_dropped_too() {
    // The schema documents individual keys with a `//<key>` sibling, not only with a
    // bare `//`. Both are prose and neither belongs in what a session reads.
    let (config, _, _) = resolve(
        json!({"repos": {"acme/app": {
            "//": "bare prose",
            "//verify": "prose about verify",
            "verify": ["cargo test"],
            "issueCreate": {"//notes": "prose about notes", "command": "x"},
            "taskSource": "github", "issueRepo": "acme/app",
            "issueKeys": {"acme/app": "WID"}, "ide": "code", "language": "ja"
        }}}),
        "acme/app",
    );
    let text = serde_json::to_string(&config.unwrap()).unwrap();
    assert!(!text.contains("prose"), "{text}");
}

#[test]
fn documentation_keys_never_reach_the_reader() {
    let (config, _, _) = resolve(
        json!({"repos": {"acme/app": {
            "//": "prose the session does not need",
            "taskSources": [{"//": "also prose", "type": "github", "issueRepo": "acme/app"}],
            "issueKeys": {"acme/app": "WID"},
            "ide": "code"
        }}}),
        "acme/app",
    );
    let text = serde_json::to_string(&config.unwrap()).unwrap();
    assert!(!text.contains("prose"), "{text}");
}

#[test]
fn a_source_missing_its_required_keys_warns_instead_of_failing() {
    let (config, _, warnings) = resolve(
        json!({"repos": {"acme/app": {"taskSources": [{"type": "jira", "jira": {"project": "ABC"}}],
               "issueKeys": {"acme/app": "WID"}, "ide": "code"}}}),
        "acme/app",
    );
    assert!(config.is_some());
    assert!(
        warnings.iter().any(|w| w.contains("jira.cloudId")),
        "{warnings:?}"
    );
}

#[test]
fn a_duplicated_issue_key_warns() {
    let (_, _, warnings) = resolve(
        json!({"repos": {"acme/app": {
            "taskSources": [{"type": "github", "issueRepo": "acme/app"}],
            "issueKeys": {"acme/app": "WID", "acme/other": "WID"},
            "ide": "code"
        }}}),
        "acme/app",
    );
    assert!(
        warnings.iter().any(|w| w.contains("used more than once")),
        "{warnings:?}"
    );
}

#[test]
fn an_issue_repo_outside_issue_keys_warns_because_its_issues_would_vanish() {
    let (_, _, warnings) = resolve(
        json!({"repos": {"acme/app": {
            "taskSources": [{"type": "github", "issueRepo": "acme/other"}],
            "issueKeys": {"acme/app": "WID"},
            "ide": "code"
        }}}),
        "acme/app",
    );
    assert!(
        warnings.iter().any(|w| w.contains("acme/other")),
        "{warnings:?}"
    );
}

#[test]
fn a_missing_ide_warns() {
    let (_, settings, warnings) = resolve(
        json!({"repos": {"acme/app": {"taskSource": "github", "issueRepo": "acme/app",
               "issueKeys": {"acme/app": "WID"}}}}),
        "acme/app",
    );
    assert!(settings.ide.is_none());
    assert!(warnings.iter().any(|w| w.contains("ide")), "{warnings:?}");
}

#[test]
fn the_repo_key_matches_case_insensitively() {
    let (config, _, _) = resolve(
        json!({"repos": {"Acme/App": {"taskSource": "github", "issueRepo": "Acme/App",
               "issueKeys": {"Acme/App": "WID"}, "ide": "code"}}}),
        "acme/app",
    );
    assert!(config.is_some());
}

#[test]
fn settings_take_the_most_specific_answer() {
    let (_, settings, _) = resolve(
        json!({
            "agentRunner": "codex exec '{prompt}'",
            "notification": {"command": "curl -d {message} https://example.invalid"},
            "defaults": {"agentRunner": "agy run '{prompt}'", "ide": "code"},
            "repos": {"acme/app": {
                "agentRunner": "claude '{prompt}'",
                "taskSource": "github", "issueRepo": "acme/app",
                "issueKeys": {"acme/app": "WID"}, "ide": "studio"
            }}
        }),
        "acme/app",
    );
    assert_eq!(settings.agent_runner.as_deref(), Some("claude '{prompt}'"));
    assert_eq!(settings.ide.as_deref(), Some("studio"));
    assert_eq!(
        settings.notification.template(),
        Some("curl -d {message} https://example.invalid")
    );
}

#[test]
fn a_repo_can_carry_its_own_agent_environment() {
    let (_, settings, _) = resolve(
        json!({"repos": {"acme/app": {"agentEnv": {"CLAUDE_CONFIG_DIR": "/cfg/app"},
               "taskSource": "github", "issueRepo": "acme/app",
               "issueKeys": {"acme/app": "WID"}, "ide": "code"}}}),
        "acme/app",
    );
    assert_eq!(
        settings.agent_env,
        vec![("CLAUDE_CONFIG_DIR".to_string(), "/cfg/app".to_string())]
    );
}

#[test]
fn a_hub_collects_the_dashboard_at_startup_until_somebody_says_not_to() {
    // Nothing configured and no flag: the hub collects, which is what every machine that
    // has never heard of this setting has to keep doing.
    assert!(startup_dashboard(None, None));
    assert!(!startup_dashboard(Some(&json!(false)), None));
    assert!(startup_dashboard(Some(&json!(true)), None));
}

#[test]
fn the_flag_a_hub_was_started_under_outranks_what_the_config_says() {
    // `--no-dashboard` against a config that says collect…
    assert!(!startup_dashboard(Some(&json!(true)), Some("0")));
    // …and `--dashboard` against a config that says don't, which is the case the second
    // flag exists for: a standing preference is not a thing you want to edit twice.
    assert!(startup_dashboard(Some(&json!(false)), Some("1")));
    // Anything that is neither leaves the configured answer standing. The variable is
    // inherited by every process a hub starts, so a mistyped export that reversed a
    // setting would follow the person around all day.
    //
    // Both fixtures, on purpose: an implementation that read every value other than
    // `"1"` as off would satisfy the second on its own, and the pair is what tells
    // "falls through" apart from "unknown means no".
    assert!(startup_dashboard(Some(&json!(true)), Some("no")));
    assert!(!startup_dashboard(Some(&json!(false)), Some("no")));
}

#[test]
fn a_repository_can_keep_its_own_hub_from_collecting_at_startup() {
    // Machine level says collect, this repository says don't: the more specific level
    // wins, as it does for every other setting. A repo whose board is enormous is
    // exactly the one that wants this, and it is the only one that should get it.
    //
    // Asked of the level picker rather than of the resolved `bool`, because the resolved
    // one also answers to `ADJUTANT_STARTUP_DASHBOARD` — and a `cargo test` typed in a
    // hub's own tab inherits that. Split this way the two halves are each checkable on
    // their own: which level wins here, and what the flag does to it above.
    let map = |value: Value| value.as_object().cloned().unwrap();
    let none = Map::new();
    let machine_on = map(json!({"startupDashboard": true}));
    let repo_off = map(json!({"startupDashboard": false}));
    assert_eq!(
        pick_level(&repo_off, &none, &machine_on, "startupDashboard"),
        Some(json!(false))
    );
    // And nothing at the specific level leaves the machine's answer standing, or the
    // setting would only ever be writable per repository.
    assert_eq!(
        pick_level(&none, &none, &machine_on, "startupDashboard"),
        Some(json!(true))
    );
    assert_eq!(pick_level(&none, &none, &none, "startupDashboard"), None);
}

#[test]
fn a_startup_dashboard_that_is_not_a_yes_or_no_is_said_out_loud_and_dropped() {
    // `"false"` the string is the shape somebody writes when they are thinking of the
    // command-line settings around it, and it reads as "set" to a `as_bool` that then
    // says `None`. Dropped either way — but dropped in silence is a person who turned
    // the dashboard off and watched it collect anyway.
    let (_, _, warnings) = resolve(
        json!({"startupDashboard": "false", "repos": {}}),
        "acme/app",
    );
    assert!(
        warnings.iter().any(|w| w.contains("startupDashboard")),
        "{warnings:?}"
    );
    // And what it resolves to once dropped, asked of the decision directly so that no
    // ambient variable can answer for it.
    assert!(startup_dashboard(Some(&json!("false")), None));
}

#[test]
fn machine_settings_stay_out_of_the_repo_config() {
    let (config, _, _) = resolve(
        json!({"repos": {"acme/app": {
            "terminal": {"spawn": "x {command}"}, "agentRunner": "y {prompt}",
            "worktreePattern": ".worktrees/{name}", "startupDashboard": false,
            "taskSource": "github", "issueRepo": "acme/app",
            "issueKeys": {"acme/app": "WID"}, "ide": "code"
        }}}),
        "acme/app",
    );
    let config = config.unwrap();
    for key in [
        "terminal",
        "agentRunner",
        "agentEnv",
        "worktreePattern",
        "notification",
        "startupDashboard",
        "language",
    ] {
        assert!(config.get(key).is_none(), "{key} leaked into config");
    }
}

#[test]
fn a_blank_language_is_none_and_a_non_string_is_reported() {
    for blank in ["", "   "] {
        let (_, settings, warnings) = resolve(a_repo(json!({"language": blank})), "acme/app");
        assert_eq!(settings.language, None, "{blank:?}");
        assert!(warnings.is_empty(), "{warnings:?}");
    }
    let (_, settings, _) = resolve(a_repo(json!({"language": " ja "})), "acme/app");
    assert_eq!(settings.language.as_deref(), Some("ja"));
    let (_, settings, warnings) = resolve(a_repo(json!({"language": 3})), "acme/app");
    assert_eq!(settings.language, None);
    assert!(warning_about(&warnings, "language").contains("a string"));
}

#[test]
fn waking_says_how_and_what_independently() {
    let (_, settings, _) = resolve(
        json!({
            // How to poke is a property of the terminal…
            "wake": "tmux send-keys -t {tty} {line} Enter",
            // …and what to say is a property of the agent, so a repo running a
            // different one restates only that half.
            "defaults": {"ide": "code"},
            "repos": {"acme/app": {
                "workerWake": {"line": "check /adj-outbox"},
                "taskSource": "github", "issueRepo": "acme/app",
                "issueKeys": {"acme/app": "WID"}
            }}
        }),
        "acme/app",
    );
    assert_eq!(
        settings.hub_wake.hook.template(),
        Some("tmux send-keys -t {tty} {line} Enter")
    );
    assert_eq!(settings.hub_wake.line, None);
    assert_eq!(settings.hub_wake.line_or("default"), "default");

    // The worker inherits the machine's poke and overrides only the sentence.
    assert_eq!(
        settings.worker_wake.hook.template(),
        Some("tmux send-keys -t {tty} {line} Enter")
    );
    assert_eq!(settings.worker_wake.line_or("default"), "check /adj-outbox");
}

#[test]
fn a_repo_can_be_woken_a_different_way_from_the_machine_default() {
    let (_, settings, _) = resolve(
        json!({
            "wake": "wake-tab {tty} {line}",
            "repos": {"acme/app": {
                "hubWake": {"command": "curl -s -d {line} http://localhost:9/poke",
                            "line": "check the inbox"},
                "taskSource": "github", "issueRepo": "acme/app",
                "issueKeys": {"acme/app": "WID"}, "ide": "code"
            }}
        }),
        "acme/app",
    );
    assert_eq!(
        settings.hub_wake.hook.template(),
        Some("curl -s -d {line} http://localhost:9/poke")
    );
    assert_eq!(settings.hub_wake.line_or("x"), "check the inbox");
}

#[test]
fn waking_can_be_turned_off_in_either_form() {
    let (_, a, _) = resolve(json!({"wake": false}), "acme/app");
    assert!(a.hub_wake.hook.is_off());
    let (_, b, _) = resolve(json!({"hubWake": {"command": false}}), "acme/app");
    assert!(b.hub_wake.hook.is_off());
    // …and turning one direction off leaves the other alone.
    let (_, c, _) = resolve(
        json!({"wake": "poke {pid}", "workerWake": false}),
        "acme/app",
    );
    assert!(c.worker_wake.hook.is_off());
    assert_eq!(c.hub_wake.hook.template(), Some("poke {pid}"));
}

#[test]
fn a_notification_given_as_a_bare_string_works_too() {
    let (_, settings, _) = resolve(json!({"notification": "printf '\\a'"}), "acme/app");
    assert_eq!(settings.notification.template(), Some("printf '\\a'"));
}

#[test]
fn a_non_array_task_sources_warns_rather_than_panicking() {
    let (_, _, warnings) = resolve(
        json!({"repos": {"acme/app": {"taskSources": "github", "ide": "code"}}}),
        "acme/app",
    );
    assert!(
        warnings.iter().any(|w| w.contains("not an array")),
        "{warnings:?}"
    );
}

/// A registered repo, so the interesting warnings are the only ones in the list.
fn a_repo(extra: Value) -> Value {
    let mut entry = json!({
        "taskSource": "github", "issueRepo": "acme/app",
        "issueKeys": {"acme/app": "WID"}, "ide": "code"
    });
    let map = entry.as_object_mut().unwrap();
    for (k, v) in extra.as_object().unwrap() {
        map.insert(k.clone(), v.clone());
    }
    json!({
        "hubWake": {"command": "tmux send-keys {line}"},
        "repos": {"acme/app": entry}
    })
}

fn warning_about(warnings: &[String], needle: &str) -> String {
    warnings
        .iter()
        .find(|w| w.contains(needle))
        .unwrap_or_else(|| panic!("nothing said about {needle}: {warnings:?}"))
        .clone()
}

#[test]
fn the_long_form_of_notification_can_be_turned_off() {
    // The short form has always been able to say "off". The long form dropped the
    // `false` on the floor and ran the built-in notifier anyway, which is the one
    // answer the person writing it was trying to prevent.
    let (_, settings, _) = resolve(
        a_repo(json!({"notification": {"command": false}})),
        "acme/app",
    );
    assert!(
        settings.notification.is_off(),
        "{:?}",
        settings.notification
    );

    let (_, settings, _) = resolve(
        a_repo(json!({"notification": {"command": "say hi"}})),
        "acme/app",
    );
    assert_eq!(settings.notification.template(), Some("say hi"));
}

#[test]
fn a_config_of_the_wrong_shape_says_so_instead_of_vanishing() {
    // `repos` as an array resolved to "this repository is not registered", which is
    // also what a correct config for a different repository looks like.
    let (config, _, warnings) = resolve(json!({"repos": [{"acme/app": {}}]}), "acme/app");
    assert!(config.is_none());
    assert!(warning_about(&warnings, "repos").contains("an array"));

    let (_, _, warnings) = resolve(json!({"repos": {"acme/app": "github"}}), "acme/app");
    assert!(warning_about(&warnings, "entry").contains("a string"));
}

#[test]
fn a_setting_of_the_wrong_shape_says_so_instead_of_being_dropped() {
    let (_, settings, warnings) = resolve(
        a_repo(json!({"terminal": "iterm", "hubRunner": 5})),
        "acme/app",
    );
    assert!(warning_about(&warnings, "terminal").contains("a string"));
    assert!(warning_about(&warnings, "hubRunner").contains("a number"));
    // Still dropped — a half-understood setting is worse than none. What changed is
    // that the person is told.
    assert_eq!(settings.terminal.spawn, None);
    assert_eq!(settings.hub_runner, None);

    let (_, _, warnings) = resolve(a_repo(json!({"terminal": {"title": 5}})), "acme/app");
    assert!(warning_about(&warnings, "terminal.title").contains("a number"));

    let (_, _, warnings) = resolve(a_repo(json!({"terminal": {"close": 5}})), "acme/app");
    assert!(warning_about(&warnings, "terminal.close").contains("a number"));
}

#[test]
fn closing_a_tab_is_a_template_like_opening_one() {
    // Reaching the struct and being in the shape table are two independent additions: a
    // field the table has never heard of reads a string perfectly well and says nothing
    // at all about the config that put a number there.
    let (_, settings, warnings) = resolve(
        a_repo(json!({"terminal": {"close": "close-tab {tty}"}})),
        "acme/app",
    );
    assert_eq!(settings.terminal.close.template(), Some("close-tab {tty}"));
    assert!(warnings.is_empty(), "{warnings:?}");

    // And off is its own answer rather than a missing one: read as unset, `false`
    // would hand the tab to the built-in closer, which is the opposite of what it says.
    let (_, settings, warnings) =
        resolve(a_repo(json!({"terminal": {"close": false}})), "acme/app");
    assert!(settings.terminal.close.is_off());
    assert_eq!(settings.terminal.close.template(), None);
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn an_env_key_that_is_not_a_variable_name_never_reaches_a_command_line() {
    // `env K=V …` is built as a shell line, so a `;` in a key is a command separator in
    // front of every worker this repository ever starts.
    let (_, settings, warnings) = resolve(
        a_repo(json!({"agentEnv": {"OK_ONE": "v", "BAD;touch /tmp/pwned": "v", "N": 5}})),
        "acme/app",
    );
    assert_eq!(
        settings.agent_env,
        vec![("OK_ONE".to_string(), "v".to_string())]
    );
    assert!(warning_about(&warnings, "BAD").contains("not a variable name"));
    assert!(warning_about(&warnings, "agentEnv N").contains("a number"));
}

#[test]
fn a_hub_runner_with_nowhere_to_put_the_name_is_called_out() {
    let (_, _, warnings) = resolve(a_repo(json!({"hubRunner": "myagent --resume"})), "acme/app");
    assert!(warning_about(&warnings, "hubRunner").contains("{name}"));
    assert!(warning_about(&warnings, "hubRunner").contains("still runs"));

    let (_, _, warnings) = resolve(
        a_repo(json!({"hubRunner": "myagent -n {name}"})),
        "acme/app",
    );
    assert!(
        !warnings.iter().any(|w| w.contains("hubRunner")),
        "{warnings:?}"
    );
}

#[test]
fn the_auto_resume_window_is_hours_and_falls_back_when_it_cannot_be_one() {
    let (_, settings, _) = resolve(a_repo(json!({})), "acme/app");
    assert_eq!(
        settings.hub_auto_resume_hours,
        DEFAULT_HUB_AUTO_RESUME_HOURS
    );

    let (_, settings, warnings) = resolve(a_repo(json!({"hubAutoResumeHours": 0.5})), "acme/app");
    assert_eq!(settings.hub_auto_resume_hours, 0.5);
    assert!(
        !warnings.iter().any(|w| w.contains("hubAutoResumeHours")),
        "{warnings:?}"
    );

    let (_, settings, warnings) = resolve(a_repo(json!({"hubAutoResumeHours": 0})), "acme/app");
    assert_eq!(settings.hub_auto_resume_hours, 0.0);
    assert!(warnings.is_empty() || !warnings.iter().any(|w| w.contains("hubAutoResume")));

    let (_, settings, warnings) = resolve(a_repo(json!({"hubAutoResumeHours": -1})), "acme/app");
    assert_eq!(
        settings.hub_auto_resume_hours,
        DEFAULT_HUB_AUTO_RESUME_HOURS
    );
    assert!(warning_about(&warnings, "hubAutoResumeHours").contains("0 or more"));

    let (_, settings, warnings) = resolve(a_repo(json!({"hubAutoResumeHours": "3"})), "acme/app");
    assert_eq!(
        settings.hub_auto_resume_hours,
        DEFAULT_HUB_AUTO_RESUME_HOURS
    );
    assert!(warning_about(&warnings, "hubAutoResumeHours").contains("a number"));
}

#[test]
fn the_stuck_threshold_is_minutes_and_falls_back_when_it_cannot_be_one() {
    let (_, settings, _) = resolve(a_repo(json!({})), "acme/app");
    assert_eq!(settings.stuck_after_minutes, DEFAULT_STUCK_AFTER_MINUTES);
    let (_, settings, _) = resolve(a_repo(json!({"stuckAfterMinutes": 0})), "acme/app");
    assert_eq!(settings.stuck_after_minutes, 0.0);
    let (_, settings, warnings) = resolve(a_repo(json!({"stuckAfterMinutes": -5})), "acme/app");
    assert_eq!(settings.stuck_after_minutes, DEFAULT_STUCK_AFTER_MINUTES);
    assert!(warning_about(&warnings, "stuckAfterMinutes").contains("0 or more"));
}

#[test]
fn the_worker_limit_is_a_whole_number_and_anything_else_means_no_limit() {
    let (_, settings, _) = resolve(a_repo(json!({})), "acme/app");
    assert_eq!(settings.max_workers, None);

    let (_, settings, warnings) = resolve(a_repo(json!({"maxWorkers": 4})), "acme/app");
    assert_eq!(settings.max_workers, Some(4));
    assert!(
        !warnings.iter().any(|w| w.contains("maxWorkers")),
        "{warnings:?}"
    );

    for bad in [json!(0), json!(-1), json!(2.5)] {
        let (_, settings, warnings) = resolve(a_repo(json!({"maxWorkers": bad})), "acme/app");
        assert_eq!(settings.max_workers, None, "{bad}");
        assert!(warning_about(&warnings, "maxWorkers").contains("1 or more"));
    }

    let (_, settings, warnings) = resolve(a_repo(json!({"maxWorkers": "4"})), "acme/app");
    assert_eq!(settings.max_workers, None);
    assert!(warning_about(&warnings, "maxWorkers").contains("a number"));
}

#[test]
fn a_resume_runner_that_cannot_be_told_the_session_is_called_out() {
    let (_, settings, warnings) = resolve(
        a_repo(json!({"hubResumeRunner": "myagent --continue",
                      "agentResumeRunner": "myagent resume {sessionId}"})),
        "acme/app",
    );
    assert!(warning_about(&warnings, "hubResumeRunner").contains("{sessionId}"));
    assert!(
        !warnings.iter().any(|w| w.contains("agentResumeRunner")),
        "{warnings:?}"
    );
    assert_eq!(
        settings.agent_resume_runner.as_deref(),
        Some("myagent resume {sessionId}")
    );
}

#[test]
fn the_editor_is_answered_once_not_twice() {
    // `ide` describes the machine, like `terminal` and `notification`. Left in the
    // per-repo config as well, `adj config` gave two answers that could disagree.
    let (config, settings, _) = resolve(a_repo(json!({})), "acme/app");
    assert_eq!(settings.ide.as_deref(), Some("code"));
    assert!(config.unwrap().get("ide").is_none());
}

#[test]
fn one_repo_changing_its_tab_title_keeps_the_machines_terminal() {
    // Picked whole, this entry dropped `spawn` and `focus` for that repository — and
    // the symptom of a missing `spawn` is a tab that never opens.
    let (_, settings, _) = resolve(
        json!({
            "terminal": {"spawn": "tmux new-window -c {cwd} {command}", "focus": "raise {pid}",
                         "close": "close-tab {tty}"},
            "repos": {"acme/app": {
                "taskSource": "github", "issueRepo": "acme/app",
                "issueKeys": {"acme/app": "WID"}, "ide": "code",
                "terminal": {"title": "tmux rename-window {title}"}
            }}
        }),
        "acme/app",
    );
    assert_eq!(
        settings.terminal.spawn.as_deref(),
        Some("tmux new-window -c {cwd} {command}")
    );
    assert_eq!(settings.terminal.focus.as_deref(), Some("raise {pid}"));
    assert_eq!(settings.terminal.close.template(), Some("close-tab {tty}"));
    assert_eq!(
        settings.terminal.title.template(),
        Some("tmux rename-window {title}")
    );
}

#[test]
fn a_repo_changing_only_the_sentence_keeps_the_machines_wake() {
    // The same rule `Wake::resolve` applies between `wake` and `hubWake` has to apply
    // between the levels, or a repository that changes what is *said* silently throws
    // away how this machine *pokes*.
    let (_, settings, _) = resolve(
        a_repo(json!({"hubWake": {"line": "look in your inbox"}})),
        "acme/app",
    );
    assert_eq!(
        settings.hub_wake.hook.template(),
        Some("tmux send-keys {line}")
    );
    assert_eq!(settings.hub_wake.line_or("built-in"), "look in your inbox");

    // A bare value still replaces outright: it names a whole mechanism, and merging it
    // into an object would invent a setting nobody wrote.
    let (_, settings, _) = resolve(a_repo(json!({"hubWake": false})), "acme/app");
    assert!(settings.hub_wake.hook.is_off());
}

#[test]
fn the_sentence_being_the_wrong_type_is_reported_like_everything_else() {
    let (_, settings, warnings) = resolve(a_repo(json!({"hubWake": {"line": 5}})), "acme/app");
    assert!(warning_about(&warnings, "hubWake.line").contains("a number"));
    assert_eq!(settings.hub_wake.line, None);
}

#[test]
fn terminal_tmux_preset_resolves() {
    let (_, settings, warnings) =
        resolve(a_repo(json!({"terminal": {"preset": "tmux"}})), "acme/app");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(settings.terminal.is_tmux());
    assert_eq!(settings.terminal.preset.as_deref(), Some("tmux"));
    assert_eq!(settings.terminal.tmux_session(), "adjutant");
    assert_eq!(settings.terminal.tmux_socket(), None);
}

#[test]
fn the_tmux_session_and_socket_a_hub_exports_win_over_the_file() {
    let raw = a_repo(
        json!({"terminal": {"preset": "tmux", "session": "from-file", "socket": "file-sock"}}),
    );
    let env = ResolveEnv {
        tmux_session: Some("hub-session".into()),
        tmux_socket: Some("hub-sock".into()),
        ..ResolveEnv::default()
    };
    let (_, _, settings, warnings) = resolve_from_value(&raw, "acme/app", &env);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(settings.terminal.tmux_session(), "hub-session");
    assert_eq!(settings.terminal.tmux_socket(), Some("hub-sock"));
    // Blank is not an answer: the file stands.
    let blank = ResolveEnv {
        tmux_session: Some("  ".into()),
        tmux_socket: Some("".into()),
        ..ResolveEnv::default()
    };
    let (_, _, settings, _) = resolve_from_value(&raw, "acme/app", &blank);
    assert_eq!(settings.terminal.tmux_session(), "from-file");
    assert_eq!(settings.terminal.tmux_socket(), Some("file-sock"));
}

#[test]
fn terminal_tmux_object_with_custom_session_and_socket() {
    let (_, settings, warnings) = resolve(
        a_repo(json!({
            "terminal": {
                "preset": "tmux",
                "session": "custom-session",
                "socket": "custom-sock",
                "focus": "adj tmux focus {pid}"
            }
        })),
        "acme/app",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(settings.terminal.is_tmux());
    assert_eq!(settings.terminal.tmux_session(), "custom-session");
    assert_eq!(settings.terminal.tmux_socket(), Some("custom-sock"));
    assert_eq!(
        settings.terminal.focus.as_deref(),
        Some("adj tmux focus {pid}")
    );
}

#[test]
fn terminal_preset_overlays_with_nested_object() {
    let config = json!({
        "terminal": { "preset": "tmux" },
        "defaults": {
            "terminal": { "socket": "shared-socket" },
            "ide": "code"
        },
        "repos": {
            "acme/app": {
                "taskSource": "github",
                "issueRepo": "acme/app",
                "issueKeys": { "acme/app": "WID" }
            }
        }
    });
    let (_, settings, warnings) = resolve(config, "acme/app");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(settings.terminal.is_tmux());
    assert_eq!(settings.terminal.tmux_socket(), Some("shared-socket"));
    assert_eq!(settings.terminal.tmux_session(), "adjutant");
}

#[test]
fn terminal_attach_is_read_and_a_non_string_is_warned_about() {
    let (_, settings, warnings) = resolve(
        a_repo(json!({"terminal": {"attach": "open-term {session}"}})),
        "acme/app",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        settings.terminal.attach.as_deref(),
        Some("open-term {session}")
    );
    let (_, settings, warnings) = resolve(a_repo(json!({"terminal": {"attach": 3}})), "acme/app");
    assert_eq!(settings.terminal.attach, None);
    assert!(
        warnings.iter().any(|w| w.contains("terminal.attach")),
        "{warnings:?}"
    );
}

fn app(extra: Value) -> Value {
    let mut entry = json!({"taskSource": "github", "issueRepo": "acme/app",
        "issueKeys": {"acme/app": "WID"}, "ide": "code"});
    entry
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    entry
}

#[test]
fn the_task_keys_are_typed_settings_on_a_registered_repo() {
    let (config, settings, _) = resolve(
        json!({"repos": {"acme/app": app(json!({
            "copilotReview": "always", "reviewEngine": "codex", "verify": ["make check", 3]
        }))}}),
        "acme/app",
    );
    assert_eq!(settings.copilot_review, CopilotReview::Always);
    assert_eq!(settings.review_engine, ReviewEngine::Codex);
    assert_eq!(
        settings.verify,
        vec!["make check".to_string(), "3".to_string()]
    );
    assert_eq!(settings.issue_keys["acme/app"], "WID");
    assert_eq!(settings.task_sources.len(), 1);
    assert_eq!(
        settings.task_sources[0]["worktreeName"],
        DEFAULT_WORKTREE_NAME
    );
    assert_eq!(
        Value::Array(settings.task_sources.clone()),
        config.unwrap()["taskSources"]
    );
}

#[test]
fn an_unknown_copilot_review_value_is_read_as_ask() {
    let (_, settings, _) = resolve(
        json!({"repos": {"acme/app": app(json!({"copilotReview": "alway"}))}}),
        "acme/app",
    );
    assert_eq!(settings.copilot_review, CopilotReview::Ask);
}

#[test]
fn a_review_engine_that_is_not_one_of_the_three_is_kept_as_configured() {
    for (value, text) in [
        (json!("sometimes"), "\"sometimes\""),
        (json!(3), "3"),
        (json!(null), "null"),
    ] {
        let (_, settings, _) = resolve(
            json!({"repos": {"acme/app": app(json!({"reviewEngine": value}))}}),
            "acme/app",
        );
        assert_eq!(
            settings.review_engine,
            ReviewEngine::Other(text.to_string())
        );
        assert_eq!(settings.review_engine.as_str(), text);
    }
}

#[test]
fn copilot_review_is_inherited_from_defaults() {
    let (_, settings, _) = resolve(
        json!({"defaults": {"copilotReview": "never", "reviewEngine": "claude"},
               "repos": {"acme/app": app(json!({}))}}),
        "acme/app",
    );
    assert_eq!(settings.copilot_review, CopilotReview::Never);
    assert_eq!(settings.review_engine, ReviewEngine::Claude);
}

#[test]
fn an_unregistered_repo_keeps_the_built_in_task_keys() {
    let (config, settings, _) = resolve(
        json!({"defaults": {"copilotReview": "never", "reviewEngine": "codex", "verify": ["x"]},
               "repos": {}}),
        "acme/app",
    );
    assert!(config.is_none());
    assert_eq!(settings.copilot_review, CopilotReview::Ask);
    assert_eq!(settings.review_engine, ReviewEngine::Auto);
    assert!(settings.verify.is_empty());
    assert!(settings.issue_keys.is_empty());
    assert!(settings.task_sources.is_empty());
}
