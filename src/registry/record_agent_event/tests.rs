use super::*;
use crate::registry::store::read_agent_session;
use std::collections::HashMap;

const T0: i64 = 1_000_000;

fn event(hook: HookEvent, at: i64) -> AgentEvent {
    AgentEvent {
        agent: "claude".to_string(),
        session_id: "s1".to_string(),
        hook,
        agent_id: None,
        agent_type: None,
        cwd: None,
        summary: None,
        pid: None,
        config_dir: None,
        at,
    }
}

fn tool(at: i64, summary: &str) -> AgentEvent {
    AgentEvent {
        summary: Some(summary.to_string()),
        ..event(HookEvent::PostToolUse, at)
    }
}

fn notification(kind: Option<&str>, at: i64) -> AgentEvent {
    event(
        HookEvent::Notification {
            kind: kind.map(str::to_string),
            message: Some("Claude needs your permission".to_string()),
        },
        at,
    )
}

/// The same event as a sub-agent `id` sent it.
fn from_sub(id: &str, event: AgentEvent) -> AgentEvent {
    AgentEvent {
        agent_id: Some(id.to_string()),
        agent_type: Some("Explore".to_string()),
        ..event
    }
}

fn started(id: &str, at: i64) -> AgentEvent {
    from_sub(id, event(HookEvent::SubagentStart, at))
}

fn stopped(id: &str, at: i64) -> AgentEvent {
    from_sub(id, event(HookEvent::SubagentStop, at))
}

/// Apply `e` to `row` with nothing looked up (events here carry no cwd and no pid), and say
/// whether the row was written or removed.
fn step_keep(row: &mut Option<AgentSession>, e: &AgentEvent) -> bool {
    match apply(row.clone(), e, &Lookups::default()) {
        Applied::Write(next) => {
            *row = Some(*next);
            true
        }
        Applied::Remove => {
            *row = None;
            true
        }
        Applied::Unchanged => false,
        Applied::NeedsLookup => panic!("needs a lookup"),
    }
}

fn rows(events: &[AgentEvent]) -> Option<AgentSession> {
    let mut row = None;
    for e in events {
        step_keep(&mut row, e);
    }
    row
}

fn status(row: &Option<AgentSession>) -> Option<&str> {
    row.as_ref()?.status.as_ref().map(AgentStatus::as_str)
}

fn no_processes() -> ProcessTable {
    ProcessTable::fixed(Some(HashMap::new()))
}

fn record(root: &Path, e: &AgentEvent) {
    record_agent_event_with(root, e, &no_processes()).unwrap();
}

fn read(root: &Path, id: &str) -> AgentSession {
    match read_agent_session(&agent_session_path(root, id)) {
        Recorded::Found(row) => row,
        _ => panic!("no row for {id}"),
    }
}

fn exists(root: &Path, id: &str) -> bool {
    agent_session_path(root, id).exists()
}

fn put(root: &Path, row: &AgentSession) {
    write_agent_session(root, row).unwrap();
}

fn row_of(id: &str, pid: Option<u32>, last_event_at: Option<i64>) -> AgentSession {
    AgentSession {
        session_id: id.to_string(),
        status: Some(AgentStatus::Running),
        pid,
        created_at: last_event_at,
        last_event_at,
        ..AgentSession::default()
    }
}

// ── which events make a row ──────────────────────────────────────

#[test]
fn a_stray_end_of_a_session_with_no_row_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    for hook in [
        HookEvent::Stop,
        HookEvent::StopFailure,
        HookEvent::SessionEnd,
        HookEvent::SubagentStop,
    ] {
        record(dir.path(), &from_sub("a1", event(hook, T0)));
    }
    record(dir.path(), &notification(Some("idle_prompt"), T0));
    record(dir.path(), &notification(Some("auth_success"), T0));
    record(
        dir.path(),
        &event(HookEvent::Other("PreCompact".into()), T0),
    );
    assert!(!agent_sessions_dir(dir.path()).exists());
}

#[test]
fn session_start_makes_an_idle_row_and_a_prompt_a_running_one() {
    let dir = tempfile::tempdir().unwrap();
    record(dir.path(), &event(HookEvent::SessionStart, T0));
    let row = read(dir.path(), "s1");
    assert_eq!(row.status, Some(AgentStatus::Idle));
    assert_eq!(row.agent.as_deref(), Some("claude"));
    assert_eq!(
        (row.created_at, row.updated_at, row.last_event_at),
        (Some(T0), Some(T0), Some(T0))
    );

    let other = tempfile::tempdir().unwrap();
    record(other.path(), &event(HookEvent::UserPromptSubmit, T0));
    assert_eq!(read(other.path(), "s1").status, Some(AgentStatus::Running));
}

#[test]
fn session_start_on_a_waiting_row_clears_only_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let ask = AgentEvent {
        summary: Some("Bash: rm -rf x".to_string()),
        ..event(HookEvent::PermissionRequest, T0)
    };
    record(dir.path(), &ask);
    assert_eq!(
        read(dir.path(), "s1").request.as_deref(),
        Some("Bash: rm -rf x")
    );

    // The pid is new to the row, so the start time comes in with the event.
    let table = ProcessTable::fixed(Some(HashMap::from([(
        4242,
        "Mon Jan  1 00:00:00 2024".to_string(),
    )])));
    let resumed = AgentEvent {
        pid: Some(4242),
        ..event(HookEvent::SessionStart, T0 + 5)
    };
    record_agent_event_with(dir.path(), &resumed, &table).unwrap();
    let row = read(dir.path(), "s1");
    assert_eq!(row.status, Some(AgentStatus::Waiting));
    assert_eq!(row.request, None);
    assert_eq!(row.updated_at, Some(T0));
    assert_eq!(row.pid, Some(4242));
    assert_eq!(row.ps_started.as_deref(), Some("Mon Jan  1 00:00:00 2024"));
}

// ── what is written, and how often ───────────────────────────────

#[test]
fn a_tool_call_sets_the_activity_and_the_same_one_is_not_written_again_within_a_minute() {
    let dir = tempfile::tempdir().unwrap();
    record(dir.path(), &tool(T0, "Edit: src/lib.rs"));
    assert_eq!(
        read(dir.path(), "s1").activity.as_deref(),
        Some("Edit: src/lib.rs")
    );
    let path = agent_session_path(dir.path(), "s1");
    let written = std::fs::read(&path).unwrap();

    record(dir.path(), &tool(T0 + 59, "Edit: src/lib.rs"));
    assert_eq!(std::fs::read(&path).unwrap(), written);

    record(dir.path(), &tool(T0 + 61, "Edit: src/lib.rs"));
    let row = read(dir.path(), "s1");
    assert_eq!(row.last_event_at, Some(T0 + 61));
    assert_eq!(row.updated_at, Some(T0));
}

#[test]
fn updated_at_moves_only_when_the_status_does() {
    let mut row = None;
    step_keep(&mut row, &tool(T0, "Edit: a"));
    step_keep(&mut row, &tool(T0 + 100, "Edit: b"));
    let at = |row: &Option<AgentSession>| row.as_ref().map(|r| (r.updated_at, r.last_event_at));
    assert_eq!(at(&row), Some((Some(T0), Some(T0 + 100))));
    step_keep(&mut row, &event(HookEvent::PermissionRequest, T0 + 200));
    assert_eq!(at(&row), Some((Some(T0 + 200), Some(T0 + 200))));
    step_keep(&mut row, &event(HookEvent::Stop, T0 + 300));
    assert_eq!(status(&row), Some("done"));
    assert_eq!(at(&row), Some((Some(T0 + 300), Some(T0 + 300))));
    let done = row.as_ref().unwrap();
    assert_eq!(
        (done.activity.as_deref(), done.request.as_deref()),
        (None, None)
    );
}

// ── a turn that ends while sub-agents run ────────────────────────

#[test]
fn done_is_held_until_the_last_subagent_stops() {
    let mut row = rows(&[started("a1", T0), started("a2", T0 + 1)]);
    assert_eq!(status(&row), Some("running"));
    step_keep(&mut row, &event(HookEvent::Stop, T0 + 10));
    let held = row.clone().unwrap();
    assert_eq!(held.status, Some(AgentStatus::Running));
    assert_eq!(held.pending_status, Some(AgentStatus::Done));
    step_keep(&mut row, &stopped("a1", T0 + 20));
    assert_eq!(status(&row), Some("running"));
    step_keep(&mut row, &stopped("a2", T0 + 30));
    let done = row.unwrap();
    assert_eq!(done.status, Some(AgentStatus::Done));
    assert_eq!(done.pending_status, None);
    assert!(done.subagents.is_empty());
    assert_eq!(done.updated_at, Some(T0 + 30));
}

#[test]
fn a_new_turn_of_the_parent_drops_what_was_held_and_later_stops_leave_it_running() {
    for turn in [
        tool(T0 + 6, "Edit: a"),
        event(HookEvent::PostToolUseFailure, T0 + 6),
        event(HookEvent::PermissionRequest, T0 + 6),
    ] {
        let mut row = rows(&[started("a1", T0), event(HookEvent::Stop, T0 + 5), turn]);
        assert_eq!(row.as_ref().unwrap().pending_status, None);
        step_keep(&mut row, &stopped("a1", T0 + 9));
        assert_ne!(status(&row), Some("done"));
    }
}

#[test]
fn an_event_that_names_a_subagent_but_is_not_its_work_changes_nothing() {
    for hook in [
        HookEvent::SessionStart,
        HookEvent::UserPromptSubmit,
        HookEvent::Stop,
    ] {
        let mut row = rows(&[tool(T0, "Edit: a")]);
        let before = row.clone();
        assert!(!step_keep(&mut row, &from_sub("a1", event(hook, T0 + 5))));
        assert_eq!(row, before);
    }
}

#[test]
fn a_start_time_that_could_not_be_read_is_asked_for_again() {
    let dir = tempfile::tempdir().unwrap();
    let e = AgentEvent {
        pid: Some(4242),
        ..tool(T0, "Edit: a")
    };
    record_agent_event_with(dir.path(), &e, &ProcessTable::fixed(None)).unwrap();
    assert_eq!(read(dir.path(), "s1").ps_started, None);
    let table = ProcessTable::fixed(Some(HashMap::from([(
        4242,
        "Mon Jan  1 00:00:00 2024".to_string(),
    )])));
    let again = AgentEvent { at: T0 + 5, ..e };
    record_agent_event_with(dir.path(), &again, &table).unwrap();
    let row = read(dir.path(), "s1");
    assert_eq!(row.ps_started.as_deref(), Some("Mon Jan  1 00:00:00 2024"));
    assert_eq!(row.pid, Some(4242));
}

#[cfg(unix)]
#[test]
fn a_row_is_readable_by_its_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    record(dir.path(), &tool(T0, "Bash: echo secret"));
    let mode = std::fs::metadata(agent_session_path(dir.path(), "s1"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_failure_is_held_the_same_way_and_a_new_prompt_drops_what_was_held() {
    let mut row = rows(&[started("a1", T0), event(HookEvent::StopFailure, T0 + 5)]);
    assert_eq!(
        row.as_ref().unwrap().pending_status,
        Some(AgentStatus::Failed)
    );
    step_keep(&mut row, &stopped("a1", T0 + 6));
    assert_eq!(status(&row), Some("failed"));

    let mut row = rows(&[started("a1", T0), event(HookEvent::Stop, T0 + 5)]);
    step_keep(&mut row, &event(HookEvent::UserPromptSubmit, T0 + 6));
    let row = row.unwrap();
    assert_eq!(row.pending_status, None);
    assert_eq!(row.status, Some(AgentStatus::Running));
}

// ── events a sub-agent sends ─────────────────────────────────────

#[test]
fn a_permission_request_from_a_subagent_makes_the_parent_wait_and_its_next_tool_call_ends_it() {
    let ask = from_sub(
        "a1",
        AgentEvent {
            summary: Some("Bash: make".to_string()),
            ..event(HookEvent::PermissionRequest, T0 + 10)
        },
    );
    let mut row = rows(&[tool(T0, "Edit: a"), ask]);
    let waiting = row.clone().unwrap();
    assert_eq!(waiting.status, Some(AgentStatus::Waiting));
    assert_eq!(waiting.request.as_deref(), Some("Bash: make"));
    assert_eq!(waiting.subagents.len(), 1);

    step_keep(&mut row, &from_sub("a1", tool(T0 + 20, "Bash: make")));
    let running = row.unwrap();
    assert_eq!(running.status, Some(AgentStatus::Running));
    assert_eq!(running.request, None);
}

#[test]
fn a_subagents_tool_call_leaves_a_running_parents_status_and_activity_alone() {
    let row = rows(&[
        tool(T0, "Edit: a"),
        from_sub("a1", tool(T0 + 100, "Bash: b")),
    ])
    .unwrap();
    assert_eq!(row.status, Some(AgentStatus::Running));
    assert_eq!(row.activity.as_deref(), Some("Edit: a"));
    assert_eq!(row.subagents.len(), 1);
    assert_eq!(row.subagents[0].kind.as_deref(), Some("Explore"));
    assert_eq!(row.updated_at, Some(T0));
}

#[test]
fn a_subagent_working_elsewhere_does_not_move_the_sessions_cwd_or_worktree() {
    let here = AgentEvent {
        cwd: Some("/home/user/repo".to_string()),
        ..event(HookEvent::SessionStart, T0)
    };
    let lookups = Lookups {
        worktree: Some((
            "/home/user/repo".to_string(),
            Some("/home/user/repo".to_string()),
        )),
        started: None,
    };
    let Applied::Write(row) = apply(None, &here, &lookups) else {
        panic!("a new row is written");
    };
    // No lookup is asked for the other directory, and none is needed to apply it.
    let there = AgentEvent {
        cwd: Some("/home/user/repo-other".to_string()),
        ..from_sub("a1", tool(T0 + 5, "Bash: ls"))
    };
    assert_eq!(wanted(Some(&row), &there), Wanted::default());
    let Applied::Write(next) = apply(Some(*row), &there, &Lookups::default()) else {
        panic!("a sub-agent's first event is written");
    };
    assert_eq!(next.cwd.as_deref(), Some("/home/user/repo"));
    assert_eq!(next.worktree.as_deref(), Some("/home/user/repo"));
    assert_eq!(next.subagents.len(), 1);
}

#[test]
fn an_event_that_needs_no_row_makes_none() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nowhere");
    for hook in [HookEvent::SessionEnd, HookEvent::Other("PreCompact".into())] {
        let e = AgentEvent {
            cwd: Some(missing.to_string_lossy().to_string()),
            pid: Some(4242),
            ..event(hook, T0)
        };
        // Neither makes a row, so the directory that does not exist is never asked about.
        record(dir.path(), &e);
        assert!(!agent_sessions_dir(dir.path()).exists());
    }
    record(dir.path(), &tool(T0, "Edit: a"));
    record(dir.path(), &event(HookEvent::SessionEnd, T0 + 1));
    assert!(!exists(dir.path(), "s1"));
}

#[test]
fn an_event_that_only_names_a_type_is_not_from_a_subagent() {
    let typed = AgentEvent {
        agent_type: Some("Explore".to_string()),
        ..tool(T0, "Edit: a")
    };
    assert!(rows(&[typed]).unwrap().subagents.is_empty());
}

#[test]
fn a_subagents_heartbeat_moves_at_most_once_a_minute() {
    let mut row = rows(&[started("a1", T0)]);
    step_keep(&mut row, &from_sub("a1", tool(T0 + 30, "Bash: b")));
    assert_eq!(row.as_ref().unwrap().subagents[0].last_seen_at, Some(T0));
    step_keep(&mut row, &from_sub("a1", tool(T0 + 60, "Bash: b")));
    assert_eq!(
        row.as_ref().unwrap().subagents[0].last_seen_at,
        Some(T0 + 60)
    );
    assert!(!step_keep(
        &mut row,
        &from_sub("a1", tool(T0 + 70, "Bash: b"))
    ));
}

#[test]
fn a_subagent_not_seen_for_ten_minutes_is_dropped_and_what_was_held_goes_through() {
    let mut row = rows(&[started("a1", T0), event(HookEvent::Stop, T0 + 5)]);
    assert_eq!(status(&row), Some("running"));
    // Not quite ten minutes: still there.
    step_keep(&mut row, &notification(Some("auth_success"), T0 + 599));
    assert_eq!(row.as_ref().unwrap().subagents.len(), 1);
    step_keep(&mut row, &notification(Some("auth_success"), T0 + 601));
    let row = row.unwrap();
    assert!(row.subagents.is_empty());
    assert_eq!(row.status, Some(AgentStatus::Done));
    assert_eq!(row.pending_status, None);
    assert_eq!(row.finished_subagents.get("a1"), Some(&(T0 + 601)));
}

#[test]
fn a_subagent_that_has_stopped_is_not_brought_back_for_five_minutes() {
    let mut row = rows(&[started("a1", T0), stopped("a1", T0 + 10)]);
    assert_eq!(
        row.as_ref().unwrap().finished_subagents.get("a1"),
        Some(&(T0 + 10))
    );
    // A late tool call and a repeated start are both ignored.
    step_keep(&mut row, &from_sub("a1", tool(T0 + 20, "Bash: b")));
    step_keep(&mut row, &started("a1", T0 + 30));
    assert!(row.as_ref().unwrap().subagents.is_empty());
    assert_eq!(status(&row), Some("running"));

    // After five minutes the entry is pruned on the next event, and the id is free again.
    step_keep(&mut row, &started("a1", T0 + 10 + 301));
    let row = row.unwrap();
    assert_eq!(row.subagents.len(), 1);
    assert!(row.finished_subagents.is_empty());
}

#[test]
fn a_subagent_that_was_never_started_is_remembered_as_stopped() {
    let row = rows(&[tool(T0, "Edit: a"), stopped("a1", T0 + 5)]).unwrap();
    assert!(row.subagents.is_empty());
    assert_eq!(row.finished_subagents.get("a1"), Some(&(T0 + 5)));
}

// ── notifications ────────────────────────────────────────────────

#[test]
fn notifications_say_waiting_idle_or_nothing_by_their_type() {
    let after = |kind: Option<&str>, before: &[AgentEvent]| {
        let mut row = rows(before);
        step_keep(&mut row, &notification(kind, T0 + 100));
        row
    };
    for kind in [
        Some("permission_prompt"),
        Some("elicitation_dialog"),
        Some("elicitation_url_dialog"),
        Some("agent_needs_input"),
        Some("a_type_from_the_future"),
        None,
    ] {
        let row = after(kind, &[tool(T0, "Edit: a")]);
        assert_eq!(status(&row), Some("waiting"), "{kind:?}");
        assert_eq!(
            row.unwrap().request.as_deref(),
            Some("Claude needs your permission")
        );
    }
    // One that makes a row.
    assert_eq!(
        status(&after(Some("permission_prompt"), &[])),
        Some("waiting")
    );

    // A request that is already there is the better answer.
    let asked = AgentEvent {
        summary: Some("Bash: make".to_string()),
        ..event(HookEvent::PermissionRequest, T0)
    };
    let row = after(Some("permission_prompt"), std::slice::from_ref(&asked)).unwrap();
    assert_eq!(row.request.as_deref(), Some("Bash: make"));

    assert_eq!(status(&after(Some("idle_prompt"), &[asked])), Some("idle"));
    assert_eq!(
        status(&after(Some("idle_prompt"), &[tool(T0, "Edit: a")])),
        Some("running")
    );
    for kind in [
        "auth_success",
        "elicitation_complete",
        "elicitation_response",
        "agent_completed",
        "quota_auto_resume_fired",
        "quota_auto_resume_stale",
        "quota_auto_resume_disabled",
    ] {
        assert_eq!(
            status(&after(Some(kind), &[tool(T0, "Edit: a")])),
            Some("running"),
            "{kind}"
        );
    }
}

// ── the row on disk ──────────────────────────────────────────────

#[test]
fn the_end_of_a_session_removes_its_row() {
    let dir = tempfile::tempdir().unwrap();
    record(dir.path(), &tool(T0, "Edit: a"));
    assert!(exists(dir.path(), "s1"));
    record(dir.path(), &event(HookEvent::SessionEnd, T0 + 5));
    assert!(!exists(dir.path(), "s1"));
}

#[test]
fn a_row_that_cannot_be_read_is_moved_aside_and_a_new_one_started() {
    let dir = tempfile::tempdir().unwrap();
    let path = agent_session_path(dir.path(), "s1");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ not json").unwrap();
    record(dir.path(), &event(HookEvent::UserPromptSubmit, T0));
    assert_eq!(
        std::fs::read_to_string(agent_session_broken_path(dir.path(), "s1")).unwrap(),
        "{ not json"
    );
    assert_eq!(read(dir.path(), "s1").status, Some(AgentStatus::Running));
}

#[test]
fn a_session_id_that_is_not_a_file_name_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let long = "a".repeat(129);
    for id in ["", "..", "a/b", "../x", ".hidden", "a b", long.as_str()] {
        let bad = AgentEvent {
            session_id: id.to_string(),
            ..event(HookEvent::SessionStart, T0)
        };
        assert!(
            record_agent_event_with(dir.path(), &bad, &no_processes()).is_err(),
            "{id:?}"
        );
    }
    assert!(!agent_sessions_dir(dir.path()).exists());
    for id in ["abc", "0b1c2d3e-0000-4000-8000-123456789abc", "a_b.c-d"] {
        assert!(valid_session_id(id), "{id}");
    }
}

#[test]
fn a_new_row_takes_its_start_time_from_the_table_and_its_worktree_from_git() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    crate::testing::init_repo(&repo, "main");
    let root = dir.path().join("state");
    let table = ProcessTable::fixed(Some(HashMap::from([
        (4242, "Mon Jan  1 00:00:00 2024".to_string()),
        (4343, "Tue Jan  2 00:00:00 2024".to_string()),
    ])));
    let cwd = repo.join("sub").to_string_lossy().to_string();
    let first = AgentEvent {
        cwd: Some(cwd.clone()),
        pid: Some(4242),
        config_dir: Some("/home/user/.claude".to_string()),
        ..event(HookEvent::SessionStart, T0)
    };
    record_agent_event_with(&root, &first, &table).unwrap();
    let row = read(&root, "s1");
    assert_eq!(row.cwd.as_deref(), Some(cwd.as_str()));
    assert_eq!(
        row.worktree.as_deref().map(Path::new),
        Some(repo.canonicalize().unwrap().as_path())
    );
    assert_eq!(row.ps_started.as_deref(), Some("Mon Jan  1 00:00:00 2024"));
    assert_eq!(row.config_dir.as_deref(), Some("/home/user/.claude"));

    let moved = AgentEvent {
        pid: Some(4343),
        ..tool(T0 + 5, "Edit: a")
    };
    let moved = AgentEvent {
        cwd: Some(cwd),
        ..moved
    };
    record_agent_event_with(&root, &moved, &table).unwrap();
    let row = read(&root, "s1");
    assert_eq!(row.pid, Some(4343));
    assert_eq!(row.ps_started.as_deref(), Some("Tue Jan  2 00:00:00 2024"));
    assert!(row.worktree.is_some());
}

#[test]
fn a_directory_outside_git_leaves_the_worktree_out_and_the_row_is_still_written() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("plain");
    std::fs::create_dir_all(&outside).unwrap();
    let e = AgentEvent {
        cwd: Some(outside.to_string_lossy().to_string()),
        ..event(HookEvent::SessionStart, T0)
    };
    record(&dir.path().join("state"), &e);
    let row = read(&dir.path().join("state"), "s1");
    assert_eq!(row.cwd, e.cwd);
    assert_eq!(row.worktree, None);
}

// ── the sweep ────────────────────────────────────────────────────

fn sweep_with(root: &Path, table: &ProcessTable, hook: HookEvent) {
    let e = event(hook, T0);
    record_agent_event_with(root, &e, table).unwrap();
}

#[test]
fn a_row_whose_pid_is_gone_is_swept_with_its_lock_and_a_live_one_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, &row_of("gone", Some(999), Some(T0)));
    put(root, &row_of("alive", Some(5), Some(T0)));
    drop(crate::infra::fs::lock(&agent_session_lock_path(root, "gone")).unwrap());
    let table = ProcessTable::fixed(Some(HashMap::from([(5, "A".to_string())])));
    sweep_with(root, &table, HookEvent::SessionStart);
    assert!(!exists(root, "gone"));
    // A lock a process forked by another test still shares is left for the next sweep, so
    // allow it a moment, as the `try_lock` test does.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while agent_session_lock_path(root, "gone").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the lock was never swept"
        );
        std::thread::sleep(Duration::from_millis(10));
        sweep_with(root, &table, HookEvent::SessionStart);
    }
    assert!(exists(root, "alive"));
    assert!(exists(root, "s1"));
}

#[test]
fn a_row_whose_start_time_differs_is_swept_unless_it_matches_or_was_never_read() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let with = |id: &str, started: Option<&str>| AgentSession {
        ps_started: started.map(str::to_string),
        ..row_of(id, Some(5), Some(T0))
    };
    put(root, &with("recycled", Some("Old")));
    put(root, &with("same", Some("New")));
    put(root, &with("unanchored", None));
    put(root, &with("blank", Some("  ")));
    let table = ProcessTable::fixed(Some(HashMap::from([(5, "New".to_string())])));
    sweep_with(root, &table, HookEvent::SessionStart);
    assert!(!exists(root, "recycled"));
    for kept in ["same", "unanchored", "blank"] {
        assert!(exists(root, kept), "{kept}");
    }
}

#[test]
fn a_table_that_cannot_be_read_removes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), &row_of("gone", Some(999), Some(T0)));
    put(dir.path(), &row_of("old", None, Some(T0 - 30 * 3600)));
    sweep_with(
        dir.path(),
        &ProcessTable::fixed(None),
        HookEvent::SessionStart,
    );
    assert!(exists(dir.path(), "gone"));
    // Not a question for the table.
    assert!(!exists(dir.path(), "old"));
}

#[test]
fn a_row_with_a_pid_and_no_start_time_is_also_swept_after_a_day() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, &row_of("quiet", Some(5), Some(T0 - 25 * 3600)));
    put(root, &row_of("recent", Some(5), Some(T0 - 23 * 3600)));
    let anchored = AgentSession {
        ps_started: Some("A".to_string()),
        ..row_of("anchored", Some(5), Some(T0 - 25 * 3600))
    };
    put(root, &anchored);
    let table = ProcessTable::fixed(Some(HashMap::from([(5, "A".to_string())])));
    sweep_with(root, &table, HookEvent::SessionStart);
    assert!(!exists(root, "quiet"));
    assert!(exists(root, "recent"));
    // With a start time that matches, the pid is the proof and quiet means nothing.
    assert!(exists(root, "anchored"));
}

#[test]
fn a_row_with_no_pid_is_swept_after_a_day_whatever_its_status() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, &row_of("quiet", None, Some(T0 - 25 * 3600)));
    put(root, &row_of("recent", None, Some(T0 - 23 * 3600)));
    put(root, &row_of("untimed", None, None));
    sweep_with(root, &no_processes(), HookEvent::SessionStart);
    assert!(!exists(root, "quiet"));
    assert!(!exists(root, "untimed"));
    assert!(exists(root, "recent"));
}

#[test]
fn only_session_start_and_stop_sweep_and_never_the_sessions_own_row() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, &row_of("gone", Some(999), Some(T0)));
    put(root, &row_of("s1", Some(998), Some(T0)));
    for hook in [
        HookEvent::UserPromptSubmit,
        HookEvent::PostToolUse,
        HookEvent::PermissionRequest,
        HookEvent::StopFailure,
    ] {
        sweep_with(root, &no_processes(), hook);
        assert!(exists(root, "gone"));
    }
    // A sub-agent's `Stop` changes nothing in the row, but it is a `Stop`, and that is what
    // triggers the sweep.
    let before = std::fs::read(agent_session_path(root, "s1")).unwrap();
    record(root, &from_sub("a1", event(HookEvent::Stop, T0)));
    assert!(!exists(root, "gone"));
    assert!(exists(root, "s1"));
    assert_eq!(
        std::fs::read(agent_session_path(root, "s1")).unwrap(),
        before
    );
}

#[test]
fn a_row_another_session_has_locked_is_left_for_the_next_sweep() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, &row_of("busy", Some(999), Some(T0)));
    let held = crate::infra::fs::lock(&agent_session_lock_path(root, "busy")).unwrap();
    sweep_with(root, &no_processes(), HookEvent::SessionStart);
    assert!(exists(root, "busy"));
    assert!(agent_session_lock_path(root, "busy").exists());
    drop(held);
}

#[test]
fn another_sessions_unreadable_row_is_left_alone_and_old_broken_files_go() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let sessions = agent_sessions_dir(root);
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(agent_session_path(root, "garbled"), "{ no").unwrap();
    let old = agent_session_broken_path(root, "old");
    let fresh = agent_session_broken_path(root, "fresh");
    std::fs::write(&old, "x").unwrap();
    std::fs::write(&fresh, "x").unwrap();
    let eight_days = SystemTime::now() - Duration::from_secs(8 * 86_400);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(eight_days)
        .unwrap();
    sweep_with(root, &no_processes(), HookEvent::SessionStart);
    assert!(agent_session_path(root, "garbled").exists());
    assert!(!old.exists());
    assert!(fresh.exists());
}

#[test]
fn readers_leave_a_dead_row_out_without_deleting_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let now = now_secs();
    put(root, &row_of("gone", Some(999), Some(now)));
    put(root, &row_of("older", Some(5), Some(now - 100)));
    put(root, &row_of("newer", Some(5), Some(now)));
    put(root, &row_of("a-tie", Some(5), Some(now)));
    std::fs::write(agent_session_broken_path(root, "x"), "{").unwrap();
    std::fs::write(agent_session_path(root, "garbled"), "{").unwrap();
    let table = ProcessTable::fixed(Some(HashMap::from([(5, "A".to_string())])));
    let ids: Vec<String> = agent_sessions_with(root, &table)
        .unwrap()
        .into_iter()
        .map(|row| row.session_id)
        .collect();
    assert_eq!(ids, ["a-tie", "newer", "older"]);
    assert!(exists(root, "gone"));
    assert!(matches!(agent_session(root, "gone"), Recorded::Found(_)));
    assert_eq!(agent_session(root, "../gone"), Recorded::Absent);
    assert!(
        agent_sessions_with(&root.join("none"), &table)
            .unwrap()
            .is_empty()
    );
    // A file where the directory should be is a failure to look, not an empty list.
    let blocked = tempfile::tempdir().unwrap();
    std::fs::write(agent_sessions_dir(blocked.path()), "").unwrap();
    assert!(agent_sessions_with(blocked.path(), &table).is_err());
}

// ── the status line ──────────────────────────────────────────────

fn window(used: f64, resets_at: i64) -> RateWindow {
    RateWindow {
        used_percent: Some(used),
        resets_at: Some(resets_at),
        other: serde_json::Map::new(),
    }
}

/// One draw of the status line.
fn draw(
    at: i64,
    model: Option<&str>,
    context_percent: Option<f64>,
    five_hour: Option<RateWindow>,
    seven_day: Option<RateWindow>,
) -> AgentEvent {
    event(
        HookEvent::StatusLine {
            model: model.map(str::to_string),
            context_percent,
            five_hour,
            seven_day,
        },
        at,
    )
}

#[test]
fn a_status_line_never_makes_a_row() {
    let dir = tempfile::tempdir().unwrap();
    record(
        dir.path(),
        &draw(T0, Some("Opus"), Some(43.0), Some(window(1.0, 5)), None),
    );
    assert!(!exists(dir.path(), "s1"));
    assert!(!agent_session_lock_path(dir.path(), "s1").exists());
}

#[test]
fn a_status_line_sets_the_figures_and_leaves_the_status_and_updated_at() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    record(root, &event(HookEvent::SessionStart, T0));
    record(root, &event(HookEvent::UserPromptSubmit, T0 + 1));
    record(
        root,
        &draw(
            T0 + 10,
            Some("Opus"),
            Some(43.0),
            Some(window(23.5, 1_767_225_600)),
            Some(window(61.0, 1_767_744_000)),
        ),
    );
    let row = read(root, "s1");
    assert_eq!(row.status, Some(AgentStatus::Running));
    assert_eq!(row.updated_at, Some(T0 + 1));
    assert_eq!(row.last_event_at, Some(T0 + 10));
    assert_eq!(row.model.as_deref(), Some("Opus"));
    assert_eq!(row.context_percent, Some(43.0));

    let raw: Value =
        serde_json::from_slice(&std::fs::read(agent_session_path(root, "s1")).unwrap()).unwrap();
    assert_eq!(raw["contextPercent"], 43.0);
    assert_eq!(raw["rateLimits"]["fiveHour"]["usedPercent"], 23.5);
    assert_eq!(raw["rateLimits"]["fiveHour"]["resetsAt"], 1_767_225_600);
    assert_eq!(raw["rateLimits"]["sevenDay"]["usedPercent"], 61.0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(agent_session_path(root, "s1"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn the_same_figures_are_not_written_again_within_a_minute() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    record(root, &event(HookEvent::SessionStart, T0));
    let same = |at| draw(at, Some("Opus"), Some(43.0), Some(window(1.0, 5)), None);
    record(root, &same(T0 + 1));
    assert_eq!(read(root, "s1").last_event_at, Some(T0 + 1));
    record(root, &same(T0 + 31));
    assert_eq!(read(root, "s1").last_event_at, Some(T0 + 1));
    record(root, &same(T0 + 62));
    assert_eq!(read(root, "s1").last_event_at, Some(T0 + 62));
    record(
        root,
        &draw(
            T0 + 63,
            Some("Opus"),
            Some(44.0),
            Some(window(1.0, 5)),
            None,
        ),
    );
    let row = read(root, "s1");
    assert_eq!(row.context_percent, Some(44.0));
    assert_eq!(row.last_event_at, Some(T0 + 63));
}

#[test]
fn figures_a_draw_does_not_carry_keep_their_last_value() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    record(root, &event(HookEvent::SessionStart, T0));
    record(
        root,
        &draw(
            T0 + 1,
            Some("Opus"),
            Some(43.0),
            Some(window(23.5, 5)),
            Some(window(61.0, 6)),
        ),
    );
    record(
        root,
        &draw(T0 + 2, None, Some(50.0), None, Some(window(62.0, 7))),
    );
    let row = read(root, "s1");
    assert_eq!(row.model.as_deref(), Some("Opus"));
    assert_eq!(row.context_percent, Some(50.0));
    let limits = row.rate_limits.unwrap();
    assert_eq!(limits.five_hour, Some(window(23.5, 5)));
    assert_eq!(limits.seven_day, Some(window(62.0, 7)));
}

#[test]
fn a_window_a_draw_gives_without_its_reset_time_keeps_the_stored_one_and_unknown_keys() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    record(root, &event(HookEvent::SessionStart, T0));
    let mut stored = window(23.5, 5);
    stored
        .other
        .insert("x-unknown".into(), serde_json::json!(1));
    record(root, &draw(T0 + 1, None, None, Some(stored), None));
    let without_reset = RateWindow {
        used_percent: Some(30.0),
        resets_at: None,
        other: serde_json::Map::new(),
    };
    record(root, &draw(T0 + 2, None, None, Some(without_reset), None));
    let five_hour = read(root, "s1").rate_limits.unwrap().five_hour.unwrap();
    assert_eq!(five_hour.used_percent, Some(30.0));
    assert_eq!(five_hour.resets_at, Some(5));
    assert_eq!(
        five_hour.other.get("x-unknown"),
        Some(&serde_json::json!(1))
    );
}

#[test]
fn a_status_line_leaves_an_unreadable_row_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = agent_session_path(root, "s1");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "not json").unwrap();
    record(root, &draw(T0, Some("Opus"), Some(43.0), None, None));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
    assert!(!agent_session_broken_path(root, "s1").exists());
}

// ── records shared across versions ───────────────────────────────

#[test]
fn a_key_this_binary_does_not_know_survives_an_event_and_so_does_an_unknown_status() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = agent_session_path(root, "s1");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let raw = serde_json::json!({
        "sessionId": "s1",
        "agent": "claude",
        "status": "snoozing",
        "lastEventAt": T0,
        "createdAt": T0,
        "updatedAt": T0,
        "x-unknown": {"a": [1, 2]},
        "model": "some-model",
        "contextPercent": 12.0,
        "rateLimits": {"fiveHour": {"usedPercent": 1.5, "resetsAt": 5, "x-win": 1}, "x-lim": 2},
        "subagents": [{
            "id": "a1", "type": "Explore", "startedAt": T0, "lastSeenAt": T0, "x-sub": true
        }]
    });
    std::fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

    // Nothing in it changes: the status is another version's word, and it is kept as it is.
    record(root, &notification(Some("auth_success"), T0 + 5));
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
        raw
    );

    // A write keeps every key, the sub-agent's too.
    record(root, &from_sub("a1", tool(T0 + 100, "Bash: b")));
    let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(written["x-unknown"], raw["x-unknown"]);
    assert_eq!(written["model"], "some-model");
    assert_eq!(written["contextPercent"], raw["contextPercent"]);
    assert_eq!(written["rateLimits"], raw["rateLimits"]);
    assert_eq!(written["subagents"][0]["x-sub"], true);
    assert_eq!(written["subagents"][0]["lastSeenAt"], T0 + 100);
    assert_eq!(written["status"], "snoozing");

    record(root, &tool(T0 + 200, "Edit: a"));
    assert_eq!(read(root, "s1").status, Some(AgentStatus::Running));
}

#[test]
fn events_that_overlap_are_all_kept() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    record(root, &event(HookEvent::SessionStart, T0));
    std::thread::scope(|scope| {
        for n in 0..8 {
            scope.spawn(move || {
                let e = match n {
                    7 => tool(T0 + 1, "Edit: src/lib.rs"),
                    _ => started(&format!("a{n}"), T0 + 1),
                };
                record_agent_event_with(root, &e, &no_processes()).unwrap();
            });
        }
    });
    let row = read(root, "s1");
    let mut ids: Vec<&str> = row.subagents.iter().map(|sub| sub.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["a0", "a1", "a2", "a3", "a4", "a5", "a6"]);
    assert_eq!(row.activity.as_deref(), Some("Edit: src/lib.rs"));
}
