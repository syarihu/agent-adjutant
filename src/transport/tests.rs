use super::wording::wake_note_sentence;

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
