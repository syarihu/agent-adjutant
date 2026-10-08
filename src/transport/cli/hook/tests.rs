use super::*;
use crate::registry::AgentEvent;

fn fixture(name: &str) -> Value {
    let text = match name {
        "session-start" => include_str!("../../../fixtures/hooks/claude/session-start.json"),
        "user-prompt-submit" => {
            include_str!("../../../fixtures/hooks/claude/user-prompt-submit.json")
        }
        "post-tool-use" => include_str!("../../../fixtures/hooks/claude/post-tool-use.json"),
        "post-tool-use-failure" => {
            include_str!("../../../fixtures/hooks/claude/post-tool-use-failure.json")
        }
        "permission-request" => {
            include_str!("../../../fixtures/hooks/claude/permission-request.json")
        }
        "permission-request-ask-user-question" => {
            include_str!("../../../fixtures/hooks/claude/permission-request-ask-user-question.json")
        }
        "notification-permission" => {
            include_str!("../../../fixtures/hooks/claude/notification-permission.json")
        }
        "notification-idle" => {
            include_str!("../../../fixtures/hooks/claude/notification-idle.json")
        }
        "subagent-start" => include_str!("../../../fixtures/hooks/claude/subagent-start.json"),
        "subagent-post-tool-use" => {
            include_str!("../../../fixtures/hooks/claude/subagent-post-tool-use.json")
        }
        "subagent-stop" => include_str!("../../../fixtures/hooks/claude/subagent-stop.json"),
        "stop" => include_str!("../../../fixtures/hooks/claude/stop.json"),
        "stop-failure" => include_str!("../../../fixtures/hooks/claude/stop-failure.json"),
        "session-end" => include_str!("../../../fixtures/hooks/claude/session-end.json"),
        "status-line" => include_str!("../../../fixtures/hooks/claude/status-line.json"),
        other => panic!("no fixture called {other}"),
    };
    serde_json::from_str(text).unwrap()
}

fn event(payload: &Value) -> Result<AgentEvent, String> {
    claude_event(payload, Some("4242"), None, Path::new("/home/user"), 1_000)
}

#[test]
fn each_hook_name_is_the_event_it_says() {
    let hook = |name: &str| event(&fixture(name)).unwrap().hook;
    assert_eq!(hook("session-start"), HookEvent::SessionStart);
    assert_eq!(
        hook("user-prompt-submit"),
        HookEvent::UserPromptSubmit { typed: true }
    );
    assert_eq!(hook("post-tool-use"), HookEvent::PostToolUse);
    assert_eq!(hook("post-tool-use-failure"), HookEvent::PostToolUseFailure);
    assert_eq!(hook("permission-request"), HookEvent::PermissionRequest);
    assert_eq!(
        hook("notification-permission"),
        HookEvent::Notification {
            kind: Some("permission_prompt".to_string()),
            message: Some("Claude needs your permission to use Bash".to_string()),
        }
    );
    assert!(matches!(
        hook("notification-idle"),
        HookEvent::Notification { kind: Some(kind), .. } if kind == "idle_prompt"
    ));
    assert_eq!(hook("subagent-start"), HookEvent::SubagentStart);
    assert_eq!(hook("subagent-stop"), HookEvent::SubagentStop);
    assert_eq!(
        hook("stop"),
        HookEvent::Stop {
            message: Some("The test passes now.".to_string())
        }
    );
    assert_eq!(
        hook("stop-failure"),
        HookEvent::StopFailure {
            message: Some("API Error: Rate limit reached".to_string())
        }
    );
    assert_eq!(hook("session-end"), HookEvent::SessionEnd);
    let unknown = serde_json::json!({"session_id": "s", "hook_event_name": "PreCompact"});
    assert_eq!(
        event(&unknown).unwrap().hook,
        HookEvent::Other("PreCompact".to_string())
    );
}

/// The Claude `stop` fixture with `last_assistant_message` set to `message`, or removed.
fn stop_saying(message: Option<&str>) -> Value {
    let mut payload = fixture("stop");
    match message {
        Some(message) => payload["last_assistant_message"] = Value::from(message),
        None => {
            payload
                .as_object_mut()
                .unwrap()
                .remove("last_assistant_message");
        }
    }
    payload
}

fn stop_message(payload: &Value) -> Option<String> {
    match event(payload).unwrap().hook {
        HookEvent::Stop { message } => message,
        other => panic!("not a stop: {other:?}"),
    }
}

#[test]
fn the_last_message_is_trimmed_and_blank_or_missing_is_none() {
    assert_eq!(
        stop_message(&stop_saying(Some("  \n Fixed it.\n"))).as_deref(),
        Some("Fixed it.")
    );
    assert_eq!(stop_message(&stop_saying(Some(" \n\t "))), None);
    assert_eq!(stop_message(&stop_saying(None)), None);
    let mut not_text = stop_saying(None);
    not_text["last_assistant_message"] = serde_json::json!(5);
    assert_eq!(stop_message(&not_text), None);
}

#[test]
fn a_long_last_message_is_cut_on_a_character_boundary_and_keeps_its_line_breaks() {
    let long = format!("one\n\ntwo\n{}", "あ".repeat(LAST_MESSAGE_CHARS));
    let message = stop_message(&stop_saying(Some(&long))).unwrap();
    assert!(message.starts_with("one\n\ntwo\n"), "{message:?}");
    assert!(message.ends_with("あ…"));
    assert_eq!(message.chars().count(), LAST_MESSAGE_CHARS + 1);
    // Exactly the limit is not cut.
    let exact = "x".repeat(LAST_MESSAGE_CHARS);
    assert_eq!(stop_message(&stop_saying(Some(&exact))), Some(exact));
}

#[test]
fn a_sub_agent_stop_gives_the_session_no_message() {
    let stop = event(&fixture("subagent-stop")).unwrap();
    assert_eq!(stop.hook, HookEvent::SubagentStop);
    let stop = codex(&codex_fixture("subagent-stop")).unwrap();
    assert_eq!(stop.hook, HookEvent::SubagentStop);
}

#[test]
fn the_payload_gives_the_session_the_place_and_the_sub_agent() {
    let main = event(&fixture("session-start")).unwrap();
    assert_eq!(main.session_id, "0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d");
    assert_eq!(main.cwd.as_deref(), Some("/home/user/repo"));
    assert_eq!((main.agent_id, main.agent_type), (None, None));
    assert_eq!(
        (main.agent.as_str(), main.pid, main.at),
        ("claude", Some(4242), 1_000)
    );

    let sub = event(&fixture("subagent-post-tool-use")).unwrap();
    assert_eq!(sub.agent_id.as_deref(), Some("aac91a24906eff6d1"));
    assert_eq!(sub.agent_type.as_deref(), Some("general-purpose"));
    assert_eq!(sub.summary.as_deref(), Some("Bash: ls"));
}

#[test]
fn a_tool_is_named_with_the_first_thing_its_input_names() {
    let summary = |name: &str| event(&fixture(name)).unwrap().summary;
    assert_eq!(
        summary("post-tool-use").as_deref(),
        Some("Edit: /home/user/repo/src/lib.rs")
    );
    // The command and not its description, and only the first line of it.
    assert_eq!(
        summary("post-tool-use-failure").as_deref(),
        Some("Bash: cargo test")
    );
    assert_eq!(summary("session-start"), None);
    // A question names no file or command: its first line is what is asked.
    assert_eq!(
        summary("permission-request-ask-user-question").as_deref(),
        Some("AskUserQuestion: Which fruit?")
    );

    let payload = |input: Value| {
        let mut p = fixture("post-tool-use");
        p["tool_input"] = input;
        event(&p).unwrap().summary
    };
    assert_eq!(
        payload(serde_json::json!({"command": "\n  cargo test\nmore"})).as_deref(),
        Some("Edit: cargo test")
    );
    assert_eq!(
        payload(serde_json::json!({"x": 1})).as_deref(),
        Some("Edit")
    );
    assert_eq!(
        payload(serde_json::json!({"file_path": "  "})).as_deref(),
        Some("Edit")
    );
    assert_eq!(payload(Value::Null).as_deref(), Some("Edit"));
    assert_eq!(
        payload(serde_json::json!({"questions": [{"question": "\n First?\nsecond"}]})).as_deref(),
        Some("Edit: First?")
    );
    assert_eq!(
        payload(serde_json::json!({"questions": []})).as_deref(),
        Some("Edit")
    );
    // Cut on a character boundary.
    let long = "é".repeat(200);
    let cut = payload(serde_json::json!({ "file_path": long })).unwrap();
    assert_eq!(cut.chars().count(), SHOWN_CHARS);
    assert!(cut.starts_with("Edit: éé"));
}

#[test]
fn blank_values_are_absent_and_a_bad_session_id_is_refused() {
    let mut payload = fixture("post-tool-use");
    payload["cwd"] = serde_json::json!("  ");
    payload["agent_id"] = serde_json::json!("");
    let e = event(&payload).unwrap();
    assert_eq!((e.cwd, e.agent_id), (None, None));

    for id in [
        serde_json::json!(""),
        serde_json::json!("../x"),
        serde_json::json!("a/b"),
        Value::Null,
    ] {
        payload["session_id"] = id.clone();
        assert!(event(&payload).is_err(), "{id}");
    }
    payload["session_id"] = serde_json::json!("ok");
    payload.as_object_mut().unwrap().remove("hook_event_name");
    assert!(event(&payload).is_err());
}

#[test]
fn the_pid_is_a_positive_number_and_the_config_dir_falls_back_to_the_default_account() {
    let payload = fixture("stop");
    let at = |pid: Option<&str>, dir: Option<&str>| {
        claude_event(&payload, pid, dir, Path::new("/home/user"), 1).unwrap()
    };
    assert_eq!(at(Some(" 77 "), None).pid, Some(77));
    for bad in [Some("0"), Some("-3"), Some("x"), Some(""), None] {
        assert_eq!(at(bad, None).pid, None, "{bad:?}");
    }
    assert_eq!(
        at(None, None).config_dir.as_deref(),
        Some("/home/user/.claude")
    );
    assert_eq!(
        at(None, Some("  ")).config_dir.as_deref(),
        Some("/home/user/.claude")
    );
    assert_eq!(
        at(None, Some("/cfg/work")).config_dir.as_deref(),
        Some("/cfg/work")
    );
}

fn status_line(payload: &Value) -> Result<Option<AgentEvent>, String> {
    claude_status_line(payload, 1_000)
}

fn figures(payload: &Value) -> Option<HookEvent> {
    status_line(payload).unwrap().map(|event| event.hook)
}

fn window(used: f64, resets_at: i64) -> RateWindow {
    RateWindow {
        used_percent: Some(used),
        resets_at: Some(resets_at),
        other: serde_json::Map::new(),
    }
}

#[test]
fn a_status_line_payload_is_the_figures_it_shows() {
    let event = status_line(&fixture("status-line")).unwrap().unwrap();
    assert_eq!(event.session_id, "0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d");
    assert_eq!((event.agent.as_str(), event.at), ("claude", 1_000));
    assert_eq!(event.cwd, None);
    assert_eq!(
        event.hook,
        HookEvent::StatusLine {
            model: Some("Opus".to_string()),
            context_percent: Some(43.0),
            five_hour: Some(window(23.5, 1_767_225_600)),
            seven_day: Some(window(61.0, 1_767_744_000)),
        }
    );
}

#[test]
fn a_status_line_without_figures_records_nothing_and_blanks_are_absent() {
    assert_eq!(figures(&fixture("stop")), None);

    let with = |edit: &dyn Fn(&mut Value)| {
        let mut p = fixture("status-line");
        edit(&mut p);
        figures(&p)
    };
    let only_model = |event: Option<HookEvent>| match event {
        Some(HookEvent::StatusLine {
            model,
            context_percent: None,
            five_hour: None,
            seven_day: None,
        }) => model,
        other => panic!("not just a model: {other:?}"),
    };
    // Each field alone: null, blank, a string, a negative number are all absent.
    for bad in [
        serde_json::json!(null),
        serde_json::json!(""),
        serde_json::json!("43"),
        serde_json::json!(-1.0),
    ] {
        let kept = with(&|p| {
            p["context_window"]["used_percentage"] = bad.clone();
            p["rate_limits"]["five_hour"]["used_percentage"] = bad.clone();
            p["rate_limits"]["seven_day"] = bad.clone();
        });
        assert_eq!(only_model(kept).as_deref(), Some("Opus"));
    }
    // A window with no resets_at still has its percentage.
    let open = with(&|p| {
        p["rate_limits"]["five_hour"]
            .as_object_mut()
            .unwrap()
            .remove("resets_at");
    });
    assert!(matches!(
        open,
        Some(HookEvent::StatusLine { five_hour: Some(RateWindow { used_percent: Some(u), resets_at: None, .. }), .. }) if u == 23.5
    ));

    // The model: the id when the name is blank, the string as it is, nothing for blank.
    let model = |value: Value| {
        let mut p = fixture("status-line");
        p["model"] = value;
        p["context_window"] = Value::Null;
        p["rate_limits"] = Value::Null;
        figures(&p)
    };
    assert_eq!(
        only_model(model(
            serde_json::json!({"id": "claude-x", "display_name": " "})
        ))
        .as_deref(),
        Some("claude-x")
    );
    assert_eq!(
        only_model(model(serde_json::json!("opus-4"))).as_deref(),
        Some("opus-4")
    );
    assert_eq!(model(serde_json::json!("  ")), None);
    assert_eq!(model(serde_json::json!({})), None);
}

#[test]
fn a_status_line_with_a_bad_session_id_is_refused() {
    for id in [
        serde_json::json!(""),
        serde_json::json!("../x"),
        serde_json::json!("a/b"),
        serde_json::json!(null),
    ] {
        let mut p = fixture("status-line");
        p["session_id"] = id;
        assert!(status_line(&p).is_err());
    }
}

fn codex_fixture(name: &str) -> Value {
    let text = match name {
        "session-start" => include_str!("../../../fixtures/hooks/codex/session-start.json"),
        "user-prompt-submit" => {
            include_str!("../../../fixtures/hooks/codex/user-prompt-submit.json")
        }
        "post-tool-use" => include_str!("../../../fixtures/hooks/codex/post-tool-use.json"),
        "permission-request" => {
            include_str!("../../../fixtures/hooks/codex/permission-request.json")
        }
        "subagent-start" => include_str!("../../../fixtures/hooks/codex/subagent-start.json"),
        "subagent-post-tool-use" => {
            include_str!("../../../fixtures/hooks/codex/subagent-post-tool-use.json")
        }
        "subagent-stop" => include_str!("../../../fixtures/hooks/codex/subagent-stop.json"),
        "stop" => include_str!("../../../fixtures/hooks/codex/stop.json"),
        "interrupt" => include_str!("../../../fixtures/hooks/codex/interrupt.json"),
        "session-end" => include_str!("../../../fixtures/hooks/codex/session-end.json"),
        other => panic!("no codex fixture called {other}"),
    };
    serde_json::from_str(text).unwrap()
}

fn codex(payload: &Value) -> Result<AgentEvent, String> {
    codex_event(payload, None, Path::new("/home/user"), 1_000)
}

#[test]
fn each_codex_hook_name_is_the_event_it_says_and_an_interrupt_is_a_stop() {
    let hook = |name: &str| codex(&codex_fixture(name)).unwrap().hook;
    assert_eq!(hook("session-start"), HookEvent::SessionStart);
    assert_eq!(
        hook("user-prompt-submit"),
        HookEvent::UserPromptSubmit { typed: true }
    );
    assert_eq!(hook("post-tool-use"), HookEvent::PostToolUse);
    assert_eq!(hook("permission-request"), HookEvent::PermissionRequest);
    assert_eq!(hook("subagent-start"), HookEvent::SubagentStart);
    assert_eq!(hook("subagent-post-tool-use"), HookEvent::PostToolUse);
    assert_eq!(hook("subagent-stop"), HookEvent::SubagentStop);
    assert_eq!(
        hook("stop"),
        HookEvent::Stop {
            message: Some("Done.".to_string())
        }
    );
    // Cut short: there is nothing it said.
    assert_eq!(hook("interrupt"), HookEvent::Stop { message: None });
    assert_eq!(hook("session-end"), HookEvent::SessionEnd);
    let unknown = serde_json::json!({"session_id": "s", "hook_event_name": "PreCompact"});
    assert_eq!(
        codex(&unknown).unwrap().hook,
        HookEvent::Other("PreCompact".to_string())
    );
}

#[test]
fn a_codex_event_has_no_pid_and_names_the_session_the_place_and_the_sub_agent() {
    let main = codex(&codex_fixture("session-start")).unwrap();
    assert_eq!(main.agent, "codex");
    assert_eq!(main.pid, None);
    assert_eq!(main.session_id, "0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d");
    assert_eq!(main.cwd.as_deref(), Some("/work/repo"));
    assert_eq!((main.agent_id, main.agent_type), (None, None));

    let sub = codex(&codex_fixture("subagent-post-tool-use")).unwrap();
    assert_eq!(sub.agent_id.as_deref(), Some("agent-example-1"));
    assert_eq!(sub.agent_type.as_deref(), Some("worker"));
    assert_eq!(sub.summary.as_deref(), Some("Bash: ls"));
    assert_eq!(
        codex(&codex_fixture("post-tool-use"))
            .unwrap()
            .summary
            .as_deref(),
        Some("Bash: cargo test")
    );
}

#[test]
fn the_codex_config_dir_falls_back_to_the_default_install() {
    let payload = codex_fixture("stop");
    let at = |dir: Option<&str>| {
        codex_event(&payload, dir, Path::new("/home/user"), 1)
            .unwrap()
            .config_dir
    };
    assert_eq!(at(None).as_deref(), Some("/home/user/.codex"));
    assert_eq!(at(Some("  ")).as_deref(), Some("/home/user/.codex"));
    assert_eq!(at(Some("/cx")).as_deref(), Some("/cx"));
    let mut bad = payload;
    bad["session_id"] = serde_json::json!("../x");
    assert!(codex(&bad).is_err());
}

#[test]
fn a_prompt_is_typed_unless_it_is_a_wake_line_or_absent() {
    use crate::infra::terminal::{HUB_WAKE_LINE, WORKER_WAKE_LINE};
    for (event, fixture) in [
        (
            event as fn(&Value) -> Result<AgentEvent, String>,
            fixture("user-prompt-submit"),
        ),
        (codex, codex_fixture("user-prompt-submit")),
    ] {
        let hook = |prompt: Option<&str>| {
            let mut payload = fixture.clone();
            match prompt {
                Some(prompt) => payload["prompt"] = Value::String(prompt.to_string()),
                None => {
                    payload.as_object_mut().unwrap().remove("prompt");
                }
            }
            event(&payload).unwrap().hook
        };
        assert_eq!(hook(None), HookEvent::UserPromptSubmit { typed: false });
        assert_eq!(
            hook(Some("  ")),
            HookEvent::UserPromptSubmit { typed: false }
        );
        assert_eq!(
            hook(Some("hello")),
            HookEvent::UserPromptSubmit { typed: true }
        );
        for line in [WORKER_WAKE_LINE, HUB_WAKE_LINE] {
            assert_eq!(
                hook(Some(line)),
                HookEvent::UserPromptSubmit { typed: false }
            );
        }
    }
}
