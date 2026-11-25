use super::*;

#[test]
fn a_notice_is_taken_once_and_oldest_first() {
    let dir = tempfile::tempdir().unwrap();
    assert!(claim_notices(dir.path(), 5).is_empty());
    enqueue_notice(dir.path(), "o/r", "adjutant / r", "second", 200).unwrap();
    enqueue_notice(dir.path(), "o/r", "adjutant / r", "first", 100).unwrap();
    let got = claim_notices(dir.path(), 5);
    assert_eq!(
        got.iter().map(|n| n.message.as_str()).collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(got[0].title, "adjutant / r");
    assert_eq!(got[0].nwo, "o/r");
    assert!(claim_notices(dir.path(), 5).is_empty());
}

#[test]
fn a_claim_takes_no_more_than_it_was_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    for at in 1..=3 {
        enqueue_notice(dir.path(), "", "t", "m", at).unwrap();
    }
    assert_eq!(claim_notices(dir.path(), 2).len(), 2);
    assert_eq!(claim_notices(dir.path(), 2).len(), 1);
}

#[test]
fn of_many_claimers_each_notice_goes_to_exactly_one() {
    let dir = tempfile::tempdir().unwrap();
    for at in 0..20 {
        enqueue_notice(dir.path(), "", "t", &format!("m{at}"), at).unwrap();
    }
    let root = dir.path().to_path_buf();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let root = root.clone();
            std::thread::spawn(move || {
                let mut mine = Vec::new();
                loop {
                    let got = claim_notices(&root, 1);
                    if got.is_empty() {
                        return mine;
                    }
                    mine.extend(got);
                }
            })
        })
        .collect();
    let mut all: Vec<String> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .map(|n| n.id)
        .collect();
    assert_eq!(all.len(), 20);
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 20, "a notice was taken twice");
}

#[test]
fn a_notice_expires_only_after_the_clients_window() {
    let dir = tempfile::tempdir().unwrap();
    enqueue_notice(dir.path(), "o/r", "t", "m", 1000).unwrap();
    assert!(take_expired_notices(dir.path(), 1000 + NOTICE_EXPIRY_SECS - 1).is_empty());
    let expired = take_expired_notices(dir.path(), 1000 + NOTICE_EXPIRY_SECS);
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].message, "m");
    // Taken by the sweep: no client gets it, and it is not rung twice.
    assert!(claim_notices(dir.path(), 5).is_empty());
    assert!(take_expired_notices(dir.path(), 5000).is_empty());
}

#[test]
fn a_claimed_notice_is_not_expired_and_an_unreadable_file_is_not_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    enqueue_notice(dir.path(), "", "t", "m", 1).unwrap();
    assert_eq!(claim_notices(dir.path(), 1).len(), 1);
    std::fs::write(notices_dir(dir.path()).join("x.json"), "{broken").unwrap();
    assert!(take_expired_notices(dir.path(), 100_000).is_empty());
    // The sweep leaves it for a while; a client that comes for notices finds nothing to ring
    // in it, and it goes.
    assert!(notices_dir(dir.path()).join("x.json").exists());
    assert!(claim_notices(dir.path(), 5).is_empty());
    assert!(!notices_dir(dir.path()).join("x.json").exists());
}

#[test]
fn a_tag_travels_with_the_notice_and_an_old_record_without_one_reads() {
    let dir = tempfile::tempdir().unwrap();
    enqueue_notice_tagged(dir.path(), "o/r", "t", "m", 1, Some("wait-o-r/row/5")).unwrap();
    enqueue_notice(dir.path(), "o/r", "t", "plain", 2).unwrap();
    // A record as an older binary wrote it.
    write_json(
        &notices_dir(dir.path()).join("000000000003-1-0.json"),
        &json!({ "id": "000000000003-1-0", "title": "t", "message": "old", "nwo": "", "createdAt": 3 }),
    )
    .unwrap();
    let got = claim_notices(dir.path(), 5);
    let tags: Vec<_> = got.iter().map(|n| n.tag.as_deref()).collect();
    assert_eq!(tags, [Some("wait-o-r/row/5"), None, None]);
}
