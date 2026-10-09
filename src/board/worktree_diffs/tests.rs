use super::*;

fn counts(files: usize) -> Uncommitted {
    Uncommitted {
        files,
        ..Uncommitted::default()
    }
}

fn due_at(diffs: &WorktreeDiffs, now: Instant) -> Vec<String> {
    due(&diffs.lock().by_path, now)
}

#[test]
fn a_worktree_never_read_is_due_and_shows_no_counts() {
    let diffs = WorktreeDiffs::default();
    diffs.want("/w/a", true);
    assert_eq!(due_at(&diffs, Instant::now()), ["/w/a"]);
    assert_eq!(diffs.look("/w/a"), (None, None));
    assert_eq!(diffs.look("/w/elsewhere"), (None, None));
}

#[test]
fn a_listed_worktree_is_due_after_ten_seconds_and_another_after_a_minute() {
    let diffs = WorktreeDiffs::default();
    let at = Instant::now();
    diffs.want("/w/up", true);
    diffs.want("/w/down", false);
    diffs.store("/w/up", at, Ok(Some(counts(1))));
    diffs.store("/w/down", at, Ok(Some(counts(2))));
    assert!(due_at(&diffs, at + Duration::from_secs(9)).is_empty());
    assert_eq!(due_at(&diffs, at + Duration::from_secs(10)), ["/w/up"]);
    assert_eq!(
        due_at(&diffs, at + Duration::from_secs(60)),
        ["/w/down", "/w/up"]
    );
}

#[test]
fn a_failed_read_keeps_the_last_counts_and_says_why_until_one_succeeds() {
    let diffs = WorktreeDiffs::default();
    let at = Instant::now();
    diffs.want("/w/a", true);
    diffs.store("/w/a", at, Ok(Some(counts(3))));
    diffs.store("/w/a", at, Err("git could not read HEAD".to_string()));
    assert_eq!(
        diffs.look("/w/a"),
        (Some(counts(3)), Some("git could not read HEAD".to_string()))
    );
    diffs.store("/w/a", at, Ok(Some(counts(4))));
    assert_eq!(diffs.look("/w/a"), (Some(counts(4)), None));
}

#[test]
fn a_directory_that_is_gone_clears_the_counts() {
    let diffs = WorktreeDiffs::default();
    diffs.want("/w/a", true);
    diffs.store("/w/a", Instant::now(), Ok(Some(counts(3))));
    diffs.store("/w/a", Instant::now(), Ok(None));
    assert_eq!(diffs.look("/w/a"), (None, None));
}

#[test]
fn only_the_worktrees_still_listed_are_kept_and_a_forgotten_one_is_not_brought_back() {
    let diffs = WorktreeDiffs::default();
    diffs.want("/w/a", true);
    diffs.want("/w/b", true);
    diffs.keep_only(&HashSet::from(["/w/a".to_string()]));
    diffs.store("/w/b", Instant::now(), Ok(Some(counts(1))));
    assert_eq!(diffs.look("/w/b"), (None, None));
    assert_eq!(due_at(&diffs, Instant::now()), ["/w/a"]);
}

#[test]
fn a_refresh_reads_what_is_due_on_a_thread_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gone").display().to_string();
    let diffs = Arc::new(WorktreeDiffs::default());
    diffs.want(&path, true);
    diffs.refresh(Instant::now());
    // A directory that is not there is read as such, which is an answer and not a failure.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && due_at(&diffs, Instant::now()).contains(&path) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!due_at(&diffs, Instant::now()).contains(&path));
    assert_eq!(diffs.look(&path), (None, None));
    while Instant::now() < deadline && diffs.lock().reading {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!diffs.lock().reading);
}
