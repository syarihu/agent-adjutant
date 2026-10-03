use super::*;

fn call(method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let response = handle_line(&line.to_string()).expect("a request always gets a reply");
    serde_json::to_value(&response).unwrap()
}

#[test]
fn the_registration_never_names_a_binary_that_is_not_this_one() {
    // Either the bare name — which must then resolve to this very binary — or an
    // absolute path to it. Anything else registers someone else's build.
    let command = server_command();
    match BIN_NAMES.contains(&command.as_str()) {
        true => {
            let found = std::env::split_paths(&std::env::var("PATH").unwrap_or_default())
                .map(|d| d.join(&command))
                .find(|p| p.is_file())
                .and_then(|p| std::fs::canonicalize(p).ok());
            let me = std::env::current_exe()
                .ok()
                .and_then(|p| std::fs::canonicalize(p).ok());
            assert_eq!(found, me);
        }
        false => assert!(Path::new(&command).is_absolute(), "{command}"),
    }
}

#[test]
fn initialize_advertises_both_prompts_and_tools() {
    let out = call("initialize", json!({}));
    assert_eq!(out["result"]["serverInfo"]["name"], SERVER_NAME);
    assert!(out["result"]["capabilities"]["prompts"].is_object());
    assert!(out["result"]["capabilities"]["tools"].is_object());
    assert!(
        out["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("adj-report")
    );
}

#[test]
fn a_notification_gets_no_reply() {
    let line = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert!(handle_line(&line.to_string()).is_none());
}

#[test]
fn a_non_json_line_is_a_parse_error_rather_than_a_crash() {
    let out = serde_json::to_value(handle_line("not json").unwrap()).unwrap();
    assert_eq!(out["error"]["code"], -32700);
}

#[test]
fn all_three_procedures_are_listed_and_fetchable() {
    let listed = call("prompts/list", json!({}));
    let names: Vec<&str> = listed["result"]["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["adj-hub", "adj-worker", "adj-report"]);

    let got = call(
        "prompts/get",
        json!({"name": "adj-report", "arguments": {"arguments": "画像が潰れる"}}),
    );
    let text = got["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("画像が潰れる"));
    assert!(!text.contains("$ARGUMENTS"));
}

#[test]
fn an_unknown_prompt_is_an_error_not_an_empty_procedure() {
    let out = call("prompts/get", json!({"name": "adj-nope"}));
    assert_eq!(out["error"]["code"], -32602);
}

#[test]
fn every_advertised_tool_is_one_the_dispatcher_knows() {
    // Every tool here is called for real, and several of them read the config and the
    // state directory. Without this the test answered about the developer's own
    // machine — harmless while they all happened to be read-only, and a fixture dropped
    // into a live hub's inbox the day one of them is not.
    let _sandbox = crate::testing::Sandbox::empty();
    let listed = call("tools/list", json!({}));
    for tool in listed["result"]["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        assert!(!tool["description"].as_str().unwrap().is_empty(), "{name}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        // "Unknown tool" is the dispatcher saying it has never heard of this name; any
        // other error means it tried and failed, which is what we want here.
        if let Err(e) = call_tool(name, &json!({})) {
            assert!(
                !e.starts_with("Unknown tool"),
                "{name} is advertised but not wired up"
            );
        }
    }
}

/// A hub an agent cannot name is a hub an agent cannot reach. The tools are the only
/// way in for a session that has no shell, so every one of them that answers about a
/// repository's hub has to take which hub of it — otherwise the address MCP reaches is
/// always the repository's own, whatever the session was started as.
#[test]
fn every_tool_that_takes_a_repository_takes_which_hub_of_it() {
    let listed = call("tools/list", json!({}));
    for tool in listed["result"]["tools"].as_array().unwrap() {
        let properties = &tool["inputSchema"]["properties"];
        if properties.get("repo").is_none() {
            continue;
        }
        assert!(
            properties.get("hub").is_some(),
            "{} takes a repository but not which hub of it",
            tool["name"]
        );
    }
}

/// The same split the command line makes, made where the agent actually stands.
#[test]
fn a_tool_call_naming_a_hub_moves_the_address_and_not_the_lookup() {
    let _sandbox = crate::testing::Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["remote", "add", "origin", "git@github.com:acme/widget.git"],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(&args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success(),
            "git {args:?}"
        );
    }
    let cwd = json!(dir.path().to_string_lossy());

    let plain = resolve_repo(&json!({"cwd": cwd})).unwrap();
    let feature = resolve_repo(&json!({"cwd": cwd, "hub": "wid-957"})).unwrap();
    assert!(plain.hub.is_none());
    assert_eq!(feature.hub.as_deref(), Some("wid-957"));
    // Two addresses…
    assert_ne!(plain.hub_name, feature.hub_name);
    assert_ne!(plain.slug, feature.slug);
    // …and one repository, which is the key `config::resolve_config` is handed.
    assert_eq!(plain.nwo, feature.nwo);
    assert_eq!(feature.nwo, "acme/widget");

    // Blank is silence here too. A client filling every advertised property in with an
    // empty string would otherwise address a hub nobody can name a second time.
    assert_eq!(
        resolve_repo(&json!({"cwd": cwd, "hub": ""}))
            .unwrap()
            .hub_name,
        plain.hub_name
    );
}

#[test]
fn a_tool_failure_comes_back_as_content_the_model_can_read() {
    let out = call(
        "tools/call",
        json!({"name": "adjutant_skill", "arguments": {"name": "nope"}}),
    );
    assert_eq!(out["result"]["isError"], true);
    assert!(
        out["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("adj-hub")
    );
    assert!(out["error"].is_null());
}

static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct ClientNameGuard;
impl Drop for ClientNameGuard {
    fn drop(&mut self) {
        set_client_name(None);
    }
}

#[test]
fn the_skill_tool_serves_the_same_text_as_the_prompt() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let via_tool = call_tool("adjutant_skill", &json!({"name": "adj-worker"})).unwrap();
    let via_prompt = prompt_get(&json!({"name": "adj-worker"})).unwrap();
    assert_eq!(
        via_tool["content"].as_str().unwrap(),
        via_prompt["messages"][0]["content"]["text"]
            .as_str()
            .unwrap()
    );
}

#[test]
fn an_unknown_method_is_method_not_found() {
    let out = call("resources/list", json!({}));
    assert_eq!(out["error"]["code"], -32601);
}

#[test]
fn the_skill_tool_respects_explicit_agent_format() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let claude = call_tool(
        "adjutant_skill",
        &json!({"name": "adj-hub", "agent": "claude"}),
    )
    .unwrap();
    let agy = call_tool(
        "adjutant_skill",
        &json!({"name": "adj-hub", "agent": "agy"}),
    )
    .unwrap();

    assert_eq!(claude["agent"], "claude");
    assert_eq!(agy["agent"], "agy");

    let claude_text = claude["content"].as_str().unwrap();
    let agy_text = agy["content"].as_str().unwrap();

    assert!(claude_text.contains("AskUserQuestion"));
    assert!(!agy_text.contains("AskUserQuestion"));
    assert!(agy_text.contains("ask_question"));
}

#[test]
fn client_info_initialization_defaults_agent_format() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let _guard = ClientNameGuard;
    let _ = call(
        "initialize",
        json!({"clientInfo": {"name": "antigravity-cli"}}),
    );
    let via_prompt = prompt_get(&json!({"name": "adj-hub"})).unwrap();
    let text = via_prompt["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(!text.contains("AskUserQuestion"));
    assert!(text.contains("ask_question"));
}

#[test]
fn procedure_runner_selection_distinguishes_hub_and_worker() {
    let settings = config::Settings {
        hub_runner: Some("claude -n {name} {prompt}".to_string()),
        agent_runner: Some("agy --dangerously-skip-permissions -i {prompt}".to_string()),
        ..Default::default()
    };

    assert_eq!(
        runner_for_procedure(&settings, "adj-hub"),
        Some("claude -n {name} {prompt}".to_string())
    );
    assert_eq!(
        runner_for_procedure(&settings, "adj-worker"),
        Some("agy --dangerously-skip-permissions -i {prompt}".to_string())
    );
    assert_eq!(
        runner_for_procedure(&settings, "adj-report"),
        Some("agy --dangerously-skip-permissions -i {prompt}".to_string())
    );

    assert_eq!(
        prompts::resolve_agent(
            None,
            None,
            runner_for_procedure(&settings, "adj-hub")
                .as_deref()
                .map(crate::runner::agent_from_runner)
                .as_deref()
        ),
        prompts::Agent::Claude
    );
    assert_eq!(
        prompts::resolve_agent(
            None,
            None,
            runner_for_procedure(&settings, "adj-worker")
                .as_deref()
                .map(crate::runner::agent_from_runner)
                .as_deref()
        ),
        prompts::Agent::Agy
    );
}

#[test]
fn runner_commands_resolve_to_the_agent_that_starts() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let resolve =
        |r: &str| prompts::resolve_agent(None, None, Some(&crate::runner::agent_from_runner(r)));
    for r in [
        "env FOO=1 agy -i {prompt}",
        "agy",
        "/opt/bin/agy -i {prompt}",
        "env A='x y' agy -i {prompt}",
    ] {
        assert_eq!(resolve(r), prompts::Agent::Agy, "{r}");
    }
    for r in [
        "claude --session-id {sessionId}",
        "env CLAUDE_CONFIG_DIR=/x claude",
    ] {
        assert_eq!(resolve(r), prompts::Agent::Claude, "{r}");
    }
}

#[test]
fn unsupported_agent_value_is_rejected() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let res = call_tool(
        "adjutant_skill",
        &json!({"name": "adj-hub", "agent": "unknown"}),
    );
    assert!(res.is_err());
    assert_eq!(
        res.unwrap_err(),
        "agent must be claude, agy, or generic: unknown"
    );

    let prompt_res = prompt_get(&json!({"name": "adj-hub", "arguments": {"agent": "unknown"}}));
    assert!(prompt_res.is_err());
    assert_eq!(
        prompt_res.unwrap_err(),
        "agent must be claude, agy, or generic: unknown"
    );
}

#[test]
fn skill_and_prompt_resolve_runner_for_specified_worktree() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let _guard = ClientNameGuard;
    let _sandbox = crate::testing::Sandbox::new(
        r#"{
        "defaults": {
            "agentRunner": "agy --dangerously-skip-permissions -i {prompt}"
        }
    }"#,
    );

    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "remote",
            "add",
            "origin",
            "git@github.com:acme/agy-repo.git",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(&args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success(),
            "git {args:?}"
        );
    }

    let worktree = dir.path().to_str().unwrap();

    let via_prompt = prompt_get(&json!({
        "name": "adj-worker",
        "arguments": { "worktree": worktree }
    }))
    .unwrap();
    let prompt_text = via_prompt["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(prompt_text.contains("ask_question"));
    assert!(!prompt_text.contains("AskUserQuestion"));

    let via_skill = call_tool(
        "adjutant_skill",
        &json!({
            "name": "adj-worker",
            "worktree": worktree
        }),
    )
    .unwrap();
    assert_eq!(via_skill["agent"], "agy");
    let skill_text = via_skill["content"].as_str().unwrap();
    assert!(skill_text.contains("ask_question"));
    assert!(!skill_text.contains("AskUserQuestion"));
}

#[test]
fn cli_skill_picks_the_agent_from_the_runner_without_an_override() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let _guard = ClientNameGuard;
    for runner in [
        "agy --dangerously-skip-permissions -i {prompt}",
        "env FOO=1 agy -i {prompt}",
    ] {
        let _sandbox = crate::testing::Sandbox::new(&format!(
            r#"{{ "defaults": {{ "agentRunner": "{runner}" }} }}"#
        ));
        let text = crate::cmd::skill_text("adj-worker", "", None).unwrap();
        assert!(text.contains("ask_question"), "{runner}");
        assert!(!text.contains("AskUserQuestion"), "{runner}");
    }
}

#[test]
fn install_validates_target_names() {
    assert!(install("unknown-agent").is_err());
    assert!(uninstall("unknown-agent").is_err());
}

#[test]
fn a_watcher_goes_on_until_this_process_serves_the_board() {
    let url = || "http://127.0.0.1:1/".to_string();
    assert_eq!(watch_mode(&Ok(HubBoard::Serving(url())), true), None);
    assert_eq!(
        watch_mode(&Ok(HubBoard::Resident(url())), false),
        Some(Mode::Resident)
    );
    assert_eq!(
        watch_mode(&Ok(HubBoard::AlreadyRunning), false),
        Some(Mode::Handover)
    );
    // An error at the start is said and got past; in a watcher it is tried again.
    assert_eq!(watch_mode(&Err("no".into()), false), None);
    assert_eq!(watch_mode(&Err("no".into()), true), Some(Mode::Resident));
}

#[test]
fn an_error_is_said_once_until_it_changes() {
    let mut last = None;
    assert!(is_new_error(&mut last, "a"));
    assert!(!is_new_error(&mut last, "a"));
    assert!(is_new_error(&mut last, "b"));
    assert!(is_new_error(&mut last, "a"));
}
