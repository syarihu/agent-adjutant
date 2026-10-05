use std::collections::HashSet;
use std::net::TcpListener;
use std::time::{Duration, Instant};

use serde_json::json;

use super::*;

/// A relative state directory is the main checkout's of the repository the server command was
/// typed in, and outside any repository the working directory's.
#[test]
fn the_resident_root_is_taken_against_the_checkout_it_was_typed_in() {
    let sandbox = crate::testing::Sandbox::empty();
    let _state =
        crate::testing::EnvVar::set(&sandbox, crate::infra::env::STATE_DIR_ENV, "relative-state");
    let checkout = tempfile::tempdir().unwrap();
    let repo = crate::kernel::identity::RepoInfo {
        main: checkout.path().to_string_lossy().to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    assert_eq!(
        resident_root(Some(&repo)),
        checkout.path().join("relative-state")
    );
    assert_eq!(
        resident_root(None),
        std::env::current_dir().unwrap().join("relative-state")
    );
}

#[test]
fn a_board_url_carries_its_path() {
    assert_eq!(
        resident_board_url(4577, "acme-x-1", "tok"),
        "http://127.0.0.1:4577/b/acme-x-1/?token=tok"
    );
    assert_eq!(board_url(4577, "tok"), "http://127.0.0.1:4577/?token=tok");
}

#[test]
fn the_server_log_is_readable_by_its_owner_alone() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.log");
    drop(private_log(&path).unwrap());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // A log an older version made with the default mask is tightened, not trusted.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    drop(private_log(&path).unwrap());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn a_taken_port_falls_back_to_a_free_one() {
    let taken = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = taken.local_addr().unwrap().port();
    let second = bind_preferring(port).unwrap();
    let got = second.local_addr().unwrap().port();
    assert_ne!(got, port);
    assert_ne!(got, 0);
}

#[test]
fn a_pane_is_read_again_only_when_it_has_moved_and_the_last_read_is_old() {
    let lines = LastLines::default();
    let t0 = Instant::now();
    let reads = std::cell::Cell::new(0);
    // The read is made at epoch second 1000 unless a case says otherwise.
    let look_at = |activity, at: Instant, secs| {
        lines.look("pane", Some(activity), at, secs, || {
            reads.set(reads.get() + 1);
            Some(format!("read {}", reads.get()))
        })
    };
    let look = |activity, at| look_at(activity, at, 1000);
    assert_eq!(look(10, t0).as_deref(), Some("read 1"));
    // Nothing moved: the answer stands, however old.
    assert_eq!(
        look(10, t0 + Duration::from_secs(60)).as_deref(),
        Some("read 1")
    );
    // Moved, but read a moment ago.
    assert_eq!(
        look(11, t0 + Duration::from_secs(2)).as_deref(),
        Some("read 1")
    );
    // Moved and read long enough ago.
    assert_eq!(look(11, t0 + LAST_LINE_MIN_AGE).as_deref(), Some("read 2"));
    assert_eq!(reads.get(), 2);
    // Activity has one-second resolution: output in the second of the read may have come
    // after it, so that read is not trusted once the age is up.
    assert_eq!(
        look_at(2000, t0 + Duration::from_secs(20), 2000).as_deref(),
        Some("read 3")
    );
    assert_eq!(
        look_at(2000, t0 + Duration::from_secs(21), 2000).as_deref(),
        Some("read 3")
    );
    assert_eq!(
        look_at(2000, t0 + Duration::from_secs(30), 2001).as_deref(),
        Some("read 4")
    );
    assert_eq!(
        look_at(2000, t0 + Duration::from_secs(40), 2005).as_deref(),
        Some("read 4")
    );
    // A read that found nothing, or a window with no known activity, is tried again.
    let nothing = |activity, at: Instant| lines.look("empty", activity, at, 1000, || None);
    assert_eq!(nothing(Some(5), t0), None);
    let found = |activity, at: Instant| {
        lines.look("empty", activity, at, 1000, || Some("late".to_string()))
    };
    assert_eq!(found(Some(5), t0 + Duration::from_secs(1)), None);
    assert_eq!(
        found(Some(5), t0 + LAST_LINE_MIN_AGE).as_deref(),
        Some("late")
    );
    assert_eq!(
        found(None, t0 + LAST_LINE_MIN_AGE * 2).as_deref(),
        Some("late")
    );
    // A pane that is no longer listed is forgotten.
    lines.keep_only(&HashSet::new());
    assert_eq!(
        look(11, t0 + Duration::from_secs(70)).as_deref(),
        Some("read 5")
    );
}

#[test]
fn binding_the_resident_records_this_process_and_refuses_a_second_while_the_lock_is_held() {
    let sandbox = crate::testing::Sandbox::empty();
    let root = sandbox.state();
    let first = bind_resident(&root, 0).unwrap();
    assert_ne!(first.bound, 0);
    let record = crate::infra::fs::read_json(&root.join("server.json")).unwrap();
    assert_eq!(record["pid"], json!(std::process::id()));
    assert_eq!(record["port"], json!(first.bound));
    for key in ["psStarted", "startedAt", "version"] {
        assert!(record.get(key).is_some(), "{key} in {record}");
    }
    assert_eq!(
        crate::registry::live_resident(&root),
        Some((std::process::id(), first.bound))
    );
    let Err(refused) = bind_resident(&root, 0) else {
        panic!("a second resident bound while the first held the lock")
    };
    assert!(
        refused.starts_with("another adj server is running"),
        "{refused}"
    );
}

/// The resident is detached and stands elsewhere, so it is handed the root it was given, not
/// whatever its own working directory would make of a relative one.
#[test]
fn the_resident_is_started_with_the_root_it_was_given() {
    use std::process::Stdio;
    let sandbox = crate::testing::Sandbox::empty();
    let _state =
        crate::testing::EnvVar::set(&sandbox, crate::infra::env::STATE_DIR_ENV, "relative-state");
    let root = crate::registry::state_root(Some(std::path::Path::new("/src/widget")));
    let command = resident_command(
        std::path::Path::new("adj"),
        0,
        &root,
        Stdio::null(),
        Stdio::null(),
    );
    let value = command
        .get_envs()
        .find(|(name, _)| *name == crate::infra::env::STATE_DIR_ENV)
        .and_then(|(_, value)| value);
    assert_eq!(
        value,
        Some(std::path::Path::new("/src/widget/relative-state").as_os_str())
    );
}
