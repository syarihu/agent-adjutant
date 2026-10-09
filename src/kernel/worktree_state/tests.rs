use super::*;
use crate::infra::git::git;

fn run_git(dir: &Path, args: &[&str]) {
    let mut full = vec![
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@example.com",
        "-c",
        "commit.gpgsign=false",
    ];
    full.extend(args);
    let out = git(&full, Some(dir)).unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn commit_file(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).unwrap();
    run_git(dir, &["add", name]);
    run_git(dir, &["commit", "-q", "-m", &format!("add {name}")]);
}

fn state_of(dir: &Path, base: Option<&str>) -> GitState {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    worktree_git_state(dir, base, deadline).unwrap().unwrap()
}

#[test]
fn a_worktree_that_is_gone_has_no_git_state() {
    let here = tempfile::tempdir().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let gone = here.path().join("nowhere");
    assert_eq!(worktree_git_state(&gone, None, deadline), Ok(None));
}

#[test]
fn an_unborn_head_and_a_detached_one_are_answers_not_errors() {
    let sandbox = crate::testing::Sandbox::empty();
    let _ = &sandbox;
    let here = tempfile::tempdir().unwrap();
    let dir = here.path();
    crate::testing::init_repo(dir, "main");
    let fresh = state_of(dir, None);
    assert_eq!(fresh.head, None);
    assert_eq!(fresh.unpushed.count, 0);

    commit_file(dir, "a.txt", "1\n");
    run_git(dir, &["checkout", "-q", "--detach"]);
    let detached = state_of(dir, None);
    assert_eq!(detached.branch, None);
    assert!(detached.head.is_some());
}

#[test]
fn uncommitted_work_is_counted_by_file_line_and_untracked() {
    let sandbox = crate::testing::Sandbox::empty();
    let _ = &sandbox;
    let here = tempfile::tempdir().unwrap();
    let dir = here.path();
    crate::testing::init_repo(dir, "main");
    commit_file(dir, "a.txt", "one\ntwo\nthree\n");
    commit_file(dir, "gone.txt", "x\ny\n");
    std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\nfour\n").unwrap();
    std::fs::write(dir.join("new.txt"), "untracked\n").unwrap();
    // What adjutant writes into a worktree is not work, even where `.claude/` is not
    // ignored; anything else in there is.
    std::fs::create_dir_all(dir.join(".claude")).unwrap();
    for name in [
        "adjutant-worker.json",
        "adjutant-outbox.md",
        "task-brief.md",
    ] {
        std::fs::write(dir.join(".claude").join(name), "x\n").unwrap();
    }
    std::fs::write(dir.join("blob.bin"), [0u8, 159, 146, 150, 0]).unwrap();
    run_git(dir, &["add", "blob.bin"]);
    std::fs::remove_file(dir.join("gone.txt")).unwrap();

    let state = state_of(dir, None);
    assert_eq!(state.branch.as_deref(), Some("main"));
    // a.txt edited, blob.bin staged, gone.txt deleted; new.txt is not tracked.
    assert_eq!(state.uncommitted.files, 3);
    assert_eq!(state.uncommitted.untracked, 1);
    std::fs::write(dir.join(".claude").join("settings.json"), "{}\n").unwrap();
    assert_eq!(state_of(dir, None).uncommitted.untracked, 2);
    // a.txt: one line changed and one added; gone.txt: two lines; the binary counts none.
    assert_eq!(state.uncommitted.insertions, 2);
    assert_eq!(state.uncommitted.deletions, 3);
}

#[test]
fn a_changed_committed_binary_file_counts_as_binary_and_adds_no_lines() {
    let sandbox = crate::testing::Sandbox::empty();
    let _ = &sandbox;
    let here = tempfile::tempdir().unwrap();
    let dir = here.path();
    crate::testing::init_repo(dir, "main");
    std::fs::write(dir.join("blob.bin"), [0u8, 1, 2, 0]).unwrap();
    run_git(dir, &["add", "blob.bin"]);
    run_git(dir, &["commit", "-q", "-m", "add blob"]);
    std::fs::write(dir.join("blob.bin"), [0u8, 9, 9, 9, 0]).unwrap();

    let uncommitted = state_of(dir, None).uncommitted;
    assert_eq!(uncommitted.files, 1);
    assert_eq!(uncommitted.binary, 1);
    assert_eq!((uncommitted.insertions, uncommitted.deletions), (0, 0));
}

#[test]
fn the_uncommitted_look_agrees_with_the_whole_git_state_and_is_none_for_a_gone_directory() {
    let sandbox = crate::testing::Sandbox::empty();
    let _ = &sandbox;
    let here = tempfile::tempdir().unwrap();
    let dir = here.path();
    let deadline = || std::time::Instant::now() + std::time::Duration::from_secs(10);
    crate::testing::init_repo(dir, "main");
    // Before the first commit, with an untracked file.
    std::fs::write(dir.join("new.txt"), "x\n").unwrap();
    assert_eq!(
        uncommitted_of(dir, deadline()).unwrap(),
        Some(state_of(dir, None).uncommitted)
    );
    commit_file(dir, "a.txt", "one\ntwo\n");
    std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    let got = uncommitted_of(dir, deadline()).unwrap().unwrap();
    assert_eq!((got.files, got.untracked, got.insertions), (1, 1, 2));
    assert_eq!(got, state_of(dir, None).uncommitted);
    assert_eq!(uncommitted_of(&dir.join("nowhere"), deadline()), Ok(None));
}

#[test]
fn with_no_upstream_commits_no_remote_has_are_unpushed() {
    let sandbox = crate::testing::Sandbox::empty();
    let _ = &sandbox;
    let here = tempfile::tempdir().unwrap();
    let dir = here.path();
    crate::testing::init_repo(dir, "main");
    commit_file(dir, "a.txt", "1\n");
    commit_file(dir, "b.txt", "2\n");
    let state = state_of(dir, None);
    assert_eq!(state.upstream, None);
    assert_eq!(state.unpushed.against, "remotes");
    assert_eq!(state.unpushed.count, 2);
    assert_eq!(state.unpushed.commits[0].subject, "add b.txt");
    // No `origin/HEAD` and no remote: the local `main` stands in for the base.
    assert_eq!(state.merged.reference.as_deref(), Some("main"));
    assert_eq!(state.merged.merged, Some(true));

    run_git(dir, &["checkout", "-q", "-b", "feature"]);
    commit_file(dir, "c.txt", "3\n");
    assert_eq!(state_of(dir, None).merged.merged, Some(false));
    run_git(dir, &["branch", "-m", "main", "trunk"]);
    let unknown = state_of(dir, None);
    assert_eq!(unknown.merged.merged, None);
    assert_eq!(unknown.merged.base, None);
    assert!(unknown.merged.reason.is_some());
}

#[test]
fn commits_ahead_of_the_upstream_are_counted_and_merging_follows_the_base() {
    let sandbox = crate::testing::Sandbox::empty();
    let _ = &sandbox;
    let root = tempfile::tempdir().unwrap();
    let origin = root.path().join("origin.git");
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    run_git(
        root.path(),
        &["init", "-q", "--bare", "-b", "main", "origin.git"],
    );
    crate::testing::init_repo(&work, "main");
    run_git(
        &work,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    commit_file(&work, "a.txt", "1\n");
    run_git(&work, &["push", "-q", "-u", "origin", "main"]);
    run_git(&work, &["remote", "set-head", "origin", "main"]);

    run_git(&work, &["checkout", "-q", "-b", "feature"]);
    commit_file(&work, "b.txt", "2\n");
    run_git(&work, &["push", "-q", "-u", "origin", "feature"]);
    commit_file(&work, "c.txt", "3\n");

    let state = state_of(&work, None);
    assert_eq!(state.branch.as_deref(), Some("feature"));
    assert_eq!(state.upstream.as_deref(), Some("origin/feature"));
    assert_eq!(state.unpushed.against, "upstream");
    assert_eq!(state.unpushed.count, 1);
    assert_eq!(state.unpushed.commits.len(), 1);
    // The default branch comes from origin/HEAD, and the feature is not in it.
    assert_eq!(state.merged.base.as_deref(), Some("main"));
    assert_eq!(state.merged.reference.as_deref(), Some("origin/main"));
    assert_eq!(state.merged.merged, Some(false));

    // `origin/main` names the same base as `main`.
    assert_eq!(
        state_of(&work, Some("origin/main")).merged.base.as_deref(),
        Some("main")
    );

    run_git(&work, &["push", "-q", "origin", "feature:main"]);
    run_git(&work, &["fetch", "-q", "origin"]);
    let landed = state_of(&work, Some("main"));
    assert_eq!(landed.merged.merged, Some(true));
    assert_eq!(landed.merged.reference.as_deref(), Some("origin/main"));

    // A worktree started from `origin/main` has it as its upstream, but what it has not
    // pushed is measured against the remotes, not against main.
    run_git(&work, &["checkout", "-q", "-b", "task", "origin/main"]);
    commit_file(&work, "d.txt", "4\n");
    let task = state_of(&work, None);
    assert_eq!(task.upstream.as_deref(), Some("origin/main"));
    assert_eq!(task.unpushed.against, "remotes");
    assert_eq!(task.unpushed.count, 1);

    // A base that is nowhere in the repository is said, not guessed.
    let unknown = state_of(&work, Some("release"));
    assert_eq!(unknown.merged.merged, None);
    assert!(unknown.merged.reason.is_some());
}
