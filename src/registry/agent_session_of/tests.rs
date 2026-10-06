use super::*;
use crate::registry::store::{agent_sessions_dir, write_agent_session};
use std::collections::HashMap;

fn row(id: &str, pid: Option<u32>, started: Option<&str>, at: i64) -> AgentSession {
    AgentSession {
        session_id: id.to_string(),
        pid,
        ps_started: started.map(str::to_string),
        last_event_at: Some(at),
        created_at: Some(at),
        ..AgentSession::default()
    }
}

fn table() -> ProcessTable {
    ProcessTable::fixed(Some(HashMap::from([
        (5, "A".to_string()),
        (6, "B".to_string()),
    ])))
}

fn put(root: &Path, row: &AgentSession) {
    write_agent_session(root, row).unwrap();
}

fn find(root: &Path, id: Option<&str>, pid: Option<u32>, started: Option<&str>) -> Option<String> {
    let identity = AgentIdentity {
        session_id: id,
        pid,
        ps_started: started,
    };
    agent_session_of(root, &table(), &identity)
        .unwrap()
        .map(|row| row.session_id)
}

#[test]
fn the_session_id_it_was_started_with_finds_the_row() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), &row("s1", Some(5), Some("A"), 10));
    put(dir.path(), &row("s2", Some(6), Some("B"), 20));
    assert_eq!(
        find(dir.path(), Some("s1"), None, None).as_deref(),
        Some("s1")
    );
}

#[test]
fn after_a_clear_the_process_finds_the_row_the_old_id_no_longer_names() {
    let dir = tempfile::tempdir().unwrap();
    // `/clear` starts a new session in the same process; the record still holds the old id.
    put(dir.path(), &row("new", Some(5), Some("A"), 30));
    assert_eq!(
        find(dir.path(), Some("old"), Some(5), Some("A")).as_deref(),
        Some("new")
    );
}

#[test]
fn a_runner_without_a_session_id_is_found_by_its_process() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), &row("theirs", Some(6), Some("B"), 10));
    put(dir.path(), &row("mine", Some(5), Some("A"), 10));
    assert_eq!(
        find(dir.path(), None, Some(5), Some("A")).as_deref(),
        Some("mine")
    );
}

#[test]
fn of_two_rows_for_one_process_the_one_seen_last_wins() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), &row("before-clear", Some(5), Some("A"), 10));
    put(dir.path(), &row("after-clear", Some(5), Some("A"), 20));
    assert_eq!(
        find(dir.path(), None, Some(5), Some("A")).as_deref(),
        Some("after-clear")
    );
}

#[test]
fn a_reused_pid_is_not_the_same_agent() {
    let dir = tempfile::tempdir().unwrap();
    // The row is from an earlier process 5; the one running now started at another time.
    put(dir.path(), &row("stale", Some(5), Some("Old"), 10));
    assert_eq!(find(dir.path(), None, Some(5), Some("A")), None);
    // And the row is not alive in the table, so the id does not find it either.
    assert_eq!(find(dir.path(), Some("stale"), None, None), None);
}

#[test]
fn without_a_start_time_the_pid_is_not_enough() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), &row("s1", Some(5), Some("A"), 10));
    assert_eq!(find(dir.path(), None, Some(5), None), None);
    assert_eq!(find(dir.path(), None, Some(5), Some("  ")), None);
    assert_eq!(find(dir.path(), None, None, Some("A")), None);
}

#[test]
fn a_row_whose_process_is_gone_is_not_returned() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), &row("gone", Some(9), Some("Z"), 10));
    assert_eq!(find(dir.path(), Some("gone"), Some(9), Some("Z")), None);
}

#[test]
fn a_ledger_that_cannot_be_listed_is_an_error_and_not_an_absence() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(agent_sessions_dir(dir.path()), "").unwrap();
    let identity = AgentIdentity {
        session_id: None,
        pid: Some(5),
        ps_started: Some("A"),
    };
    assert!(agent_session_of(dir.path(), &table(), &identity).is_err());
}
