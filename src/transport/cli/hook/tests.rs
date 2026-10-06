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
    assert_eq!(hook("user-prompt-submit"), HookEvent::UserPromptSubmit);
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
    assert_eq!(hook("stop"), HookEvent::Stop);
    assert_eq!(hook("stop-failure"), HookEvent::StopFailure);
    assert_eq!(hook("session-end"), HookEvent::SessionEnd);
    let unknown = serde_json::json!({"session_id": "s", "hook_event_name": "PreCompact"});
    assert_eq!(
        event(&unknown).unwrap().hook,
        HookEvent::Other("PreCompact".to_string())
    );
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
