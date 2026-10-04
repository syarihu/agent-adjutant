use super::{AutoResume, HubRequest, HubStart, Planned, plan_launch};
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
