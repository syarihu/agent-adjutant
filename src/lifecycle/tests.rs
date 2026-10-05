use super::*;

use std::time::Duration;

use crate::kernel::config;
use crate::registry::Liveness;

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

/// With no hub record there is no hub to raise: `focus` answers `None` before any
/// terminal is asked to do anything.
#[test]
fn focusing_a_hub_with_no_record_raises_nothing() {
    let sandbox = crate::testing::Sandbox::empty();
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = crate::registry::context_at(repo, sandbox.state()).unwrap();
    assert!(hub::focus(&ctx, false).unwrap().is_none());
}

use super::worker::{Done, LinkRequest, LinkTo, link, undo};
use crate::task::{self, NewTask, Status, TaskPatch};

/// A hub's context in a sandboxed state directory. Hold the sandbox for the whole test.
fn hub() -> (crate::testing::Sandbox, crate::registry::Context) {
    let sandbox = crate::testing::Sandbox::empty();
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = crate::registry::context_at(repo, sandbox.state()).unwrap();
    (sandbox, ctx)
}

/// A task the test makes the way the board's form would, with nothing handed over.
fn made(ctx: &crate::registry::Context, title: &str) -> task::Task {
    let new = NewTask {
        title: Some(title.to_string()),
        ..NewTask::default()
    };
    task::create(ctx, new, false).unwrap().0
}

/// A link whose worker record cannot be written takes its task writes back: a task naming a
/// worktree whose worker reports for nothing is the half-linked state it exists to avoid. The
/// task it made is gone, and the one it took has every field it wrote put back, absent ones too.
#[test]
fn an_undone_link_removes_the_task_it_made_and_puts_back_the_one_it_took() {
    let (_sandbox, ctx) = hub();
    let created = made(&ctx, "Made by the link");
    let taken = made(&ctx, "Taken by the link");
    let waiting = TaskPatch {
        note: Some(Some("waiting for a person".to_string())),
        ..TaskPatch::default()
    };
    task::update(&ctx, &taken.id, &waiting, false).unwrap();
    let before = task::get(&ctx.state, &ctx.repo.slug, &taken.id).unwrap();
    task::update(
        &ctx,
        &taken.id,
        &TaskPatch {
            worktree: Some(Some("/tmp/acme-widget-try".to_string())),
            status: Some(Status::Dispatched),
            note: Some(None),
            ..TaskPatch::default()
        },
        false,
    )
    .unwrap();

    let back = TaskPatch {
        worktree: Some(before.worktree.clone()),
        status: Some(before.status),
        note: Some(before.note.clone()),
        ..TaskPatch::default()
    };
    undo(
        &ctx,
        vec![
            Done::Created(created.id.clone()),
            Done::Updated(taken.id.clone(), Box::new(back.clone())),
        ],
    )
    .unwrap();

    let gone = task::get(&ctx.state, &ctx.repo.slug, &created.id).unwrap_err();
    assert!(gone.starts_with("no such task"), "{gone}");
    let restored = task::get(&ctx.state, &ctx.repo.slug, &taken.id).unwrap();
    assert_eq!(restored.worktree, None);
    assert_eq!(restored.status, Status::Backlog);
    assert_eq!(restored.note.as_deref(), Some("waiting for a person"));

    // The record made for the link is already gone: say so rather than drop it.
    let err = undo(&ctx, vec![Done::Created(created.id.clone())]).unwrap_err();
    assert!(
        err.starts_with(&format!("{} could not be put back:", created.id)),
        "{err}"
    );
}

/// A worktree no worker ever registered in is refused before anything is written, whichever
/// task it was asked to take.
#[test]
fn a_link_to_a_session_that_has_not_started_writes_no_task() {
    let (sandbox, ctx) = hub();
    let worktree = sandbox.state().join("never-started");
    std::fs::create_dir_all(&worktree).unwrap();
    let existing = made(&ctx, "Already there");
    let new = NewTask {
        title: Some("Would be made".to_string()),
        ..NewTask::default()
    };

    for to in [LinkTo::New(new), LinkTo::Existing(existing.id.clone())] {
        let err = link(&ctx, &worktree, LinkRequest { to, phase: None })
            .err()
            .unwrap();
        assert_eq!(err, "the session has not started yet");
    }

    let tasks = task::list(&ctx.state, &ctx.repo.slug);
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    assert_eq!(tasks[0].status, Status::Backlog);
    assert_eq!(tasks[0].worktree, None);
}

/// A hub's context whose main checkout is a real directory in the sandbox, so that what a
/// dispatch writes under it (the slot lock) can be looked for.
fn hub_with_checkout() -> (crate::testing::Sandbox, crate::registry::Context) {
    let sandbox = crate::testing::Sandbox::empty();
    let main = sandbox.state().join("checkout");
    std::fs::create_dir_all(&main).unwrap();
    let repo = crate::kernel::identity::RepoInfo {
        main: main.to_string_lossy().to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = crate::registry::context_at(repo, sandbox.state()).unwrap();
    (sandbox, ctx)
}

/// A mistyped path is refused before anything is marked: marking writes into the worktree and
/// would create the very directory the check is for, and the slot lock would be left behind.
#[test]
fn starting_a_worker_in_a_path_that_is_not_a_directory_touches_nothing() {
    let (sandbox, ctx) = hub_with_checkout();
    let wt = sandbox.state().join("not-there");
    let request = worker::StartRequest {
        worktree: wt.to_string_lossy().to_string(),
        title: "x".to_string(),
        task: None,
        prompt: None,
        repo: None,
    };
    let err = worker::start(&ctx, &request, false).err().unwrap();
    assert_eq!(err, format!("no such directory: {}", wt.display()));
    assert!(!wt.exists());
    assert!(!crate::registry::is_starting(
        &wt,
        crate::infra::clock::now_secs()
    ));
    assert!(
        !std::path::Path::new(&ctx.repo.main)
            .join(".claude")
            .exists()
    );
}

/// A title taken from a task that does not exist fails before a slot is claimed, so a typo in
/// the task id does not leave a worktree marked as starting.
#[test]
fn starting_a_worker_for_a_task_that_does_not_exist_claims_no_slot() {
    let (sandbox, ctx) = hub_with_checkout();
    let wt = sandbox.state().join("worktree");
    std::fs::create_dir_all(&wt).unwrap();
    let request = worker::StartRequest {
        worktree: wt.to_string_lossy().to_string(),
        title: String::new(),
        task: Some("nope".to_string()),
        prompt: None,
        repo: None,
    };
    let err = worker::start(&ctx, &request, false).err().unwrap();
    assert_eq!(err, "no such task: nope");
    assert!(!crate::registry::is_starting(
        &wt,
        crate::infra::clock::now_secs()
    ));
    assert!(!wt.join(".claude").exists());
    assert!(
        !std::path::Path::new(&ctx.repo.main)
            .join(".claude")
            .exists()
    );
}
