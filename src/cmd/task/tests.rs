use super::*;

/// `adj task next` skips a task by this prefix, and the hub procedure is what writes it.
/// The wording guard in `prompts` pins the procedure; this holds the constant to it.
#[test]
fn the_prefix_the_queue_skips_is_the_one_the_hub_writes() {
    let hub = crate::kernel::prompts::find("adj-hub").unwrap().raw_content;
    // The procedure is hard-wrapped, so compare with every run of whitespace as one space.
    let hub = hub.split_whitespace().collect::<Vec<_>>().join(" ");
    let written = format!("\"{} {{reason}}\"", task::COULD_NOT_START);
    assert!(hub.contains(&written), "adj-hub never writes {written}");
}

/// The procedures as one line of words, for text the file wraps.
fn flowed(name: &str) -> String {
    let raw = crate::kernel::prompts::find(name).unwrap().raw_content;
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Workspace, Report to and Verify commands are not in `READ_BY_WORKER`: `adj-worker` does
/// not name them as brief lines to read.
///
/// The worker finds the brief's lines by label. The writer lives in `brief` and the reader
/// is a procedure, so only a test that sees both can say a rename went to both.
#[test]
fn the_labels_the_brief_writes_are_the_ones_the_worker_reads() {
    let worker = flowed("adj-worker");
    for label in crate::kernel::brief::label::READ_BY_WORKER {
        let quoted = format!("brief's \"{label}\"");
        let bare = format!("brief's {label} ");
        // `Task` is also the start of `Task record`, so its bare form proves nothing:
        // only the quoted one counts for a label another label begins with.
        let begins_another = crate::kernel::brief::label::TASK_BRIEF
            .iter()
            .any(|other| other.starts_with(&format!("{label} ")));
        assert!(
            worker.contains(&quoted) || (!begins_another && worker.contains(&bare)),
            "adj-worker never reads the brief's {label} line"
        );
    }
}

/// `adj-report` copies two lines of the brief into what it forwards.
#[test]
fn the_report_forwards_lines_the_brief_writes() {
    use crate::kernel::brief::label;
    let report = flowed("adj-report");
    let task = format!("the \"{}\" line of `.claude/task-brief.md`", label::TASK);
    let parent = format!("brief's \"{}\"", label::PARENT_TASK);
    assert!(report.contains(&task), "adj-report never reads {task}");
    assert!(report.contains(&parent), "adj-report never reads {parent}");
}

/// The worker branches on these three phrases, so the brief must write one of them.
#[test]
fn the_done_when_the_brief_writes_is_one_the_worker_branches_on() {
    let worker = flowed("adj-worker");
    for done_when in [
        task::DoneWhen::Pr,
        task::DoneWhen::Verify,
        task::DoneWhen::ReportOnly,
    ] {
        let phrase = format!("\"{}\"", done_when.as_prose());
        assert!(worker.contains(&phrase), "adj-worker never reads {phrase}");
    }
}

/// Who implements is on the record, not in the brief: a worker told it was handed to
/// Jules would act on a line the board never shows.
#[test]
fn the_brief_has_no_implementer_line() {
    let written = crate::kernel::brief::render_task(&crate::kernel::brief::TaskBrief {
        key: "WID-1".to_string(),
        title: "t".to_string(),
        tracker: "github".to_string(),
        url: None,
        request: String::new(),
        branch: "b".to_string(),
        base: "-".to_string(),
        parent: "-".to_string(),
        record: "r".to_string(),
        done_when: "up to a PR".to_string(),
        stop_at: "plan".to_string(),
        handover: "-".to_string(),
        copilot_review: "ask".to_string(),
        verify: vec![],
    });
    assert!(!written.to_lowercase().contains("implementer"), "{written}");
}

#[test]
fn a_review_task_is_written_as_the_pr_the_worker_branches_on() {
    use task::DoneWhen;
    assert_eq!(brief_done_when(DoneWhen::Review), "up to a PR");
    assert_eq!(brief_done_when(DoneWhen::Pr), "up to a PR");
    assert_eq!(
        brief_done_when(DoneWhen::Verify),
        "up to handing over for verification"
    );
    assert_eq!(
        brief_done_when(DoneWhen::ReportOnly),
        "investigation only (report and stop)"
    );
}

/// Only a backlog task is said to be kept in the backlog; a task further along is not
/// sent back there by an update that did not hand it over.
#[test]
fn a_task_not_handed_over_is_described_by_where_it_is() {
    assert_eq!(
        not_handed_line(Status::Backlog, "hub").as_deref(),
        Some("Kept in the backlog. Nothing is in hub's inbox yet.")
    );
    let queued = not_handed_line(Status::Queued, "hub").expect("a queued task gets a line");
    assert!(!queued.contains("backlog"), "{queued}");
    assert!(queued.contains("hub"), "{queued}");
    for status in [
        Status::Dispatched,
        Status::Pr,
        Status::Done,
        Status::Cancelled,
    ] {
        assert_eq!(not_handed_line(status, "hub"), None, "{}", status.as_str());
    }
}
