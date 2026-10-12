use super::*;
use crate::registry::store::{agent_message_path, write_agent_message, write_agent_session};

const T0: i64 = 1_000_000;

fn row(id: &str, message: Option<&str>) -> AgentSession {
    AgentSession {
        session_id: id.to_string(),
        last_message: message.map(str::to_string),
        last_message_at: message.map(|_| T0),
        ..AgentSession::default()
    }
}

fn cut_row(id: &str) -> AgentSession {
    row(
        id,
        Some(&cut_last_message(&"あ".repeat(3000), LAST_MESSAGE_CHARS)),
    )
}

fn side(root: &Path, id: &str, at: i64, text: &str) {
    write_agent_message(
        root,
        &WholeLastMessage {
            session_id: id.to_string(),
            at,
            text: text.to_string(),
            other: Default::default(),
        },
    )
    .unwrap();
}

#[test]
fn a_side_file_of_the_rows_turn_is_the_whole_text() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_agent_session(root, &cut_row("s1")).unwrap();
    side(root, "s1", T0, &"あ".repeat(3000));
    let read = whole_last_message(root, "s1").unwrap().unwrap();
    assert_eq!(read.text.unwrap().chars().count(), 3000);
    assert!(read.whole);
    assert_eq!(read.at, Some(T0));
    assert_eq!(read.session_id, "s1");
}

#[test]
fn a_side_file_of_another_turn_is_not_used_and_the_cut_text_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_agent_session(root, &cut_row("s1")).unwrap();
    side(root, "s1", T0 - 60, &"old ".repeat(2000));
    let read = whole_last_message(root, "s1").unwrap().unwrap();
    assert!(!read.whole);
    assert_eq!(read.text.unwrap().chars().count(), LAST_MESSAGE_CHARS + 1);
}

#[test]
fn an_unreadable_side_file_is_a_fallback_and_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_agent_session(root, &cut_row("s1")).unwrap();
    std::fs::create_dir_all(agent_message_path(root, "s1").parent().unwrap()).unwrap();
    std::fs::write(agent_message_path(root, "s1"), "not json").unwrap();
    let read = whole_last_message(root, "s1").unwrap().unwrap();
    assert!(!read.whole);
    assert!(read.text.is_some());
}

#[test]
fn a_short_row_with_no_side_file_is_whole_and_a_row_without_a_message_has_no_text() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_agent_session(root, &row("short", Some("Done."))).unwrap();
    let read = whole_last_message(root, "short").unwrap().unwrap();
    assert_eq!(read.text.as_deref(), Some("Done."));
    assert!(read.whole);
    write_agent_session(root, &row("none", None)).unwrap();
    let read = whole_last_message(root, "none").unwrap().unwrap();
    assert_eq!((read.text, read.whole), (None, true));
}

#[test]
fn a_row_cut_before_this_version_has_no_whole_text_to_show() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // As the old hook wrote it: 1000 characters and an ellipsis.
    let old = format!("{}…", "x".repeat(LAST_MESSAGE_CHARS));
    write_agent_session(root, &row("s1", Some(&old))).unwrap();
    let read = whole_last_message(root, "s1").unwrap().unwrap();
    assert!(!read.whole);
}

#[test]
fn no_row_is_none_an_unreadable_row_and_a_bad_id_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    assert_eq!(whole_last_message(root, "nope"), Ok(None));
    std::fs::create_dir_all(root.join("agent-sessions")).unwrap();
    std::fs::write(root.join("agent-sessions/bad.json"), "not json").unwrap();
    assert!(whole_last_message(root, "bad").is_err());
    assert!(whole_last_message(root, "../x").is_err());
    assert!(whole_last_message(root, "").is_err());
}

#[test]
fn looks_cut_at_its_boundaries() {
    let ellipsis = |n: usize| format!("{}…", "x".repeat(n - 1));
    assert!(looks_cut(&ellipsis(LAST_MESSAGE_CHARS + 1)));
    assert!(looks_cut(&ellipsis(LAST_MESSAGE_CHARS * 9 / 10)));
    assert!(!looks_cut(&ellipsis(LAST_MESSAGE_CHARS * 9 / 10 - 1)));
    assert!(looks_cut(&"x".repeat(LAST_MESSAGE_CHARS + 1)));
    assert!(!looks_cut(&"x".repeat(LAST_MESSAGE_CHARS)));
    assert!(!looks_cut(&"x".repeat(LAST_MESSAGE_CHARS - 1)));
}
