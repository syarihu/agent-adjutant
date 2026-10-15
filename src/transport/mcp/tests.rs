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
fn every_procedure_is_listed_and_fetchable() {
    let listed = call("prompts/list", json!({}));
    let names: Vec<&str> = listed["result"]["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "adj-hub",
            "adj-worker",
            "adj-report",
            "adj-review-hub",
            "adj-reviewer",
            "adj-aide"
        ]
    );

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
        // `adjutant_work`'s `repo` only narrows a machine-wide document to one repository; it
        // addresses no hub.
        if properties.get("repo").is_none() || tool["name"] == "adjutant_work" {
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
                .map(crate::kernel::runner::agent_from_runner)
                .as_deref()
        ),
        crate::infra::agent::Agent::Claude
    );
    assert_eq!(
        prompts::resolve_agent(
            None,
            None,
            runner_for_procedure(&settings, "adj-worker")
                .as_deref()
                .map(crate::kernel::runner::agent_from_runner)
                .as_deref()
        ),
        crate::infra::agent::Agent::Agy
    );
}

#[test]
fn runner_commands_resolve_to_the_agent_that_starts() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let resolve = |r: &str| {
        prompts::resolve_agent(
            None,
            None,
            Some(&crate::kernel::runner::agent_from_runner(r)),
        )
    };
    for r in [
        "env FOO=1 agy -i {prompt}",
        "agy",
        "/opt/bin/agy -i {prompt}",
        "env A='x y' agy -i {prompt}",
    ] {
        assert_eq!(resolve(r), crate::infra::agent::Agent::Agy, "{r}");
    }
    for r in [
        "claude --session-id {sessionId}",
        "env CLAUDE_CONFIG_DIR=/x claude",
    ] {
        assert_eq!(resolve(r), crate::infra::agent::Agent::Claude, "{r}");
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
        let text = crate::transport::wording::skill_text("adj-worker", "", None).unwrap();
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

#[test]
fn mcp_takes_the_agent_aliases_adj_skill_takes() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let codex = call_tool(
        "adjutant_skill",
        &json!({"name": "adj-hub", "agent": "codex"}),
    )
    .unwrap();
    assert_eq!(codex["agent"], "generic");
    assert!(
        !codex["content"]
            .as_str()
            .unwrap()
            .contains("AskUserQuestion")
    );

    let claude_code = call_tool(
        "adjutant_skill",
        &json!({"name": "adj-hub", "agent": "claude-code"}),
    )
    .unwrap();
    assert_eq!(claude_code["agent"], "claude");
    assert!(
        claude_code["content"]
            .as_str()
            .unwrap()
            .contains("AskUserQuestion")
    );

    let via_prompt =
        prompt_get(&json!({"name": "adj-hub", "arguments": {"agent": "antigravity"}})).unwrap();
    let text = via_prompt["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(!text.contains("AskUserQuestion"));
    assert!(text.contains("ask_question"));
}

#[test]
fn an_unknown_procedure_is_worded_per_entry_point() {
    let _lock = TEST_MUTEX.lock().unwrap();
    assert_eq!(
        prompt_get(&json!({"name": "nope"})).unwrap_err(),
        "Unknown prompt: nope"
    );
    assert_eq!(
        call_tool("adjutant_skill", &json!({"name": "nope"})).unwrap_err(),
        "no such procedure: nope (adj-hub / adj-worker / adj-report / adj-review-hub / adj-reviewer / adj-aide)"
    );
}

#[test]
fn every_tool_definition_names_the_entry_it_is_listed_under() {
    let mut seen = std::collections::HashSet::new();
    for tool in tools::TOOLS {
        assert_eq!((tool.definition)()["name"], tool.name);
        assert!(seen.insert(tool.name), "{} is listed twice", tool.name);
    }
}

#[test]
fn every_mcp_tool_the_aide_may_use_exists() {
    let prefix = format!("mcp__{}__", crate::kernel::runner::MCP_SERVER_NAME);
    for entry in crate::kernel::runner::AIDE_TOOLS {
        if let Some(name) = entry.strip_prefix(&prefix) {
            assert!(
                tools::TOOLS.iter().any(|tool| tool.name == name),
                "{entry} names no tool"
            );
        }
    }
}

#[test]
fn the_aide_is_allowed_without_asking_no_tool_that_writes_but_the_display() {
    // `AIDE_TOOLS` is what the aide may use without a prompt, on top of the person's own
    // permissions. A tool that writes or sends runs unasked once it is on this list, so it is
    // added by its own change, which relaxes this test with it. `adjutant_show` is the one that
    // writes, and only the aide's display state (see
    // `the_aide_s_tools_write_nothing_but_the_display_state`).
    for off in [
        "adjutant_send",
        "adjutant_tell",
        "adjutant_gate_open",
        "adjutant_gate_close",
        "adjutant_refresh",
        "adjutant_pending",
        "adjutant_outbox",
    ] {
        assert!(
            !crate::kernel::runner::AIDE_TOOLS
                .iter()
                .any(|t| t.ends_with(off)),
            "{off} is on the aide's list"
        );
    }
    // The tools of this server on the list are exactly these: one added without a change to this
    // test is a tool nobody decided the aide may run unasked.
    let prefix = format!("mcp__{}__", crate::kernel::runner::MCP_SERVER_NAME);
    let mut on: Vec<&str> = crate::kernel::runner::AIDE_TOOLS
        .iter()
        .filter_map(|t| t.strip_prefix(&prefix))
        .collect();
    on.sort();
    assert_eq!(
        on,
        [
            "adjutant_config",
            "adjutant_gate_read",
            "adjutant_hub_status",
            "adjutant_show",
            "adjutant_skill",
            "adjutant_work",
        ]
    );
}

/// Every file under `dir` with its contents, by path relative to it.
fn snapshot(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let name = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                out.insert(name, std::fs::read(&path).unwrap_or_default());
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// A board of `acme/widget` with one open gate on a task that has a parent. Hold the sandbox.
fn a_board_with_a_gate(sandbox: &crate::testing::Sandbox) -> (tempfile::TempDir, String) {
    let main = tempfile::tempdir().unwrap();
    let slug = crate::kernel::identity::slug_for("acme/widget", None);
    let repo = crate::kernel::identity::RepoInfo {
        main: main.path().to_string_lossy().to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: slug.clone(),
        hub_name: format!("adjutant-{slug}"),
        nwo_source: "dirname",
    };
    crate::registry::note_board(&sandbox.state(), &repo);
    let tasks = crate::task::dir(&sandbox.state(), &slug);
    std::fs::create_dir_all(&tasks).unwrap();
    let task = json!({
        "id": "t1", "kind": "start", "title": "Add a timer", "doneWhen": "pr",
        "autoStart": true, "status": "queued",
        "parent": "https://github.com/acme/widget/issues/40",
        "createdAt": "20260922T000000Z", "updatedAt": "20260922T000000Z",
    });
    std::fs::write(tasks.join("t1.json"), task.to_string()).unwrap();
    let gates = sandbox.state().join("gates").join(&slug);
    std::fs::create_dir_all(&gates).unwrap();
    let gate = json!({
        "id": "g1", "kind": "plan", "worktree": "/tmp/wt", "task": "t1",
        "title": "Design review", "facts": ["6 files to touch"],
        "focus": "How is the TTL held?", "openedAt": "20260922T010000Z",
    });
    std::fs::write(gates.join("g1.json"), gate.to_string()).unwrap();
    (main, slug)
}

#[test]
fn work_without_a_resident_says_how_to_start_one() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let sandbox = crate::testing::Sandbox::empty();
    let before = snapshot(&sandbox.state());
    for args in [json!({}), json!({"repo": "acme/widget"})] {
        let err = call_tool("adjutant_work", &args).unwrap_err();
        assert!(err.contains("adj server start"), "{err}");
    }
    assert_eq!(snapshot(&sandbox.state()), before);
}

#[test]
fn a_gate_is_read_in_full_by_its_board_and_id() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let sandbox = crate::testing::Sandbox::empty();
    let (_main, slug) = a_board_with_a_gate(&sandbox);
    let out = call_tool("adjutant_gate_read", &json!({"board": slug, "id": "g1"})).unwrap();
    assert_eq!(out["board"], slug.as_str());
    assert_eq!(out["repo"], "acme/widget");
    assert_eq!(out["gate"]["id"], "g1");
    assert_eq!(out["gate"]["title"], "Design review");
    assert_eq!(out["gate"]["facts"], json!(["6 files to touch"]));
    assert_eq!(out["gate"]["focus"], "How is the TTL held?");
    for args in [
        json!({"id": "g1"}),
        json!({"board": "  ", "id": "g1"}),
        json!({"board": slug}),
        json!({"board": slug, "id": ""}),
        json!({"board": "nope-0000000000000000", "id": "g1"}),
        json!({"board": "../boards/x", "id": "g1"}),
        json!({"board": slug, "id": "missing"}),
        json!({"board": slug, "id": "../tasks/t1"}),
    ] {
        assert!(call_tool("adjutant_gate_read", &args).is_err(), "{args}");
    }
}

#[test]
fn show_refuses_what_it_cannot_show_before_writing_anything() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let sandbox = crate::testing::Sandbox::empty();
    let (_main, slug) = a_board_with_a_gate(&sandbox);
    let before = snapshot(&sandbox.state());
    for (args, why) in [
        (json!({"kind": "pull"}), "kind must be gate"),
        (json!({}), "kind must be gate"),
        (json!({"kind": ""}), "kind must be gate"),
        (json!({"kind": "gate"}), "needs `board`"),
        (json!({"kind": "gate", "board": slug}), "needs `id`"),
        (
            json!({"kind": "gate", "board": slug, "id": "missing"}),
            "missing",
        ),
        (json!({"kind": "waits", "id": "g1"}), "takes no"),
        (
            json!({"kind": "gate", "board": slug, "id": "g1", "pr": "a/b#1"}),
            "takes no `pr`",
        ),
        (json!({"kind": "read"}), "kind=read needs `pr`"),
        (
            json!({"kind": "read", "pr": "a/b#1", "point": 1, "level": "info"}),
            "not both",
        ),
        (
            json!({"kind": "read", "pr": "a/b#1", "point": 0}),
            "whole number from 1",
        ),
        (
            json!({"kind": "read", "pr": "a/b#1", "point": 1.5}),
            "whole number from 1",
        ),
        (
            json!({"kind": "read", "pr": "a/b#1", "board": slug}),
            "takes no `board`",
        ),
        (json!({"kind": "read", "pr": "a/b#1"}), "no read-through"),
        (json!({"kind": "task", "board": slug}), "needs `id`"),
        (
            json!({"kind": "task", "board": slug, "id": "t1", "parent": "x"}),
            "not both",
        ),
        (json!({"kind": "task", "id": "t1"}), "needs `board`"),
        (
            json!({"kind": "task", "board": slug, "id": "t1", "path": "p"}),
            "takes no `path`",
        ),
        (json!({"kind": "pr"}), "kind=pr needs"),
        (json!({"kind": "pr", "board": slug}), "kind=pr needs"),
        (
            json!({"kind": "pr", "pr": "a/b#1", "board": slug, "id": "t1"}),
            "kind=pr needs",
        ),
        (json!({"kind": "pr", "pr": "a/b#1"}), "no record of a/b#1"),
        (
            json!({"kind": "pr", "board": slug, "id": "t1"}),
            "has no PR",
        ),
        (json!({"kind": "session", "board": slug}), "needs `id`"),
        (
            json!({"kind": "diff", "pr": "a/b#1", "from": 1, "to": 2}),
            "needs `path`",
        ),
        (
            json!({"kind": "diff", "pr": "a/b#1", "path": "p", "to": 2}),
            "needs `from`",
        ),
        (
            json!({"kind": "diff", "pr": "a/b#1", "path": "p", "from": 1}),
            "needs `to`",
        ),
        (
            json!({"kind": "diff", "path": "p", "from": 1, "to": 2}),
            "kind=diff needs",
        ),
        (
            json!({"kind": "diff", "pr": "a/b#1", "path": "p", "from": 1, "to": 2, "parent": "x"}),
            "takes no `parent`",
        ),
        (
            json!({"kind": "diff", "pr": "a/b#1", "path": "p", "from": 1, "to": 2}),
            "no diff stored",
        ),
    ] {
        let err = call_tool("adjutant_show", &args).unwrap_err();
        assert!(err.contains(why), "{args}: {err}");
    }
    assert_eq!(snapshot(&sandbox.state()), before);
}

#[test]
fn showing_a_read_through_and_a_pr_by_reference_writes_nothing_but_the_display_state() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let sandbox = crate::testing::Sandbox::empty();
    let others = sandbox.state().join("others");
    std::fs::create_dir_all(others.join("reads")).unwrap();
    let sha = "0123456789012345678901234567890123456789";
    let record = json!({
        "id": "acme/widget#7", "repo": "Acme/Widget", "number": 7,
        "url": "https://github.com/acme/widget/pull/7", "registered": true, "title": "Add a timer",
        "base": "main", "head": "f", "headSha": sha, "prState": "open",
        "additions": 1, "deletions": 1, "changedFiles": 1,
        "firstSeenAt": "20261001T000000Z", "readAt": "20261001T000000Z",
    });
    std::fs::write(others.join("acme~widget~7.json"), record.to_string()).unwrap();
    let read = json!({"id": "acme/widget#7", "madeAt": "20261001T000000Z", "sha": sha,
        "points": [{"level": "attention", "path": "a.rs", "line": 3, "title": "T", "why": "w"}]});
    std::fs::write(others.join("reads/acme~widget~7.json"), read.to_string()).unwrap();
    let before = snapshot(&sandbox.state());

    let shown = call_tool(
        "adjutant_show",
        &json!({"kind": "read", "pr": "acme/widget#7", "level": "attention"}),
    )
    .unwrap();
    assert_eq!(
        shown["parts"][0]["ref"],
        json!({"board": "", "id": "", "pr": "acme/widget#7", "level": "attention"})
    );
    assert_eq!(shown["readThrough"]["mode"], "read");
    assert_eq!(
        shown["readThrough"]["points"][0],
        json!({"n": 1, "level": "attention", "title": "T", "path": "a.rs", "line": 3})
    );
    let shown = call_tool(
        "adjutant_show",
        &json!({"kind": "pr", "pr": "https://github.com/Acme/Widget/pull/7"}),
    )
    .unwrap();
    assert_eq!(shown["parts"][0]["ref"]["pr"], "acme/widget#7");
    assert_eq!(shown["topic"]["key"], "acme/widget#pr:7");
    assert!(shown.get("readThrough").is_none());
    let after = snapshot(&sandbox.state());
    let mut changed: Vec<&String> = after
        .iter()
        .filter(|(path, bytes)| before.get(*path) != Some(*bytes))
        .map(|(path, _)| path)
        .collect();
    changed.sort();
    assert_eq!(changed, ["aide-display.json", "aide-display.lock"]);
}

#[test]
fn the_aide_s_tools_write_nothing_but_the_display_state() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let sandbox = crate::testing::Sandbox::empty();
    let (_main, slug) = a_board_with_a_gate(&sandbox);
    let before = snapshot(&sandbox.state());

    // The read tools leave the state directory as it was.
    call_tool("adjutant_gate_read", &json!({"board": slug, "id": "g1"})).unwrap();
    call_tool("adjutant_config", &json!({})).ok();
    assert!(call_tool("adjutant_work", &json!({})).is_err());
    assert_eq!(snapshot(&sandbox.state()), before);

    // `adjutant_show` adds the display and its lock, and changes nothing else: not the gate, not
    // the task, not the address book.
    let waits = call_tool("adjutant_show", &json!({"kind": "waits"})).unwrap();
    assert_eq!(waits["topic"]["key"], "waits");
    assert_eq!(waits["cleared"], false);
    let gate = call_tool(
        "adjutant_show",
        &json!({"kind": "gate", "board": slug, "id": "g1"}),
    )
    .unwrap();
    assert_eq!(gate["cleared"], true);
    assert_eq!(gate["topic"]["label"], "acme/widget · 親 #40");
    assert_eq!(gate["earlier"][0]["key"], "waits");
    assert_eq!(gate["parts"][0]["ref"], json!({"board": slug, "id": "g1"}));
    let after = snapshot(&sandbox.state());
    let mut changed: Vec<&String> = after
        .iter()
        .filter(|(path, bytes)| before.get(*path) != Some(*bytes))
        .map(|(path, _)| path)
        .collect();
    changed.sort();
    assert_eq!(changed, ["aide-display.json", "aide-display.lock"]);
    assert!(before.keys().all(|path| after.contains_key(path)));
}
