use super::*;

/// Only an executable file counts: a directory called `terminal-notifier`, or a file
/// nobody may run, would send `default_command` down a branch that cannot deliver.
#[test]
fn the_path_search_wants_something_it_can_actually_run() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let path = std::ffi::OsString::from(format!("/nonexistent:{}", bin.display()));

    assert!(!in_path(&path, "terminal-notifier"));

    let file = bin.join("terminal-notifier");
    std::fs::write(&file, "#!/bin/sh\n").unwrap();
    assert!(!in_path(&path, "terminal-notifier"), "not executable yet");

    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(in_path(&path, "terminal-notifier"));

    std::fs::create_dir(bin.join("notify-send")).unwrap();
    assert!(
        !in_path(&path, "notify-send"),
        "a directory is not a program"
    );
}
