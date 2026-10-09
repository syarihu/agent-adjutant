use super::*;

fn branch(repo: &str, name: &str) -> BranchRef {
    BranchRef {
        host: "github.com".to_string(),
        owner: "acme".to_string(),
        repo: repo.to_string(),
        branch: name.to_string(),
    }
}

fn pr(number: u64, state: &str) -> BranchPr {
    BranchPr {
        number,
        url: format!("https://github.com/acme/widget/pull/{number}"),
        state: state.to_string(),
    }
}

fn read(answers: Vec<Result<Option<BranchPr>, String>>) -> task::BranchPrsRead {
    task::BranchPrsRead {
        answers,
        failed: None,
    }
}

fn entry(read_at: Instant, found: Option<SessionPr>) -> Entry {
    Entry {
        read_at,
        pr: found,
        error: None,
    }
}

#[test]
fn a_branch_is_due_when_never_read_old_enough_or_named_by_the_news() {
    let now = Instant::now();
    let wanted = [
        branch("widget", "new"),
        branch("widget", "fresh"),
        branch("widget", "old"),
        branch("gadget", "named"),
    ];
    let mut entries = HashMap::new();
    let key = |i: usize| key_of(&wanted[i]);
    entries.insert(key(1), entry(now, None));
    entries.insert(key(2), entry(now - BRANCH_REREAD, None));
    entries.insert(key(3), entry(now, None));
    let none = HashSet::new();
    assert_eq!(due(&entries, &wanted, now, &none), [0, 2]);
    let news = HashSet::from(["acme/gadget".to_string()]);
    assert_eq!(due(&entries, &wanted, now, &news), [0, 2, 3]);
    // A repository the news names makes every branch of it due.
    let news = HashSet::from(["acme/widget".to_string()]);
    assert_eq!(due(&entries, &wanted, now, &news), [0, 1, 2]);
}

#[test]
fn a_found_answer_replaces_the_old_one_and_none_is_an_answer() {
    let now = Instant::now();
    let asked = [branch("widget", "a"), branch("widget", "b")];
    let mut entries = HashMap::new();
    let said = apply(
        &mut entries,
        &asked,
        read(vec![Ok(Some(pr(5, "draft"))), Ok(None)]),
        now,
    );
    assert!(said.is_empty());
    assert_eq!(entries[&key_of(&asked[0])].pr.as_ref().unwrap().number, 5);
    assert_eq!(
        entries[&key_of(&asked[0])].pr.as_ref().unwrap().state,
        "draft"
    );
    assert_eq!(entries[&key_of(&asked[1])].pr, None);
    apply(
        &mut entries,
        &asked[..1],
        read(vec![Ok(Some(pr(6, "merged")))]),
        now,
    );
    assert_eq!(entries[&key_of(&asked[0])].pr.as_ref().unwrap().number, 6);
}

#[test]
fn a_failed_read_keeps_the_answer_and_says_why_once_per_change() {
    let now = Instant::now();
    let asked = [branch("widget", "a")];
    let mut entries = HashMap::new();
    apply(
        &mut entries,
        &asked,
        read(vec![Ok(Some(pr(5, "open")))]),
        now,
    );
    let fail = |why: &str| read(vec![Err(why.to_string())]);
    let first = apply(&mut entries, &asked, fail("not found"), now);
    assert_eq!(first.len(), 1);
    assert!(first[0].contains("a in acme/widget") && first[0].contains("not found"));
    let entry = &entries[&key_of(&asked[0])];
    assert_eq!(entry.pr.as_ref().unwrap().number, 5);
    assert_eq!(entry.error.as_deref(), Some("not found"));
    assert!(apply(&mut entries, &asked, fail("not found"), now).is_empty());
    assert_eq!(apply(&mut entries, &asked, fail("denied"), now).len(), 1);
    // Heard again, it is no longer failing and says so the next time it does.
    apply(
        &mut entries,
        &asked,
        read(vec![Ok(Some(pr(5, "open")))]),
        now,
    );
    assert_eq!(entries[&key_of(&asked[0])].error, None);
    assert_eq!(apply(&mut entries, &asked, fail("denied"), now).len(), 1);
}

#[test]
fn a_round_that_failed_as_a_whole_is_not_said_branch_by_branch() {
    let asked = [branch("widget", "a"), branch("widget", "b")];
    let mut entries = HashMap::new();
    let said = apply(
        &mut entries,
        &asked,
        task::BranchPrsRead {
            answers: vec![Err("rate limit".to_string()), Err("rate limit".to_string())],
            failed: Some("rate limit".to_string()),
        },
        Instant::now(),
    );
    assert!(said.is_empty());
    assert_eq!(entries.len(), 2);
}

#[test]
fn a_branch_is_looked_up_by_the_repository_in_any_case() {
    let prs = BranchPrs::default();
    prs.lock().entries.insert(
        ("acme/widget".to_string(), "Feature".to_string()),
        entry(
            Instant::now(),
            Some(SessionPr {
                number: 3,
                url: "u".to_string(),
                state: "open".to_string(),
            }),
        ),
    );
    assert_eq!(prs.look("Acme/Widget", "Feature").0.unwrap().number, 3);
    assert_eq!(prs.look("acme/widget", "feature"), (None, None));
}

#[test]
fn a_failed_lookup_is_looked_up_with_the_pr_of_the_last_good_one() {
    let prs = BranchPrs::default();
    let asked = [branch("widget", "a")];
    {
        let mut inner = prs.lock();
        let now = Instant::now();
        apply(
            &mut inner.entries,
            &asked,
            read(vec![Ok(Some(pr(5, "open")))]),
            now,
        );
        apply(
            &mut inner.entries,
            &asked,
            read(vec![Err("denied".to_string())]),
            now,
        );
    }
    let (found, error) = prs.look("acme/widget", "a");
    assert_eq!(found.unwrap().number, 5);
    assert_eq!(error.as_deref(), Some("denied"));
    // Never read at all, or read and found nothing, says nothing.
    assert_eq!(prs.look("acme/widget", "b"), (None, None));
}

#[test]
fn what_is_held_goes_only_for_repositories_whose_worktrees_were_listed() {
    let now = Instant::now();
    let kept = branch("widget", "kept");
    let gone = branch("widget", "gone");
    let unlisted = branch("gadget", "unlisted");
    let mut entries = HashMap::new();
    for r in [&kept, &gone, &unlisted] {
        entries.insert(key_of(r), entry(now, None));
    }
    // `acme/gadget` could not be listed this round, so its entry stays whatever was wanted.
    let listed = HashSet::from(["acme/widget".to_string()]);
    prune(&mut entries, std::slice::from_ref(&kept), &listed);
    assert!(entries.contains_key(&key_of(&kept)));
    assert!(!entries.contains_key(&key_of(&gone)));
    assert!(entries.contains_key(&key_of(&unlisted)));
}

#[test]
fn an_origin_that_cannot_be_read_is_an_error_and_not_another_host() {
    let dir = tempfile::tempdir().unwrap();
    let repo = |main: &std::path::Path| crate::kernel::identity::RepoInfo {
        main: main.to_string_lossy().to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "origin",
    };
    // Not a repository: git does not answer, which says nothing of which host it is on.
    assert!(branches_in(&repo(&dir.path().join("nowhere")), "github.com").is_err());
    // A name taken from the directory is not on any host, and that is an answer.
    let mut named = repo(dir.path());
    named.nwo_source = "dirname";
    assert_eq!(branches_in(&named, "github.com"), Ok(Vec::new()));
}
