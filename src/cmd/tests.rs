use super::*;

#[test]
fn a_wake_note_reads_as_a_sentence() {
    assert_eq!(
        wake_note_sentence(
            "the wake was not typed into the session (pid 7): its screen shows a question or a menu"
        ),
        "The wake was not typed into the session (pid 7): its screen shows a question or a menu."
    );
    assert_eq!(wake_note_sentence(""), "");
}

/// A hub told where to read and write has to hand both to the tabs it opens. Losing
/// them does not fail: the worker registers in the default world and reports into an
/// inbox the hub is not watching, and both halves look healthy from where they stand.
#[test]
fn a_tab_is_handed_the_config_and_state_directory_it_must_not_lose() {
    let sandbox = crate::testing::Sandbox::empty();
    let parts = forwarded_env(&sandbox.state());
    assert_eq!(parts.first().map(String::as_str), Some("env"));
    assert!(
        parts
            .iter()
            .any(|p| p.starts_with(&format!("{}=", crate::infra::env::CONFIG_ENV))),
        "{parts:?}"
    );
    assert!(
        parts
            .iter()
            .any(|p| p.starts_with(&format!("{}=", crate::infra::env::STATE_DIR_ENV))),
        "{parts:?}"
    );
}

/// A machine that never sets them gets the command line it always had — no `env`
/// prefix, nothing to read past.
#[test]
fn nothing_is_forwarded_when_nothing_was_given() {
    let sandbox = crate::testing::Sandbox::empty();
    unsafe {
        std::env::remove_var(crate::infra::env::CONFIG_ENV);
        std::env::remove_var(crate::infra::env::XDG_CONFIG_HOME_ENV);
        std::env::remove_var(crate::infra::env::STATE_DIR_ENV);
    }
    assert!(forwarded_env(&sandbox.state()).is_empty());
}

#[test]
fn a_relative_config_reaches_a_tab_as_an_absolute_path() {
    let sandbox = crate::testing::Sandbox::empty();
    let _config = crate::testing::EnvVar::set(
        &sandbox,
        crate::infra::env::CONFIG_ENV,
        "relative-config.json",
    );
    unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
    config::anchor_config_env();
    let env = forwarded_env(&sandbox.state());
    assert_eq!(env[0], "env");
    assert!(env.contains(&format!(
        "{}={}",
        crate::infra::env::CONFIG_ENV,
        std::env::current_dir()
            .unwrap()
            .join("relative-config.json")
            .display()
    )));
}

/// A tab reads `ADJUTANT_STATE_DIR` against its own worktree, so it is sent the absolute root
/// the command read: the main checkout's, not the working directory's.
#[test]
fn a_relative_state_dir_reaches_a_tab_as_the_root_the_command_read() {
    let sandbox = crate::testing::Sandbox::empty();
    let _state =
        crate::testing::EnvVar::set(&sandbox, crate::infra::env::STATE_DIR_ENV, "relative-state");
    let root = crate::registry::state_root(Some(std::path::Path::new("/src/widget")));
    let env = forwarded_env(&root);
    assert!(
        env.contains(&format!(
            "{}=/src/widget/relative-state",
            crate::infra::env::STATE_DIR_ENV
        )),
        "{env:?}"
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
    let command = serve::resident_command(
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

/// With `ADJUTANT_CONFIG` unset, `XDG_CONFIG_HOME` is what picks the config, so a tab
/// that did not receive it would read whatever its own shell had.
#[test]
fn a_relative_xdg_config_home_reaches_a_tab_as_an_absolute_path() {
    let sandbox = crate::testing::Sandbox::empty();
    unsafe { std::env::remove_var(crate::infra::env::CONFIG_ENV) };
    let _xdg = crate::testing::EnvVar::set(
        &sandbox,
        crate::infra::env::XDG_CONFIG_HOME_ENV,
        "relative-xdg",
    );
    config::anchor_config_env();
    let env = forwarded_env(&sandbox.state());
    assert_eq!(env[0], "env");
    assert!(
        env.contains(&format!(
            "{}={}",
            crate::infra::env::XDG_CONFIG_HOME_ENV,
            std::env::current_dir()
                .unwrap()
                .join("relative-xdg")
                .display()
        )),
        "{env:?}"
    );
    assert!(
        !env.iter()
            .any(|p| p.starts_with(&format!("{}=", crate::infra::env::CONFIG_ENV))),
        "{env:?}"
    );
}

/// A `GIT_DIR` left over from whatever started the agent would send every git command it
/// runs to another repository, while adjutant itself works on this one.
#[test]
fn the_agent_is_started_without_variables_that_point_it_elsewhere() {
    let cmd = agent_command("true");
    let removed: Vec<&std::ffi::OsStr> = cmd
        .get_envs()
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| name)
        .collect();
    let expected = [
        crate::infra::env::HUB_SESSION_ENV,
        crate::infra::env::HUB_SERVE_ENV,
        crate::infra::env::HUB_ENV,
    ]
    .into_iter()
    .chain(crate::infra::git::REPOSITORY_LOCATION_ENV);
    for name in expected {
        assert!(
            removed.contains(&std::ffi::OsStr::new(name)),
            "{name} is not removed: {removed:?}"
        );
    }
}
use crate::messaging::Liveness;

/// A `look` that answers down a script and then keeps repeating its last word, so a
/// test can say "still there twice, then gone" without owning a process to kill.
fn answers(script: &[Liveness]) -> impl FnMut() -> Liveness + '_ {
    let mut n = 0;
    move || {
        let answer = script[n.min(script.len() - 1)];
        n += 1;
        answer
    }
}

/// What `close` acts on is the worker's own absence rather than anything the close
/// command said. Waiting for that has to be bounded, must not read "cannot tell" as
/// "gone", and must not spend the real budget every time it is tested.
#[test]
fn waiting_for_a_worker_to_go_looks_again_but_not_forever() {
    let budget = Duration::from_secs(2);
    let poll = Duration::from_millis(100);

    // Already gone at the first look: nothing is waited for at all, which is the common
    // case and the reason this polls instead of sleeping the budget.
    let mut waits = 0;
    assert_eq!(
        settled(answers(&[Liveness::Gone]), |_| waits += 1, budget, poll),
        Liveness::Gone
    );
    assert_eq!(waits, 0);

    // Still unwinding for the first two looks, then gone: it stops looking the moment
    // it has its answer.
    let mut waits = 0;
    assert_eq!(
        settled(
            answers(&[Liveness::Alive, Liveness::Alive, Liveness::Gone]),
            |_| waits += 1,
            budget,
            poll
        ),
        Liveness::Gone
    );
    assert_eq!(waits, 2);

    // Never goes: bounded by the budget, and the answer is what was seen rather than
    // what the caller was hoping for.
    let mut waits = 0;
    assert_eq!(
        settled(answers(&[Liveness::Alive]), |_| waits += 1, budget, poll),
        Liveness::Alive
    );
    assert_eq!(waits, 20);

    // "Cannot tell" is not "gone", however long it is asked. This is the answer that
    // gets folded into "nobody there" everywhere a message is being delivered, and
    // folding it here is what removes a live worker's worktree.
    assert_eq!(
        settled(answers(&[Liveness::CannotTell]), |_| {}, budget, poll),
        Liveness::CannotTell
    );

    // A zero interval is a caller's mistake, not a reason to spin forever.
    assert_eq!(
        settled(answers(&[Liveness::Alive]), |_| {}, budget, Duration::ZERO),
        Liveness::Alive
    );
}
