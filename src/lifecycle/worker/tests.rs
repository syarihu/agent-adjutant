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
        model: None,
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

#[test]
fn a_fresh_plan_with_a_chosen_model_passes_the_model_flag() {
    let _sandbox = crate::testing::Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let mut req = fresh(dir.path());
    req.model = Some("opus".to_string());
    let Ok(Planned::Launch(launch)) = plan_launch(&req) else {
        panic!("expected planned launch");
    };
    assert!(
        launch.command.contains("--model opus"),
        "{}",
        launch.command
    );
    assert_eq!(launch.model.as_deref(), Some("opus"));
    assert!(launch.model_note.is_none());
}

#[test]
fn a_fresh_plan_with_an_invalid_model_is_refused() {
    let _sandbox = crate::testing::Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let mut req = fresh(dir.path());
    req.model = Some("-m".to_string());
    let Err(err) = plan_launch(&req) else {
        panic!("expected refusal for invalid model");
    };
    assert!(err.contains("refused model name"), "{err}");
}

#[test]
fn an_explicit_model_with_a_runner_that_has_no_placeholder_is_refused() {
    let _sandbox = crate::testing::Sandbox::new(
        r#"{"repos": {}, "defaults": {"agentRunner": "codex exec {prompt}"}}"#,
    );
    let dir = tempfile::tempdir().unwrap();
    let mut req = fresh(dir.path());
    req.model = Some("opus".to_string());
    let Err(err) = plan_launch(&req) else {
        panic!("expected refusal for runner without placeholder");
    };
    assert!(err.contains("agentRunner has no {model}"), "{err}");
}

#[test]
fn a_resumed_worker_uses_the_saved_model_when_none_is_requested() {
    let _sandbox = crate::testing::Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    registry::save_worker_session(dir.path(), "WID-1", None, None, "sid-1", Some("haiku")).unwrap();
    let mut req = fresh(dir.path());
    req.resume = true;
    let Ok(Planned::Launch(launch)) = plan_launch(&req) else {
        panic!("expected planned launch");
    };
    assert!(
        launch.command.contains("--model haiku"),
        "{}",
        launch.command
    );
    assert_eq!(launch.model.as_deref(), Some("haiku"));
    assert!(launch.model_note.is_none());
}

#[test]
fn a_resumed_worker_with_an_explicit_model_overrides_the_saved_one() {
    let _sandbox = crate::testing::Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    registry::save_worker_session(dir.path(), "WID-1", None, None, "sid-1", Some("haiku")).unwrap();
    let mut req = fresh(dir.path());
    req.resume = true;
    req.model = Some("opus".to_string());
    let Ok(Planned::Launch(launch)) = plan_launch(&req) else {
        panic!("expected planned launch");
    };
    assert!(
        launch.command.contains("--model opus"),
        "{}",
        launch.command
    );
    assert!(
        !launch.command.contains("--model haiku"),
        "{}",
        launch.command
    );
    assert_eq!(launch.model.as_deref(), Some("opus"));
    assert!(launch.model_note.is_none());
}

#[test]
fn a_saved_model_with_a_refused_resume_runner_keeps_model_in_launch_and_sets_note() {
    let _sandbox = crate::testing::Sandbox::new(
        r#"{"repos": {}, "defaults": {"agentResumeRunner": "codex resume {sessionId} {prompt}"}}"#,
    );
    let dir = tempfile::tempdir().unwrap();
    registry::save_worker_session(dir.path(), "WID-1", None, None, "sid-1", Some("haiku")).unwrap();
    let mut req = fresh(dir.path());
    req.resume = true;
    let Ok(Planned::Launch(launch)) = plan_launch(&req) else {
        panic!("expected planned launch");
    };
    assert!(!launch.command.contains("--model"), "{}", launch.command);
    assert_eq!(launch.model.as_deref(), Some("haiku"));
    let note = launch.model_note.expect("model_note should be set");
    assert!(
        note.contains("the saved model haiku is not passed: agentResumeRunner has no {model}"),
        "{note}"
    );
}

#[test]
fn a_saved_invalid_model_is_not_passed_and_sets_note_while_keeping_saved_model() {
    let _sandbox = crate::testing::Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    registry::save_worker_session(dir.path(), "WID-1", None, None, "sid-1", Some("-x")).unwrap();
    let mut req = fresh(dir.path());
    req.resume = true;
    let Ok(Planned::Launch(launch)) = plan_launch(&req) else {
        panic!("expected planned launch");
    };
    assert!(!launch.command.contains("-x"), "{}", launch.command);
    assert!(!launch.command.contains("--model"), "{}", launch.command);
    assert_eq!(launch.model.as_deref(), Some("-x"));
    let note = launch.model_note.expect("model_note should be set");
    assert!(
        note.contains("the saved model -x is not passed: refused model name \"-x\""),
        "{note}"
    );
}

mod open_tab {
    use super::super::claim_slot::{exited_message, open_worker_tab_with};
    use crate::infra::terminal::{Performed, SpawnRequest, StartWatch};
    use crate::registry;

    fn performed() -> Performed {
        Performed {
            description: "opened a tab".to_string(),
            script: String::new(),
            ran: true,
            screen: false,
        }
    }

    fn open(
        pane: Option<&str>,
        watched: Result<StartWatch, String>,
    ) -> (Result<Performed, String>, bool) {
        let _sandbox = crate::testing::Sandbox::empty();
        let dir = tempfile::tempdir().unwrap();
        registry::mark_worker_starting(dir.path()).unwrap();
        let worktree = dir.path().to_string_lossy().to_string();
        let pane = pane.map(str::to_string);
        let done = open_worker_tab_with(
            &worktree,
            &SpawnRequest {
                cwd: &worktree,
                title: "WID-1",
                command: "true",
                title_command: None,
            },
            false,
            &|_, _| Ok((performed(), pane.clone())),
            &|_| watched.clone(),
        );
        let marked = registry::is_starting(dir.path(), crate::infra::clock::now_secs());
        (done, marked)
    }

    #[test]
    fn a_worker_that_exits_at_once_is_an_error_and_gives_the_slot_back() {
        let (done, marked) = open(
            Some("%1"),
            Ok(StartWatch::Exited {
                status: Some(1),
                screen: Some("adjutant: no such worktree: x".to_string()),
            }),
        );
        let err = done.unwrap_err();
        assert!(err.contains("exited at once"), "{err}");
        assert!(err.contains("adjutant: no such worktree: x"), "{err}");
        assert!(!marked);
    }

    #[test]
    fn an_exit_with_status_zero_is_not_a_failure_to_start() {
        let (done, marked) = open(
            Some("%1"),
            Ok(StartWatch::Exited {
                status: Some(0),
                screen: Some("a worker is already running in this worktree (pid 7)".to_string()),
            }),
        );
        let said = done.unwrap().description;
        assert!(said.contains("without starting anything new"), "{said}");
        assert!(said.contains("already running"), "{said}");
        assert!(!marked);
    }

    #[test]
    fn a_silent_exit_does_not_end_in_a_colon() {
        let said = exited_message(Some(1), Some(""), "/wt");
        assert_eq!(
            said,
            "the worker exited at once (status 1) without printing anything; run `adj worker --worktree /wt` there to see why"
        );
        assert_eq!(
            exited_message(None, Some("boom"), "/wt"),
            "the worker exited at once: boom; nothing was started"
        );
        assert_eq!(
            exited_message(Some(2), None, "/wt"),
            "the worker exited at once (status 2), but what it printed could not be read; run `adj worker --worktree /wt` there to see why"
        );
    }

    #[test]
    fn a_window_that_closed_at_once_is_an_error_and_gives_the_slot_back() {
        let (done, marked) = open(Some("%1"), Ok(StartWatch::Gone));
        assert!(done.unwrap_err().contains("closed at once"));
        assert!(!marked);
    }

    #[test]
    fn a_worker_still_up_keeps_its_mark() {
        let (done, marked) = open(Some("%1"), Ok(StartWatch::Alive));
        assert!(done.is_ok());
        assert!(marked);
    }

    #[test]
    fn a_watch_that_failed_keeps_the_mark_since_a_worker_may_be_running() {
        let (done, marked) = open(Some("%1"), Err("no server".to_string()));
        let description = done.unwrap().description;
        assert!(
            description.contains("could not be checked (no server)"),
            "{description}"
        );
        assert!(marked);
    }

    #[test]
    fn without_a_pane_nothing_is_watched() {
        let (done, marked) = open(None, Ok(StartWatch::Alive));
        assert!(done.is_ok());
        assert!(marked);
        let _sandbox = crate::testing::Sandbox::empty();
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().to_string_lossy().to_string();
        let done = open_worker_tab_with(
            &worktree,
            &SpawnRequest {
                cwd: &worktree,
                title: "t",
                command: "true",
                title_command: None,
            },
            true,
            &|_, _| Ok((performed(), None)),
            &|_| panic!("nothing to watch"),
        );
        assert!(done.is_ok());
    }
}
