use super::*;
use std::time::Duration;

fn exe_in(dir: &Path, name: &str) -> String {
    let exe = dir.join(name);
    std::fs::write(&exe, "").unwrap();
    exe.to_string_lossy().to_string()
}

#[test]
fn the_file_is_written_once_and_left_alone_while_it_is_right() {
    let dir = tempfile::tempdir().unwrap();
    let exe = exe_in(dir.path(), "adj");
    let state = dir.path().join("state");
    let path = write_agent_hooks(&state, &exe).unwrap();
    assert!(path.starts_with(state.join("agent-hooks")));
    let written = read_json(&path).unwrap();
    assert_eq!(binary_of(&written).as_deref(), Some(exe.as_str()));

    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(write_agent_hooks(&state, &exe).unwrap(), path);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );

    // A file that went wrong is put right.
    std::fs::write(&path, "{}").unwrap();
    write_agent_hooks(&state, &exe).unwrap();
    assert_eq!(read_json(&path).unwrap(), written);
}

#[test]
fn another_binary_gets_a_file_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let (one, two) = (exe_in(dir.path(), "adj"), exe_in(dir.path(), "adj-dev"));
    let state = dir.path().join("state");
    let first = write_agent_hooks(&state, &one).unwrap();
    let second = write_agent_hooks(&state, &two).unwrap();
    assert_ne!(first, second);
    assert!(first.exists() && second.exists());
}

#[test]
fn files_of_binaries_that_are_gone_are_swept_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let (gone, alive) = (exe_in(dir.path(), "gone"), exe_in(dir.path(), "alive"));
    let state = dir.path().join("state");
    let gone_file = write_agent_hooks(&state, &gone).unwrap();
    let alive_file = write_agent_hooks(&state, &alive).unwrap();
    let hooks = state.join("agent-hooks");
    let broken = hooks.join("claude-deadbeef.json");
    std::fs::write(&broken, "not json").unwrap();
    let hub = hooks.join("claude-hub-deadbeef.json");
    std::fs::copy(&gone_file, &hub).unwrap();

    std::fs::remove_file(&gone).unwrap();
    let kept = write_agent_hooks(&state, &alive).unwrap();
    assert_eq!(kept, alive_file);
    assert!(!gone_file.exists());
    assert!(alive_file.exists());
    assert!(broken.exists());
    assert!(hub.exists());
}

#[test]
fn a_relative_path_is_refused_rather_than_written() {
    let dir = tempfile::tempdir().unwrap();
    let err = write_agent_hooks(dir.path(), "adj").unwrap_err();
    assert!(err.contains("not absolute"), "{err}");
    assert!(!dir.path().join("agent-hooks").exists());
}

#[test]
fn a_runner_that_wants_no_hooks_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        agent_hooks_for(dir.path(), "codex exec {prompt}"),
        Hooks::NotWanted
    );
    assert_eq!(
        agent_hooks_for(dir.path(), "claude --settings x {prompt}"),
        Hooks::NotWanted
    );
    assert!(!dir.path().join("agent-hooks").exists());
}

#[test]
fn a_claude_runner_gets_the_file_of_this_binary() {
    let dir = tempfile::tempdir().unwrap();
    let Hooks::Injected(path) = agent_hooks_for(dir.path(), "claude {prompt}") else {
        panic!("not injected");
    };
    let exe = std::env::current_exe().unwrap();
    assert_eq!(
        binary_of(&read_json(&path).unwrap()).as_deref(),
        exe.to_str()
    );
}
