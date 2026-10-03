use super::*;

#[test]
fn create_drops_keys_the_record_does_not_know() {
    let _sandbox = crate::testing::Sandbox::empty();
    let repo = crate::kernel::identity::RepoInfo {
        main: "/tmp/acme-widget".to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = super::super::context_of(repo).unwrap();
    let (task, _) = create(&ctx, &json!({"title": "t", "futureField": 1})).unwrap();
    assert!(task.extra.is_empty());
    let text = std::fs::read_to_string(task::path_of(&dir(&ctx), &task.id)).unwrap();
    assert!(!text.contains("futureField"), "{text}");
}

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

#[test]
fn title_is_taken_verbatim_when_present() {
    let input = json!({ "title": "explicit title", "body": "first line\nsecond line" });
    assert_eq!(derive_title(&input).as_deref(), Some("explicit title"));
}

#[test]
fn title_is_derived_from_the_first_non_empty_line_of_the_body() {
    let input = json!({ "body": "\n\n  Fix the flaky network retry logic  \nand more details" });
    assert_eq!(
        derive_title(&input).as_deref(),
        Some("Fix the flaky network retry logic")
    );
}

#[test]
fn title_is_capped_at_eighty_characters() {
    let long_line = "a".repeat(120);
    let input = json!({ "body": long_line });
    let derived = derive_title(&input).expect("derived");
    assert_eq!(derived.len(), 80);
}

/// A form sends the stop point whether or not one was picked, and nothing picked is the
/// default rather than a refusal.
#[test]
fn a_stop_point_left_blank_is_the_default_and_anything_unknown_is_refused() {
    for blank in [json!({}), json!({"stopAt": null}), json!({"stopAt": ""})] {
        assert!(check_typed_values(&blank).is_ok(), "{blank}");
        let filled = with_defaults(&blank, "t", "20260922T000000Z").unwrap();
        assert_eq!(filled["stopAt"], "plan", "{blank}");
    }
    assert_eq!(
        with_defaults(&json!({"stopAt": "all"}), "t", "20260922T000000Z").unwrap()["stopAt"],
        "all"
    );
    for bad in [json!({"stopAt": "verify"}), json!({"stopAt": 1})] {
        assert!(check_typed_values(&bad).is_err(), "{bad}");
    }
}

/// The parent task is typed on the form and quoted on a command line, so it is held to
/// the same rule as the issue URL.
#[test]
fn a_parent_task_url_that_could_close_a_quote_is_refused() {
    let ok = json!({"parent": "https://github.com/acme/widget/issues/1"});
    assert!(check_typed_values(&ok).is_ok());
    // A key typed on the board is turned into its URL by the hub, not refused here.
    assert!(check_typed_values(&json!({"parent": "ALPHA-233"})).is_ok());
    assert!(check_typed_values(&json!({"parent": ""})).is_ok());
    for bad in [
        "https://x.test/a'; rm -rf ~; '",
        "not a url",
        "https://x.test/a b",
        "ALPHA-233; rm",
    ] {
        let err = check_typed_values(&json!({ "parent": bad })).unwrap_err();
        assert!(err.starts_with("not a task URL: "), "{err}");
    }
}

#[test]
fn a_parent_may_be_a_key_or_a_plain_url_and_nothing_else() {
    assert!(check_parent("ABC-123").is_ok());
    assert!(check_parent("https://example.test/browse/ABC-123").is_ok());
    assert!(check_parent("ABC-123 && id").is_err());
}

#[test]
fn a_text_field_is_cleared_by_null_or_empty_and_refused_as_anything_else() {
    assert_eq!(
        text_field("pr", &json!("https://x/pull/1"))
            .unwrap()
            .as_deref(),
        Some("https://x/pull/1")
    );
    assert_eq!(text_field("pr", &json!(null)).unwrap(), None);
    assert_eq!(text_field("note", &json!("")).unwrap(), None);
    assert_eq!(
        text_field("instruction", &json!("優先して実装してください"))
            .unwrap()
            .as_deref(),
        Some("優先して実装してください")
    );
    assert_eq!(text_field("instruction", &json!("")).unwrap(), None);
    assert_eq!(text_field("instruction", &json!(null)).unwrap(), None);
    for bad in [json!(42), json!(true), json!(["a"]), json!({"a": 1})] {
        assert!(text_field("pr", &bad).is_err(), "{bad} was taken");
    }
}

#[test]
fn a_pr_that_cannot_be_read_says_whether_it_is_a_bare_number_or_not_a_pr() {
    assert!(unreadable_pr("12").contains("bare pull request number"));
    assert!(unreadable_pr("#12").contains("bare pull request number"));
    assert!(unreadable_pr("--web").contains("not a pull request this can read"));
    assert!(unreadable_pr("https://example.com/x").contains("not a pull request"));
}

/// A value that would reach `gh` as a flag is refused before `gh` is run at all.
#[test]
fn an_issue_that_looks_like_a_flag_is_not_handed_to_gh() {
    assert!(matches!(
        read_issue(".", "--web"),
        Err(why) if why.contains("--web")
    ));
}

/// The snapshot is text somebody read from the issue; a caller's JSON does not get to
/// claim it.
#[test]
fn a_snapshot_in_the_input_is_dropped() {
    let input =
        json!({ "title": "t", "issueSnapshot": { "url": "u", "title": "x", "fetchedAt": "s" } });
    let filled = with_defaults(&input, "t", "20260922T000000Z").unwrap();
    assert!(filled.get("issueSnapshot").is_none());
}

#[test]
fn empty_title_and_body_produce_nothing() {
    let input = json!({ "title": "   ", "body": "   \n\n  " });
    assert!(derive_title(&input).is_none());
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
