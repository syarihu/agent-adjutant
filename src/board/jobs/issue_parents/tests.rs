use super::*;

fn issue(number: u64) -> IssueRef {
    task::issue_ref_of(&format!("https://github.com/acme/widget/issues/{number}")).unwrap()
}

fn parent(number: u64) -> TrackerParent {
    TrackerParent {
        url: format!("https://github.com/acme/widget/issues/{number}"),
        number,
        title: "The parent".to_string(),
        open: true,
        total: 3,
        completed: 1,
    }
}

fn read(answers: Vec<Result<Option<TrackerParent>, String>>) -> task::IssueParentsRead {
    task::IssueParentsRead {
        answers,
        failed: None,
    }
}

fn want(number: u64, finished: bool) -> Wanted {
    Wanted {
        issue: issue(number),
        finished,
    }
}

fn entry(read_at: Instant, parent: Option<TrackerParent>, error: Option<&str>) -> Entry {
    Entry {
        read_at,
        parent,
        error: error.map(str::to_string),
        ever_read: error.is_none(),
        round_failed: error.is_some(),
    }
}

#[test]
fn an_issue_is_due_when_never_read_or_read_long_enough_ago() {
    let now = Instant::now();
    let wanted = [want(1, false), want(2, false), want(3, false)];
    let mut entries = HashMap::new();
    entries.insert(issue(2), entry(now, None, None));
    entries.insert(issue(3), entry(now - PARENT_REREAD, None, None));
    assert_eq!(due(&entries, &wanted, now), [0, 2]);
    assert_eq!(
        due(&entries, &wanted, now - Duration::from_secs(1)),
        [0],
        "an entry read after `now` is not old"
    );
}

#[test]
fn a_finished_issue_is_read_once_unless_that_read_failed() {
    let now = Instant::now();
    let old = now - PARENT_REREAD * 3;
    let wanted = [want(1, true), want(2, true), want(3, true), want(4, false)];
    let mut entries = HashMap::new();
    entries.insert(issue(2), entry(old, None, None));
    entries.insert(issue(3), entry(old, None, Some("denied")));
    entries.insert(issue(4), entry(old, None, None));
    // 1 never read; 2 read once, so never again; 3 failed, so asked again; 4 is not finished.
    assert_eq!(due(&entries, &wanted, now), [0, 2, 3]);
}

#[test]
fn what_the_tracker_says_goes_from_a_parent_to_a_kept_parent_to_none() {
    let now = Instant::now();
    let asked = [issue(7)];
    let mut entries = HashMap::new();
    let look =
        |entries: &HashMap<IssueRef, Entry>| entries.get(&issue(7)).and_then(|e| e.parent.clone());
    // Never read: there is no answer, so the record is used.
    assert_eq!(look(&entries), None);
    apply(&mut entries, &asked, read(vec![Ok(Some(parent(5)))]), now);
    assert_eq!(look(&entries).unwrap().number, 5);
    // A read that failed keeps what the tracker last said.
    let said = apply(
        &mut entries,
        &asked,
        read(vec![Err("rate limit".to_string())]),
        now,
    );
    assert_eq!(said.len(), 1);
    assert!(said[0].contains("acme/widget#7") && said[0].contains("rate limit"));
    assert_eq!(look(&entries).unwrap().number, 5);
    assert_eq!(entries[&issue(7)].error.as_deref(), Some("rate limit"));
    // Said once per change.
    assert!(
        apply(
            &mut entries,
            &asked,
            read(vec![Err("rate limit".to_string())]),
            now
        )
        .is_empty()
    );
    // Read and found to have none: the tracker no longer holds a parent for it.
    apply(&mut entries, &asked, read(vec![Ok(None)]), now);
    assert_eq!(look(&entries), None);
    assert_eq!(entries[&issue(7)].error, None);
}

#[test]
fn a_round_that_failed_as_a_whole_is_not_said_issue_by_issue() {
    let asked = [issue(1), issue(2)];
    let mut entries = HashMap::new();
    let said = apply(
        &mut entries,
        &asked,
        task::IssueParentsRead {
            answers: vec![Err("down".to_string()), Err("down".to_string())],
            failed: Some("down".to_string()),
        },
        Instant::now(),
    );
    assert!(said.is_empty());
    assert_eq!(entries.len(), 2);
}

#[test]
fn the_parent_is_looked_up_by_the_issue_url_in_any_spelling() {
    let parents = IssueParents::default();
    parents
        .lock()
        .entries
        .insert(issue(7), entry(Instant::now(), Some(parent(5)), None));
    let found = |url: &str| parents.look(url).map(|p| p.number);
    assert_eq!(found("https://github.com/acme/widget/issues/7"), Some(5));
    assert_eq!(found("https://GitHub.com/Acme/Widget/issues/7#x"), Some(5));
    assert_eq!(found("https://github.com/acme/widget/issues/8"), None);
    assert_eq!(found("not an issue"), None);
}

#[test]
fn an_issue_never_read_successfully_is_retried_sooner_than_one_that_was() {
    let now = Instant::now();
    let wanted = [want(1, false), want(2, false)];
    let mut entries = HashMap::new();
    let minutes = |m: u64| Duration::from_secs(m * 60);
    entries.insert(issue(1), entry(now - minutes(3), None, Some("down")));
    entries.insert(issue(2), entry(now - minutes(3), None, None));
    // 1 never succeeded: due after the short interval. 2 did: it waits for the long one.
    assert_eq!(due(&entries, &wanted, now), [0]);
    entries.insert(issue(1), entry(now - minutes(1), None, Some("down")));
    assert!(due(&entries, &wanted, now).is_empty());
    // A failure after a success keeps the long interval.
    let asked = [issue(2)];
    apply(
        &mut entries,
        &asked,
        read(vec![Err("down".to_string())]),
        now - minutes(3),
    );
    assert!(entries[&issue(2)].ever_read);
    assert!(due(&entries, &wanted, now).is_empty());
}

#[test]
fn an_issue_github_refuses_waits_the_long_interval_but_a_failed_round_is_retried_soon() {
    let now = Instant::now();
    let minutes = |m: u64| Duration::from_secs(m * 60);
    let asked = [issue(1), issue(2)];
    let mut entries = HashMap::new();
    // Issue 1 on its own says not found; the round as a whole failed for issue 2.
    apply(
        &mut entries,
        &asked[..1],
        read(vec![Err("Could not resolve".to_string())]),
        now - minutes(3),
    );
    apply(
        &mut entries,
        &asked[1..],
        task::IssueParentsRead {
            answers: vec![Err("rate limit".to_string())],
            failed: Some("rate limit".to_string()),
        },
        now - minutes(3),
    );
    let wanted = [want(1, false), want(2, false)];
    assert_eq!(due(&entries, &wanted, now), [1]);
    assert_eq!(due(&entries, &wanted, now + minutes(8)), [0, 1]);
}
