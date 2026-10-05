use super::{AutoResume, HubRequest, HubStart, Planned, close, plan_launch};
use crate::registry;

/// A sandbox hub with a session saved and seen alive just now.
fn hub_with_a_saved_session() -> crate::registry::Context {
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = registry::context_of(repo).unwrap();
    registry::save_hub_session(
        &ctx.state,
        &ctx.repo.slug,
        &ctx.repo.nwo,
        ctx.repo.hub.as_deref(),
        &ctx.repo.hub_name,
        "sid-saved",
    )
    .unwrap();
    registry::touch_hub_session(&ctx.state, &ctx.repo.slug, "sid-saved").unwrap();
    ctx
}

fn plain() -> HubRequest {
    HubRequest {
        start: HubStart::Auto,
        dashboard: None,
        extra: Vec::new(),
    }
}

/// A plain `adj hub` comes back to the session that was alive a moment ago, and plans a
/// command that reopens it.
#[test]
fn a_plain_hub_plans_to_resume_a_recent_session() {
    let _sandbox = crate::testing::Sandbox::empty();
    let ctx = hub_with_a_saved_session();
    let Planned::Launch(launch) = plan_launch(&ctx, &plain()).unwrap() else {
        panic!("nothing is running, so this has to plan a launch");
    };
    assert!(matches!(launch.auto, Some(AutoResume::Resuming { .. })));
    assert_eq!(launch.session, "sid-saved");
    assert_eq!(launch.resumed.unwrap().session_id, "sid-saved");
    assert!(launch.command.contains("sid-saved"), "{}", launch.command);
}

/// `hubAutoResumeHours` of 0 turns the coming-back off, so the same saved session starts a
/// new one instead.
#[test]
fn a_hub_with_auto_resume_off_plans_a_new_session() {
    let _sandbox = crate::testing::Sandbox::new(r#"{"repos": {}, "hubAutoResumeHours": 0}"#);
    let ctx = hub_with_a_saved_session();
    let Planned::Launch(launch) = plan_launch(&ctx, &plain()).unwrap() else {
        panic!("nothing is running, so this has to plan a launch");
    };
    assert!(launch.auto.is_none());
    assert!(launch.resumed.is_none());
    assert_ne!(launch.session, "sid-saved");
}

/// A parent hub in a real repository under `dir`, with its record written as `record` says and
/// a board address remembered for it: what `close` has to clear.
fn parent_hub_with_a_record(
    dir: &std::path::Path,
    record: serde_json::Value,
) -> (
    crate::registry::Context,
    crate::mail::RepoHub,
    std::path::PathBuf,
) {
    let main = dir.join("widget");
    std::fs::create_dir_all(&main).unwrap();
    crate::testing::init_repo(&main, "main");
    let repo = crate::kernel::identity::RepoInfo {
        main: main.to_string_lossy().into_owned(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: Some("feature".to_string()),
        slug: "acme-widget-feature".to_string(),
        hub_name: "adjutant-acme-widget-feature".to_string(),
        nwo_source: "dirname",
    };
    let ctx = registry::context_of(repo).unwrap();
    let path = registry::hub_record_path(&ctx.state, &ctx.repo.slug);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut body = serde_json::json!({
        "hubName": ctx.repo.hub_name,
        "hub": "feature",
        "cwd": ctx.repo.main,
    });
    body.as_object_mut()
        .unwrap()
        .extend(record.as_object().unwrap().clone());
    std::fs::write(&path, body.to_string()).unwrap();
    registry::note_board(&ctx.state, &ctx.repo);
    let hub = crate::mail::RepoHub {
        id: format!("hub-{}", ctx.repo.slug),
        parent: true,
        key: ctx.repo.hub.clone(),
        name: ctx.repo.hub_name.clone(),
        title: None,
        slug: ctx.repo.slug.clone(),
        state: crate::mail::RepoHubState {
            present: false,
            stale: false,
            pid: None,
            started_at: None,
        },
        inbox_count: 2,
        inbox: Vec::new(),
        children: 0,
    };
    (ctx, hub, path)
}

/// A parent hub whose record names no process is closed: the record and the board's address
/// for it are gone, and the unread count it kept is handed back.
#[test]
fn closing_a_parent_hub_that_names_no_process_clears_its_record_and_board_address() {
    let sandbox = crate::testing::Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let (ctx, hub, path) = parent_hub_with_a_record(dir.path(), serde_json::json!({}));
    let address = ctx.state.join("boards").join(format!("{}.json", hub.slug));
    assert!(path.exists() && address.exists());

    let closed = close(&ctx.state, &ctx.repo, &hub).unwrap();

    assert_eq!(closed.unread, 2);
    assert!(!path.exists());
    assert!(!address.exists());
    drop(sandbox);
}

/// A parent hub whose record names a live process other than this one is not closed from
/// outside: the record stays, and the refusal says it is still running.
#[test]
fn closing_a_parent_hub_that_is_still_running_is_refused_and_leaves_its_record() {
    let sandbox = crate::testing::Sandbox::empty();
    // Reparented away from the test by the shell that exits, so it is not a descendant of it.
    let out = std::process::Command::new("sh")
        .args(["-c", "sleep 300 >/dev/null 2>&1 & echo $!"])
        .output()
        .unwrap();
    let sleeper: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    struct Reap(u32);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = std::process::Command::new("kill")
                .arg(self.0.to_string())
                .status();
        }
    }
    let _reap = Reap(sleeper);
    let dir = tempfile::tempdir().unwrap();
    let (ctx, hub, path) = parent_hub_with_a_record(
        dir.path(),
        serde_json::json!({
            "pid": sleeper,
            "psStarted": registry::ps_started(sleeper),
            "nameInCommand": false,
        }),
    );

    let refused = close(&ctx.state, &ctx.repo, &hub).err().unwrap();

    assert!(refused.contains("still running"), "{refused}");
    assert!(path.exists());
    drop(sandbox);
}
