use super::*;
use crate::infra::fs::write_json;
use crate::registry::{hub_record_path, save_hub_session, save_worker_session, worker_record_path};
use crate::testing::Sandbox;
use serde_json::json;

fn a_message(body: &str) -> Message {
    Message {
        from: "w".into(),
        worktree: None,
        kind: "report".into(),
        subject: "s".into(),
        body: body.into(),
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut names: Vec<String> = read
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn an_absent_hub_still_takes_delivery() {
    let _sandbox = Sandbox::empty();
    let delivery = send(
        "acme-widget",
        "adjutant-acme-widget",
        &Message {
            from: "wid-957-34".into(),
            worktree: None,
            kind: "report".into(),
            subject: "検索結果の画像が縦に潰れる".into(),
            body: "## Symptom\nthe image is squashed".into(),
        },
    )
    .unwrap();
    assert!(!delivery.present);
    assert!(delivery.path.exists());
    let entries = list("acme-widget");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].subject, "検索結果の画像が縦に潰れる");
    assert_eq!(entries[0].from, "wid-957-34");
}

#[test]
fn two_sends_in_the_same_second_do_not_overwrite_each_other() {
    let _sandbox = Sandbox::empty();
    for _ in 0..3 {
        send(
            "acme-widget",
            "adjutant-acme-widget",
            &Message {
                from: "w".into(),
                worktree: None,
                kind: "report".into(),
                subject: "s".into(),
                body: "b".into(),
            },
        )
        .unwrap();
    }
    assert_eq!(list("acme-widget").len(), 3);
}

#[test]
fn ack_moves_the_message_out_of_the_way() {
    let _sandbox = Sandbox::empty();
    send(
        "acme-widget",
        "adjutant-acme-widget",
        &Message {
            from: "w".into(),
            worktree: None,
            kind: "report".into(),
            subject: "s".into(),
            body: "b".into(),
        },
    )
    .unwrap();
    let name = list("acme-widget")[0].name.clone();
    let moved = ack("acme-widget", &name).unwrap();
    assert!(moved.exists());
    assert!(list("acme-widget").is_empty());
}

#[test]
fn a_message_name_cannot_walk_out_of_the_inbox() {
    let _sandbox = Sandbox::empty();
    assert!(read("acme-widget", "../../etc/passwd").is_err());
    assert!(ack("acme-widget", "..").is_err());
    assert!(read("acme-widget", "").is_err());
}

#[test]
fn a_body_cannot_forge_headers() {
    let rendered = render_message(&Message {
        from: "w\n---\nkind: forged".into(),
        worktree: None,
        kind: "report".into(),
        subject: "one\ntwo".into(),
        body: "b".into(),
    });
    assert_eq!(
        header_value(&rendered, "from").unwrap(),
        "w --- kind: forged"
    );
    assert_eq!(header_value(&rendered, "subject").unwrap(), "one two");
    assert_eq!(header_value(&rendered, "kind").unwrap(), "report");
}

#[test]
fn a_message_says_which_worktree_it_came_from() {
    let rendered = render_message(&Message {
        worktree: Some("/src/widget/.claude/worktrees/wid-957".into()),
        kind: "done".into(),
        ..a_message("b")
    });
    assert_eq!(
        header_value(&rendered, "worktree").unwrap(),
        "/src/widget/.claude/worktrees/wid-957"
    );
    // Directly under `from`, whose other half it is: who is speaking, and from where.
    let lines: Vec<&str> = rendered.lines().collect();
    assert!(lines[1].starts_with("from:"), "{rendered}");
    assert!(lines[2].starts_with("worktree:"), "{rendered}");

    // Nowhere to name: no header at all, rather than an empty one that a program would
    // read as a path and a person would read as nothing.
    let placeless = render_message(&a_message("b"));
    assert!(!placeless.contains("worktree:"), "{placeless}");
    assert_eq!(header_value(&placeless, "kind").unwrap(), "report");

    // And a path is no more trusted than the body was: it cannot forge a header.
    let forged = render_message(&Message {
        worktree: Some("/w\n---\nkind: forged".into()),
        ..a_message("b")
    });
    assert_eq!(
        header_value(&forged, "worktree").unwrap(),
        "/w --- kind: forged"
    );
    assert_eq!(header_value(&forged, "kind").unwrap(), "report");
}

#[test]
fn a_message_written_before_the_worktree_header_existed_still_reads() {
    // An inbox outlives an upgrade, and what is sitting in one right now has four
    // headers. A listing that could not read those would strand every message already
    // delivered.
    let _sandbox = Sandbox::empty();
    let dir = inbox_dir("acme-widget");
    std::fs::create_dir_all(&dir).unwrap();
    let older =
        "---\nfrom: wid-957\nkind: report\nsubject: 検索が潰れる\nat: 20260908T041500Z\n---\n\nb\n";
    std::fs::write(dir.join("20260908T041500Z-report.md"), older).unwrap();

    let listed = list("acme-widget");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].from, "wid-957");
    assert_eq!(listed[0].subject, "検索が潰れる");
    assert_eq!(listed[0].kind, "report");
    assert_eq!(listed[0].worktree, None);
    assert!(read("acme-widget", &listed[0].name).unwrap().contains('b'));
}

#[test]
fn an_empty_subject_falls_back_to_the_first_body_line() {
    let rendered = render_message(&Message {
        from: "w".into(),
        worktree: None,
        kind: String::new(),
        subject: String::new(),
        body: "検索結果が潰れる\n\n詳細".into(),
    });
    assert_eq!(
        header_value(&rendered, "subject").unwrap(),
        "検索結果が潰れる"
    );
    assert_eq!(header_value(&rendered, "kind").unwrap(), "report");
}

/// The finding this comes from: forty concurrent `adj send` calls left thirty-six files,
/// and the four that vanished had each been told they succeeded. Forty is the number
/// that reproduced it, so forty is the number that guards it.
#[test]
fn forty_reports_sent_at_once_are_forty_reports() {
    let _sandbox = Sandbox::empty();
    let bodies: Vec<String> = (0..40).map(|i| format!("finding number {i}")).collect();
    std::thread::scope(|scope| {
        for body in &bodies {
            scope.spawn(move || {
                send("acme-widget", "adjutant-acme-widget", &a_message(body)).unwrap();
            });
        }
    });

    let entries = list("acme-widget");
    assert_eq!(entries.len(), 40, "reports were lost: {entries:?}");

    // Every one of them whole, and every one of them still its own report: a delivery
    // that overwrites is as bad as one that drops, and counting alone would miss it.
    let mut seen: Vec<String> = entries
        .iter()
        .map(|e| {
            let text = read("acme-widget", &e.name).unwrap();
            assert!(
                text.starts_with("---\nfrom: w\n"),
                "half a message: {text:?}"
            );
            text.lines().last().unwrap().to_string()
        })
        .collect();
    seen.sort();
    let mut expected = bodies.clone();
    expected.sort();
    assert_eq!(seen, expected);

    // Nothing staged is left behind — invisible to `list`, so it would grow unnoticed.
    let staged: Vec<String> = names_in(&inbox_dir("acme-widget"))
        .into_iter()
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(staged.is_empty(), "staging files left over: {staged:?}");
}

/// Acking never overwrites the archive: the point of keeping a message is that a report
/// that was mishandled can still be found, and an overwrite is exactly losing one.
#[test]
fn acking_the_same_name_twice_keeps_both() {
    let _sandbox = Sandbox::empty();
    let dir = inbox_dir("acme-widget");
    std::fs::create_dir_all(&dir).unwrap();
    // The same name twice on purpose — a send can reuse a name the moment the previous
    // message with it has been acked, and both then land on one archived name.
    for body in ["the first report", "the second report"] {
        std::fs::write(dir.join("20260908T112233Z-report.md"), body).unwrap();
        ack("acme-widget", "20260908T112233Z-report.md").unwrap();
    }
    let archived = names_in(&archive_dir("acme-widget"));
    assert_eq!(
        archived,
        vec![
            "20260908T112233Z-report-1.md".to_string(),
            "20260908T112233Z-report.md".to_string()
        ]
    );
    let bodies: Vec<String> = archived
        .iter()
        .map(|n| std::fs::read_to_string(archive_dir("acme-widget").join(n)).unwrap())
        .collect();
    assert!(bodies.contains(&"the first report".to_string()));
    assert!(bodies.contains(&"the second report".to_string()));
}

#[test]
fn a_numbered_name_keeps_its_extension() {
    assert_eq!(numbered("report.md", 0), "report.md");
    assert_eq!(numbered("report.md", 2), "report-2.md");
    assert_eq!(numbered("a.b.md", 2), "a.b-2.md");
    assert_eq!(numbered("noext", 2), "noext-2");
}

/// Only one ack can succeed, because only one of them takes the message.
///
/// Filing first and unlinking second let every acker "succeed": each filed a copy, and
/// each then unlinked the inbox name — including the one that by then belonged to a
/// message somebody had just sent, which nothing had filed.
#[test]
fn one_message_acked_five_times_at_once_is_acked_once() {
    let _sandbox = Sandbox::empty();
    let dir = inbox_dir("acme-widget");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("20260908T112233Z-report.md"), "the only report").unwrap();

    let outcomes: Vec<Result<PathBuf, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..5)
            .map(|_| scope.spawn(|| ack("acme-widget", "20260908T112233Z-report.md")))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(
        outcomes.iter().filter(|r| r.is_ok()).count(),
        1,
        "{outcomes:?}"
    );
    for outcome in outcomes.iter().filter(|r| r.is_err()) {
        let message = outcome.as_ref().unwrap_err();
        assert!(message.contains("is waiting"), "{message}");
    }
    // Filed exactly once, and the inbox is empty rather than holding a copy.
    assert_eq!(names_in(&archive_dir("acme-widget")).len(), 1);
    assert!(list("acme-widget").is_empty());
    // Nothing staged left behind on any of the five paths.
    let staged: Vec<String> = names_in(&dir)
        .into_iter()
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(staged.is_empty(), "{staged:?}");
}

/// A reader looking into the inbox while senders are writing never sees half a message.
///
/// The forty-send test reads only once every sender has finished, so it cannot see a
/// partial file even if one were briefly published. This one reads *during*, which is
/// the only way to look at the window the staging-then-linking exists to close.
#[test]
fn a_reader_during_delivery_never_sees_half_a_message() {
    let _sandbox = Sandbox::empty();
    let body = "x".repeat(64 * 1024);
    let done = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        let senders: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    for _ in 0..25 {
                        send("acme-widget", "adjutant-acme-widget", &a_message(&body)).unwrap();
                    }
                })
            })
            .collect();
        let reader = scope.spawn(|| {
            let mut looked_at = 0usize;
            // At least one pass *after* the senders stop, so a reader the scheduler ran
            // late still reads the two hundred files they left. Otherwise a correct
            // implementation fails whenever this thread happens to start last.
            loop {
                let finished = done.load(std::sync::atomic::Ordering::Relaxed);
                for entry in list("acme-widget") {
                    // A name `list` returned can only be gone if something removed it,
                    // and nothing here does — so anything readable must be whole.
                    if let Ok(text) = read("acme-widget", &entry.name) {
                        assert!(text.starts_with("---\nfrom: w\n"), "half a header");
                        assert!(
                            text.trim_end().ends_with(&body),
                            "half a body: {} bytes",
                            text.len()
                        );
                        looked_at += 1;
                    }
                }
                if finished {
                    return looked_at;
                }
            }
        });
        for sender in senders {
            sender.join().unwrap();
        }
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(reader.join().unwrap() >= 200);
    });
    assert_eq!(list("acme-widget").len(), 200);
}

/// An ack that stops half way must leave the message reachable.
///
/// Taking it before filing it is what stops a concurrent ack deleting somebody else's
/// new message — but between the two steps the only copy wears a name `list` does not
/// offer and `read` will not open, so a process that dies there used to leave a report
/// that exists and cannot be reached.
#[test]
fn a_message_left_held_by_an_interrupted_ack_comes_back() {
    let _sandbox = Sandbox::empty();
    let dir = inbox_dir("acme-widget");
    std::fs::create_dir_all(&dir).unwrap();
    // Exactly what an ack leaves behind when it dies after the rename: the content, a
    // holding name, and the name it came from written into it.
    let held = dir.join(format!("{HOLDING}424242-0-20260908T112233Z-report.md"));
    std::fs::write(&held, "the report nobody filed").unwrap();
    // Old enough that no ack could still be in flight.
    let long_ago =
        std::time::SystemTime::now() - std::time::Duration::from_secs(HELD_STALE_SECS * 2);
    std::fs::File::options()
        .write(true)
        .open(&held)
        .unwrap()
        .set_modified(long_ago)
        .unwrap();

    let waiting = list("acme-widget");
    assert_eq!(waiting.len(), 1, "{waiting:?}");
    assert_eq!(waiting[0].name, "20260908T112233Z-report.md");
    assert_eq!(
        read("acme-widget", &waiting[0].name).unwrap(),
        "the report nobody filed"
    );
    assert!(!held.exists());
}

/// A hold that could still be in flight is left alone: putting it back while its ack is
/// between the two steps would file it *and* leave a copy waiting.
#[test]
fn a_message_still_being_acked_is_not_put_back_underneath_it() {
    let _sandbox = Sandbox::empty();
    let dir = inbox_dir("acme-widget");
    std::fs::create_dir_all(&dir).unwrap();
    let held = dir.join(format!("{HOLDING}424242-0-20260908T112233Z-report.md"));
    std::fs::write(&held, "mid-flight").unwrap();

    assert!(list("acme-widget").is_empty());
    assert!(held.exists());
}

#[test]
fn an_inbox_lists_the_newest_first_with_when_each_was_sent() {
    let _sandbox = Sandbox::empty();
    let slug = "acme-widget";
    let dir = inbox_dir(slug);
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
    write(
        "20260101T000001Z-report.md",
        "---\nfrom: a\nkind: report\nsubject: first\nat: 20260101T000001Z\n---\n\nb\n",
    );
    // No `at` header: the file name says when.
    write(
        "20260101T000002Z-question.md",
        "---\nfrom: b\nkind: question\nsubject: second\n---\n\nb\n",
    );
    write("odd-name.md", "---\nfrom: c\nsubject: third\n---\n\nb\n");
    let entries = list(slug);
    assert_eq!(entries[0].at.as_deref(), Some("20260101T000001Z"));
    assert_eq!(entries[1].at.as_deref(), Some("20260101T000002Z"));
    assert_eq!(entries[2].at, None);
}

#[test]
fn all_repo_hubs_discovers_all_sources_and_sorts_repo_first() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let dir = tempfile::tempdir().unwrap();
    let main_path = dir.path().to_string_lossy().to_string();
    let repo = crate::kernel::identity::RepoInfo {
        main: main_path.clone(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };

    // 1. Repo hub record in hubs/
    write_json(
        &hub_record_path(&root, "acme-widget"),
        &json!({
            "hubName": "adjutant-acme-widget",
            "cwd": main_path,
        }),
    )
    .unwrap();

    // 2. Parent task hub in hubs/
    let parent_slug = crate::kernel::identity::slug_for("acme/widget", Some("parent-task"));
    write_json(
        &hub_record_path(&root, &parent_slug),
        &json!({
            "hubName": format!("adjutant-{parent_slug}"),
            "cwd": main_path,
            "hub": "parent-task",
        }),
    )
    .unwrap();

    // 3. Saved session in sessions/
    let session_slug = crate::kernel::identity::slug_for("acme/widget", Some("other-hub"));
    save_hub_session(
        &root,
        &session_slug,
        "acme/widget",
        Some("other-hub"),
        &format!("adjutant-{session_slug}"),
        "sess-123",
    )
    .unwrap();

    // 4. Main checkout worker record naming a parent hub
    write_json(
        &worker_record_path(Path::new(&main_path)),
        &json!({
            "hub": "main-worker-hub",
        }),
    )
    .unwrap();

    let hubs = all_repo_hubs(&repo);
    // `other-hub` is known only from its saved session, which no longer lists a parent
    // hub on its own (#161): nothing points at it, so it is finished.
    assert_eq!(hubs.len(), 3);
    // Repository hub is always first
    assert_eq!(hubs[0].id, "hub");
    assert_eq!(hubs[0].key, None);
    assert_eq!(hubs[0].name, "adjutant-acme-widget");

    // The remaining hubs are sorted by id
    let ids: Vec<_> = hubs.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(ids, vec!["hub", "hub-main-worker-hub", "hub-parent-task"]);
}

#[test]
fn all_repo_hubs_discovers_hub_from_saved_worker_session_when_record_absent() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let main_path = dir.path().to_string_lossy().to_string();
    let repo = crate::kernel::identity::RepoInfo {
        main: main_path.clone(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };

    save_worker_session(
        Path::new(&main_path),
        "Previous worker task",
        Some("unregistered-worker-hub"),
        None,
        "sid-prev",
    )
    .unwrap();

    let hubs = all_repo_hubs(&repo);
    assert_eq!(hubs.len(), 2);
    assert_eq!(hubs[0].id, "hub");
    assert_eq!(hubs[1].id, "hub-unregistered-worker-hub");
    assert_eq!(hubs[1].key.as_deref(), Some("unregistered-worker-hub"));
}

fn widget_repo(main: &Path) -> crate::kernel::identity::RepoInfo {
    crate::kernel::identity::RepoInfo {
        main: main.to_string_lossy().to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    }
}

#[test]
fn a_parent_hub_known_only_from_its_saved_session_is_not_listed() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let dir = tempfile::tempdir().unwrap();
    let slug = crate::kernel::identity::slug_for("acme/widget", Some("WID-957"));
    save_hub_session(
        &root,
        &slug,
        "acme/widget",
        Some("WID-957"),
        "adjutant-x",
        "sess-1",
    )
    .unwrap();

    let hubs = all_repo_hubs(&widget_repo(dir.path()));
    assert_eq!(hubs.len(), 1, "{hubs:?}");
    assert_eq!(hubs[0].id, "hub");
    assert_eq!(hubs[0].children, 0);
    // The session is still there to resume.
    assert_eq!(hub_sessions_for(&root, "acme/widget").len(), 1);
}

#[test]
fn a_parent_hub_with_an_ended_worker_is_listed_with_its_count() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    // The worker's record is gone; its saved session still names the hub.
    save_worker_session(dir.path(), "WID-957", Some("WID-957"), None, "sid-1").unwrap();

    let hubs = all_repo_hubs(&widget_repo(dir.path()));
    assert_eq!(hubs.len(), 2, "{hubs:?}");
    assert_eq!(hubs[0].children, 0);
    assert!(hubs[1].parent);
    assert_eq!(hubs[1].key.as_deref(), Some("WID-957"));
    assert_eq!(hubs[1].children, 1);
}

#[test]
fn children_are_counted_by_slug_not_by_spelling() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let dir = tempfile::tempdir().unwrap();
    write_json(&worker_record_path(dir.path()), &json!({"hub": "WID-957"})).unwrap();
    let slug = crate::kernel::identity::slug_for("acme/widget", Some("wid-957"));
    save_hub_session(
        &root,
        &slug,
        "acme/widget",
        Some("wid-957"),
        "adjutant-x",
        "sess-1",
    )
    .unwrap();

    let hubs = all_repo_hubs(&widget_repo(dir.path()));
    assert_eq!(hubs.len(), 2, "{hubs:?}");
    assert_eq!(hubs[1].slug, slug);
    assert_eq!(hubs[1].children, 1);
    // The hub session's own name is the better one to show.
    assert_eq!(hubs[1].name, "adjutant-x");
}

#[test]
fn a_worker_moved_to_another_hub_counts_only_there() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let dir = tempfile::tempdir().unwrap();
    // Dispatched by A, moved to B: the record says B, the session still says A.
    write_json(&worker_record_path(dir.path()), &json!({"hub": "B"})).unwrap();
    save_worker_session(dir.path(), "t", Some("A"), None, "sid-1").unwrap();
    let a = crate::kernel::identity::slug_for("acme/widget", Some("A"));
    save_hub_session(&root, &a, "acme/widget", Some("A"), "adjutant-a", "sess-a").unwrap();

    let hubs = all_repo_hubs(&widget_repo(dir.path()));
    let keys: Vec<_> = hubs.iter().map(|h| h.key.as_deref()).collect();
    assert_eq!(keys, vec![None, Some("B")], "{hubs:?}");
    assert_eq!(hubs[1].children, 1);
}

#[test]
fn a_hub_known_only_from_a_worker_is_given_a_name() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    write_json(&worker_record_path(dir.path()), &json!({"hub": "WID-957"})).unwrap();

    let hubs = all_repo_hubs(&widget_repo(dir.path()));
    let slug = crate::kernel::identity::slug_for("acme/widget", Some("WID-957"));
    assert_eq!(hubs[1].name, format!("adjutant-{slug}"));
}

#[test]
fn a_parent_hub_record_without_its_key_is_still_told_apart_from_the_repository_hub() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let dir = tempfile::tempdir().unwrap();
    let main_path = dir.path().to_string_lossy().to_string();
    let repo = crate::kernel::identity::RepoInfo {
        main: main_path.clone(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    // Written by a version that did not record `hub`: the key comes back out of the slug.
    let readable = crate::kernel::identity::slug_for("acme/widget", Some("WID-100"));
    write_json(
        &hub_record_path(&root, &readable),
        &json!({"hubName": format!("adjutant-{readable}"), "cwd": main_path}),
    )
    .unwrap();
    // And one whose key the slug cannot give back: still a parent hub, key unknown.
    let lossy = crate::kernel::identity::slug_for("acme/widget", Some("v1.2"));
    write_json(
        &hub_record_path(&root, &lossy),
        &json!({"hubName": format!("adjutant-{lossy}"), "cwd": main_path}),
    )
    .unwrap();

    let hubs = all_repo_hubs(&repo);
    assert_eq!(hubs.len(), 3, "{hubs:?}");
    assert_eq!(hubs[0].id, "hub");
    assert!(!hubs[0].parent);
    let recovered = hubs.iter().find(|h| h.slug == readable).unwrap();
    assert!(recovered.parent);
    assert_eq!(recovered.key.as_deref(), Some("wid-100"));
    assert_eq!(recovered.id, "hub-wid-100");
    let unknown = hubs.iter().find(|h| h.slug == lossy).unwrap();
    assert!(unknown.parent);
    assert_eq!(unknown.key, None);
    assert_eq!(unknown.id, format!("hub-{lossy}"));
}

#[test]
fn all_repo_hubs_seeds_default_hub_when_repo_addresses_parent_hub() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let main_path = dir.path().to_string_lossy().to_string();
    let repo = crate::kernel::identity::RepoInfo {
        main: main_path,
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: Some("WID-100".to_string()),
        slug: "acme-widget-wid-100".to_string(),
        hub_name: "adjutant-wid-100".to_string(),
        nwo_source: "dirname",
    };

    let hubs = all_repo_hubs(&repo);
    assert_eq!(hubs.len(), 2);
    assert_eq!(hubs[0].id, "hub");
    assert_eq!(
        hubs[0].slug,
        crate::kernel::identity::slug_for("acme/widget", None)
    );
    assert_eq!(hubs[0].key, None);
    assert_eq!(hubs[1].id, "hub-WID-100");
    assert_eq!(hubs[1].slug, "acme-widget-wid-100");
    assert_eq!(hubs[1].key.as_deref(), Some("WID-100"));
}

#[test]
fn only_claude_and_agy_runners_have_their_screens_read() {
    use crate::infra::agent::Agent;
    // The built-in runner is Claude Code.
    assert_eq!(wake_agent(None), Agent::Claude);
    assert_eq!(wake_agent(Some("claude {prompt}")), Agent::Claude);
    assert_eq!(
        wake_agent(Some("/opt/bin/claude --resume {sessionId}")),
        Agent::Claude
    );
    assert_eq!(wake_agent(Some("agy")), Agent::Agy);
    assert_eq!(wake_agent(Some("/usr/local/bin/agy")), Agent::Agy);
    assert_eq!(
        wake_agent(Some(
            "env CLAUDE_CONFIG_DIR=/tmp/c claude --resume {sessionId}"
        )),
        Agent::Claude
    );
    assert_eq!(
        wake_agent(Some("AGY_HOME=/tmp/a env agy {prompt}")),
        Agent::Agy
    );
    assert_eq!(
        wake_agent(Some("env X=1 codex exec {prompt}")),
        Agent::Generic
    );
    // A path or a word among the arguments is not the program.
    assert_eq!(
        wake_agent(Some("codex exec --tool /usr/bin/agy {prompt}")),
        Agent::Generic
    );
    assert_eq!(wake_agent(Some("agy {prompt}")), Agent::Agy);
    assert_eq!(wake_agent(Some("/usr/local/bin/agy {prompt}")), Agent::Agy);
    // Anything else is somebody else's agent, typed into without looking.
    assert_eq!(wake_agent(Some("codex exec {prompt}")), Agent::Generic);
    assert_eq!(wake_agent(Some("my-wrapper {prompt}")), Agent::Generic);
    assert_eq!(wake_agent(Some("claude-wrapper {prompt}")), Agent::Generic);
}

#[test]
fn a_worker_is_woken_only_when_the_message_needs_action() {
    assert!(should_wake_worker(
        "[question 20260927T123456Z] which screen?"
    ));
    assert!(should_wake_worker("[question:123] detail"));
    assert!(should_wake_worker("[Question 123] detail"));
    assert!(should_wake_worker(
        "[質問 20260927T123456Z] どの画面ですか？"
    ));
    assert!(should_wake_worker("[gate 20260927-123456] approve"));
    assert!(should_wake_worker("[Gate 123] changes"));
    assert!(should_wake_worker(
        "[linked 20260922T050000Z-x] this session is now a task's worker"
    ));

    // Plain notices, acknowledgements, words starting with gate/question, and empty subjects do not wake the worker.
    assert!(!should_wake_worker("[ack] received"));
    assert!(!should_wake_worker("[ACK] received"));
    assert!(!should_wake_worker("[gateway timeout] 504"));
    assert!(!should_wake_worker("[gatekeeper] alert"));
    assert!(!should_wake_worker("[questionnaire] please fill"));
    assert!(!should_wake_worker(
        "Filed as WID-98: https://github.com/..."
    ));
    assert!(!should_wake_worker(
        "Started: a worker on branch foo is running in another tab."
    ));
    assert!(!should_wake_worker(
        "No reply needed. Please carry on with your own task."
    ));
    assert!(!should_wake_worker("hello"));
    assert!(!should_wake_worker(""));
}

#[test]
fn the_hub_is_woken_for_actionable_messages_and_not_for_self_notes_or_acks() {
    let hub = "hub-main";

    // Actionable messages from outside wake the hub.
    assert!(should_wake_hub("worker-1", hub, "report", "found bug"));
    assert!(should_wake_hub(
        "worker-1",
        hub,
        "answer",
        "[question 123] answers here"
    ));
    assert!(should_wake_hub("worker-1", hub, "done", "cleanup"));
    assert!(should_wake_hub("dashboard", hub, "request", "start task"));
    assert!(should_wake_hub("board", hub, "next", "next"));
    assert!(should_wake_hub(
        "dashboard",
        hub,
        "gate",
        "[gate 123] approve"
    ));
    assert!(should_wake_hub("jules", hub, "jules-pr", "PR ready"));
    assert!(should_wake_hub("jules", hub, "jules-review", "new review"));
    assert!(should_wake_hub(
        "custom-sender",
        hub,
        "custom-kind",
        "action needed"
    ));
    assert!(should_wake_hub(
        "worker-1",
        hub,
        "",
        "empty kind defaults to report"
    ));

    // Notes left by the hub for itself or future users do not wake the hub.
    assert!(!should_wake_hub(
        "unknown",
        hub,
        "question",
        "[question 123] waiting for reply"
    ));
    assert!(!should_wake_hub(
        hub,
        hub,
        "question",
        "[question 123] waiting for reply"
    ));
    assert!(!should_wake_hub(
        "unknown",
        hub,
        "needs-user",
        "missing info"
    ));
    assert!(!should_wake_hub(hub, hub, "report", "self report"));

    // Acks and notices do not wake the hub.
    assert!(!should_wake_hub("worker-1", hub, "ack", "received"));
    assert!(!should_wake_hub("worker-1", hub, "notice", "plain notice"));
    assert!(!should_wake_hub(
        "worker-1",
        hub,
        "report",
        "[ack] received"
    ));
    assert!(!should_wake_hub(
        "worker-1",
        hub,
        "report",
        "[ACK:123] received"
    ));
}
