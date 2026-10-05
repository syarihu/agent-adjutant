use super::{Planned, WorkerRequest, plan_launch};
use crate::registry;
use std::path::Path;

fn fresh(worktree: &Path) -> WorkerRequest {
    WorkerRequest {
        repo: Some("acme/widget".to_string()),
        hub: None,
        worktree: Some(worktree.to_string_lossy().to_string()),
        title: Some("WID-1".to_string()),
        task: None,
        prompt: None,
        resume: false,
    }
}

/// A fresh start with the built-in runner plans a session to record, and writes nothing
/// itself.
#[test]
fn a_fresh_launch_with_the_built_in_runner_records_a_session() {
    let _sandbox = crate::testing::Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let Ok(Planned::Launch(launch)) = plan_launch(&fresh(dir.path())) else {
        panic!("nothing is running, so this has to plan a launch");
    };
    let session = launch
        .fresh_session
        .as_deref()
        .expect("a session to record");
    assert!(launch.command.contains(session), "{}", launch.command);
    assert!(launch.resumed.is_none());
    assert!(!registry::worker_record_path(dir.path()).exists());
}

/// A runner with no `{session_id}` has nothing to record, so none is planned.
#[test]
fn an_agent_runner_without_a_session_id_records_none() {
    let _sandbox = crate::testing::Sandbox::new(
        r#"{"repos": {}, "defaults": {"agentRunner": "my-agent {prompt}"}}"#,
    );
    let dir = tempfile::tempdir().unwrap();
    let Ok(Planned::Launch(launch)) = plan_launch(&fresh(dir.path())) else {
        panic!("nothing is running, so this has to plan a launch");
    };
    assert!(launch.fresh_session.is_none());
}

/// A worker already running here turns the launch away, and the slot `adj work` marked goes
/// back.
#[test]
fn a_live_worker_turns_the_launch_away_and_gives_back_the_mark() {
    let _sandbox = crate::testing::Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    registry::register_worker(dir.path(), "WID-1", None, None, None).unwrap();
    registry::mark_worker_starting(dir.path()).unwrap();
    assert!(registry::is_starting(
        dir.path(),
        crate::infra::clock::now_secs()
    ));
    let Ok(planned) = plan_launch(&fresh(dir.path())) else {
        panic!("a running worker is an answer, not an error");
    };
    assert!(matches!(planned, Planned::Running { pid: Some(p) } if p == std::process::id()));
    assert!(!registry::is_starting(
        dir.path(),
        crate::infra::clock::now_secs()
    ));
}
