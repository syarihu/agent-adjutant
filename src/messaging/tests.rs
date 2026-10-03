use super::*;

use crate::testing::Sandbox;

/// The name this process answers to in `ps`. A record has to carry it for the presence
/// guard to accept the test binary as the session it names.
fn this_process_name() -> String {
    std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string()
}

/// Winners, losers, and launches that reached no answer at all.
fn tally(outcomes: &[Result<Claim, String>]) -> (usize, usize, usize) {
    let count = |f: fn(&Result<Claim, String>) -> bool| outcomes.iter().filter(|o| f(o)).count();
    (
        count(|o| matches!(o, Ok(Claim::Ours))),
        count(|o| matches!(o, Ok(Claim::Taken(_)))),
        count(|o| o.is_err()),
    )
}

fn claim_ours(slug: &str, hub_name: &str) {
    match claim_hub(slug, hub_name, "/src/widget", true, None, None).unwrap() {
        Claim::Ours => {}
        Claim::Taken(status) => panic!("expected to win the claim, but {status:?} holds it"),
    }
}

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
fn process_starts_keep_the_whole_start_time_and_skip_what_is_not_a_process_line() {
    let output = "    1 Mon Sep 30 10:43:37 2026\n\
        12345 水  9/30 10:43:37 2026\n\
         \n\
        garbage line\n\
        777\n";
    let starts = parse_process_starts(output);
    assert_eq!(
        starts.get(&1).map(String::as_str),
        Some("Mon Sep 30 10:43:37 2026")
    );
    assert_eq!(
        starts.get(&12345).map(String::as_str),
        Some("水  9/30 10:43:37 2026")
    );
    assert_eq!(starts.get(&777).map(String::as_str), Some(""));
    assert_eq!(starts.len(), 3);
    assert!(parse_process_starts("").is_empty());
}

#[test]
fn the_process_snapshot_agrees_with_asking_ps_about_one_pid() {
    let pid = std::process::id();
    let table = ProcessTable::snapshot();
    assert_eq!(table.started(pid), ps_started(pid));
    assert!(ProcessTable::each().started(pid).is_some());
}

#[test]
fn a_session_id_is_a_version_4_uuid_and_a_new_one_each_time() {
    let a = new_session_id().unwrap();
    let b = new_session_id().unwrap();
    assert_ne!(a, b);
    let parts: Vec<&str> = a.split('-').collect();
    assert_eq!(
        parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
        [8, 4, 4, 4, 12],
        "{a}"
    );
    assert!(parts[2].starts_with('4'), "{a}");
    assert!("89ab".contains(&parts[3][..1]), "{a}");
    assert!(a.chars().all(|c| c == '-' || c.is_ascii_hexdigit()), "{a}");
}

#[test]
fn a_hub_session_outlives_the_record_that_hub_stop_clears() {
    let _sandbox = Sandbox::empty();
    claim_ours("acme-widget", "adjutant-acme-widget");
    save_hub_session(
        "acme-widget",
        "acme/widget",
        None,
        "adjutant-acme-widget",
        "sid-1",
    )
    .unwrap();
    unregister_hub("acme-widget").unwrap();
    let saved = hub_session("acme-widget").expect("the session went with the record");
    assert_eq!(saved.session_id, "sid-1");
    assert_eq!(saved.hub, None);
    assert_eq!(saved.hub_name.as_deref(), Some("adjutant-acme-widget"));
}

#[test]
fn a_repositorys_resumable_hubs_are_listed_its_own_first() {
    let _sandbox = Sandbox::empty();
    save_hub_session(
        "w-alpha",
        "acme/widget",
        Some("ALPHA-1"),
        "adjutant-w-alpha",
        "a",
    )
    .unwrap();
    save_hub_session("w", "acme/widget", None, "adjutant-w", "b").unwrap();
    save_hub_session("other", "acme/other", None, "adjutant-other", "c").unwrap();
    let found = hub_sessions_for("acme/widget");
    assert_eq!(
        found
            .iter()
            .map(|s| s.session_id.as_str())
            .collect::<Vec<_>>(),
        ["b", "a"]
    );
}

#[test]
fn last_alive_is_only_an_answer_about_the_session_it_names() {
    let _sandbox = Sandbox::empty();
    assert_eq!(hub_last_alive("acme-widget", "sid-1"), None);
    touch_hub_session("acme-widget", "sid-1").unwrap();
    let last = hub_last_alive("acme-widget", "sid-1").expect("the beat was not recorded");
    assert!((now_secs() - last).abs() < 5, "{last}");
    // An old hub's server beating after a new hub saved its own session says nothing
    // about the new one.
    assert_eq!(hub_last_alive("acme-widget", "sid-2"), None);
    // And the beat is not a session: listing what can be resumed does not read it.
    assert!(hub_sessions_for("acme/widget").is_empty());
}

#[test]
fn a_worker_session_carries_what_the_worker_was_started_with() {
    let dir = tempfile::tempdir().unwrap();
    save_worker_session(
        dir.path(),
        "WID-1 fix",
        Some("ALPHA-1"),
        Some("WID-1"),
        "sid-w",
    )
    .unwrap();
    // `close` clears the presence record; the session is not its to clear.
    register_worker(
        dir.path(),
        "WID-1 fix",
        Some("ALPHA-1"),
        Some("WID-1"),
        None,
    )
    .unwrap();
    unregister_worker(dir.path()).unwrap();
    let saved = worker_session(dir.path()).unwrap();
    assert_eq!(saved.session_id, "sid-w");
    assert_eq!(saved.title.as_deref(), Some("WID-1 fix"));
    assert_eq!(saved.hub.as_deref(), Some("ALPHA-1"));
    assert_eq!(saved.task.as_deref(), Some("WID-1"));
}

#[test]
fn a_session_file_without_an_id_names_nothing_to_resume() {
    let dir = tempfile::tempdir().unwrap();
    let path = worker_session_path(dir.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, r#"{"sessionId": "  ", "title": "t"}"#).unwrap();
    assert_eq!(worker_session(dir.path()), None);
    std::fs::write(&path, "not json").unwrap();
    assert_eq!(worker_session(dir.path()), None);
}

#[test]
fn stamp_is_utc_and_sorts() {
    assert_eq!(utc_stamp(0), "19700101T000000Z");
    assert_eq!(utc_stamp(1_767_225_600), "20260101T000000Z");
    assert_eq!(utc_stamp(1_757_306_100), "20250908T043500Z");
    assert!(utc_stamp(1) < utc_stamp(2));
}

#[test]
fn stamp_handles_leap_days() {
    // 2024-02-29T12:00:00Z — a year that is a leap year despite the /100 rule biting in
    // the neighbouring centuries.
    assert_eq!(utc_stamp(1_709_208_000), "20240229T120000Z");
    // 2000-02-29T00:00:00Z — the /400 exception.
    assert_eq!(utc_stamp(951_782_400), "20000229T000000Z");
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
fn a_live_registration_reads_as_present() {
    let _sandbox = Sandbox::empty();
    // The current process is alive by definition; its command line is the test binary,
    // so that is the name the record has to carry for the guard to pass.
    let name = std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    claim_ours("acme-widget", &name);
    let status = hub_status("acme-widget", &name);
    assert!(status.present, "{status:?}");
    assert!(!status.stale);
    assert_eq!(status.pid, Some(std::process::id()));
}

#[test]
fn a_recycled_pid_is_not_the_hub() {
    let _sandbox = Sandbox::empty();
    // Alive, and the pid is right — but it started at a different moment, which is what
    // a recycled pid looks like and the one case that would otherwise report "present"
    // and drop the report on the floor.
    write_json(
        &hub_record_path("acme-widget"),
        &json!({
            "pid": std::process::id(),
            "hubName": this_process_name(),
            "cwd": "/src/widget",
            "psStarted": "Thu Jan  1 00:00:00 1970",
        }),
    )
    .unwrap();
    let status = hub_status("acme-widget", &this_process_name());
    assert!(!status.present);
    assert!(status.stale);
}

#[test]
fn no_record_at_all_is_absent_but_not_stale() {
    let _sandbox = Sandbox::empty();
    let status = hub_status("acme-widget", "adjutant-acme-widget");
    assert!(!status.present);
    assert!(!status.stale);
    assert_eq!(status.pid, None);
}

/// What `close` is allowed to conclude about a worktree, and every reading that used to
/// let it conclude "free" without evidence.
/// A start time that cannot anchor anything has to be the same answer as none at all,
/// in every one of the four places a record is read. Blank, the comparison each of them
/// makes fails against every live process, and "not the process I recorded" means
/// `Gone` at a hub's claim — a live hub's name handed to the next launcher — absent at
/// either presence check, and a live worker's worktree offered up for deletion.
/// `ps_started` never writes one; anything else that writes the file can.
#[test]
fn a_blank_start_time_is_no_anchor_in_any_of_the_four_readers() {
    let _sandbox = Sandbox::empty();
    let ours = std::process::id();
    let name = this_process_name();
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();

    for blank in ["", "   "] {
        write_json(
            &hub_record_path("acme-widget"),
            &json!({"pid": ours, "hubName": name, "cwd": "/", "psStarted": blank}),
        )
        .unwrap();
        // The claim: with nothing saying this pid was recycled, the name stays taken.
        assert!(
            matches!(holder(&hub_record_path("acme-widget")), Liveness::Alive),
            "{blank:?}"
        );
        match claim_hub("acme-widget", &name, "/", true, None, None).unwrap() {
            Claim::Taken(_) => {}
            Claim::Ours => panic!("{blank:?}: a live hub's name was taken away"),
        }

        // The presence check falls through to its other anchor — the name in the
        // command line — instead of reporting a running hub as gone.
        let status = hub_status("acme-widget", &name);
        assert!(status.present, "{blank:?}: {status:?}");
        assert!(!status.stale, "{blank:?}: {status:?}");

        // A worker has no second anchor, so what is left is whether the pid is there.
        write_json(
            &worker_record_path(worktree),
            &json!({"pid": ours, "title": "WID-957", "psStarted": blank}),
        )
        .unwrap();
        assert!(worker_status(worktree).present, "{blank:?}");

        // And the reader that is about to delete something wants more than a pid that
        // exists: with no anchor it declines to act at all.
        let WorkerRecord::Named(worker) = read_worker(worktree) else {
            panic!("{blank:?} still names a pid and should be read as naming one");
        };
        assert_eq!(worker.started, None, "{blank:?}");
        assert_eq!(worker_liveness(&worker), Liveness::CannotTell, "{blank:?}");
    }
}

#[test]
fn a_worker_record_is_only_read_as_nobody_there_when_it_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    assert!(matches!(read_worker(worktree), WorkerRecord::Absent));

    let record = worker_record_path(worktree);
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    for content in [
        // Not JSON at all.
        "{ this is not json".to_string(),
        // Parseable, and naming nobody. `holder` calls this `Gone` for the hub's
        // question; for this one it is a fail-open.
        "{}".to_string(),
        json!({"pid": null}).to_string(),
        json!({"pid": "1234"}).to_string(),
        json!({"pid": 0}).to_string(),
        // `as u32` would truncate this to 1 and go looking at init.
        json!({"pid": 4294967297u64}).to_string(),
    ] {
        std::fs::write(&record, &content).unwrap();
        assert!(
            matches!(read_worker(worktree), WorkerRecord::Unreadable),
            "{content} was read as an answer"
        );
    }

    register_worker(worktree, "WID-957", None, None, None).unwrap();
    let WorkerRecord::Named(worker) = read_worker(worktree) else {
        panic!("a record this process just wrote does not name it");
    };
    assert_eq!(worker.pid, std::process::id());
    assert_eq!(worker.title.as_deref(), Some("WID-957"));
    assert_eq!(worker_liveness(&worker), Liveness::Alive);

    // The record going away says nothing about the process. This is the one that
    // matters: it is what a `close` command that removed the record rather than the tab
    // looks like, and reading it as death deletes a live worker's worktree.
    unregister_worker(worktree).unwrap();
    assert_eq!(worker_liveness(&worker), Liveness::Alive);

    // A pid that is alive but started at another moment is a recycled pid, which is
    // gone in the only sense that matters.
    let recycled = WorkerIdentity {
        started: Some("Thu Jan  1 00:00:00 1970".to_string()),
        ..worker.clone()
    };
    assert_eq!(worker_liveness(&recycled), Liveness::Gone);

    // And a record with no start time in it — which `register_worker` writes when `ps`
    // would not answer at that moment — anchors nothing. The pid is in use; nothing
    // says it is still this worker's, and a tab is about to be closed on the answer.
    let unanchored = WorkerIdentity {
        started: None,
        ..worker.clone()
    };
    assert_eq!(worker_liveness(&unanchored), Liveness::CannotTell);
    // A blank one belongs with those two rather than with the real ones: compared as a
    // start time it matches no process alive, which would read as a recycled pid and
    // call this worker gone — the answer that clears the record and says the worktree
    // is free to delete.
    for content in [
        json!({"pid": std::process::id(), "psStarted": null}).to_string(),
        json!({"pid": std::process::id()}).to_string(),
        json!({"pid": std::process::id(), "psStarted": ""}).to_string(),
        json!({"pid": std::process::id(), "psStarted": "   "}).to_string(),
    ] {
        std::fs::write(&record, &content).unwrap();
        let WorkerRecord::Named(read_back) = read_worker(worktree) else {
            panic!("{content} names a pid and should be read as naming one");
        };
        assert_eq!(read_back.started, None);
        assert_eq!(
            worker_liveness(&read_back),
            Liveness::CannotTell,
            "{content}"
        );
    }
}

#[test]
fn a_record_is_only_cleared_while_it_still_names_the_worker_it_was_read_from() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    register_worker(worktree, "WID-957", None, None, None).unwrap();
    let WorkerRecord::Named(worker) = read_worker(worktree) else {
        panic!("a record this process just wrote does not name it");
    };

    // The window this closes: a worker whose tab was closed is observed gone, and a new
    // worker registers in the same worktree before the record is cleared. Clearing it
    // then reports a free worktree, and the next caller finds no record and agrees.
    register_worker_as(worktree, 4321, "WID-958");
    assert!(matches!(
        unregister_worker_if(worktree, &worker).unwrap(),
        Cleared::AnotherWorker
    ));
    assert!(
        worker_record_path(worktree).exists(),
        "another worker's record was cleared"
    );

    // Its own record it may clear, and a record already gone is the outcome it wanted.
    register_worker(worktree, "WID-957", None, None, None).unwrap();
    let WorkerRecord::Named(worker) = read_worker(worktree) else {
        panic!("a record this process just wrote does not name it");
    };
    assert!(matches!(
        unregister_worker_if(worktree, &worker).unwrap(),
        Cleared::Yes
    ));
    assert!(!worker_record_path(worktree).exists());
    // A record already gone is the end state this was asking for.
    assert!(matches!(
        unregister_worker_if(worktree, &worker).unwrap(),
        Cleared::Yes
    ));

    // A record that cannot be read as anybody's is not anybody's to delete either, and
    // says so in its own words rather than borrowing the newcomer's.
    std::fs::write(worker_record_path(worktree), "{}").unwrap();
    assert!(matches!(
        unregister_worker_if(worktree, &worker).unwrap(),
        Cleared::Unreadable
    ));
    assert!(worker_record_path(worktree).exists());
}

/// A record for a worker that is not this process, which `register_worker` cannot write.
fn register_worker_as(worktree: &Path, pid: u32, title: &str) {
    write_json(
        &worker_record_path(worktree),
        &json!({"pid": pid, "title": title, "psStarted": "Thu Jan  1 00:00:00 1970"}),
    )
    .unwrap();
}

/// The record's pid is put on a process that is not this one, so that a test which means
/// to read the record from outside the worker can.
fn as_another_process(worktree: &Path) {
    let path = worker_record_path(worktree);
    let mut record = read_json(&path).unwrap();
    record["pid"] = json!(1);
    write_json(&path, &record).unwrap();
}

/// A git checkout for `current_worktree` to answer about, its path as git prints it.
fn checkout(dir: &tempfile::TempDir) -> PathBuf {
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success()
    );
    std::fs::canonicalize(dir.path()).unwrap()
}

#[test]
fn hub_id_follows_the_record_for_the_worker_it_names() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let worktree = checkout(&dir);
    register_worker_as(&worktree, 4242, "t");
    let mut record = read_json(&worker_record_path(&worktree)).unwrap();
    record["hub"] = json!("moved-to");
    write_json(&worker_record_path(&worktree), &record).unwrap();

    // Started under one hub, linked to another since: the environment is the old word.
    unsafe { std::env::set_var(HUB_ENV, "started-under") };
    let answer = hub_id_with(None, Some(&worktree), |pid| pid == 4242);
    assert_eq!(answer.unwrap().as_deref(), Some("moved-to"));
    // Said outright still beats it.
    let answer = hub_id_with(Some("flag"), Some(&worktree), |pid| pid == 4242);
    assert_eq!(answer.unwrap().as_deref(), Some("flag"));
    // Nothing to ask when the two agree: a common case must not walk the process tree.
    unsafe { std::env::set_var(HUB_ENV, "moved-to") };
    let answer = hub_id_with(None, Some(&worktree), |_| panic!("asked for nothing"));
    assert_eq!(answer.unwrap().as_deref(), Some("moved-to"));
    unsafe { std::env::remove_var(HUB_ENV) };
}

#[test]
fn a_record_without_a_hub_answers_the_repository_hub_for_its_own_worker() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let worktree = checkout(&dir);
    register_worker_as(&worktree, 4242, "t");

    unsafe { std::env::set_var(HUB_ENV, "started-under") };
    let answer = hub_id_with(None, Some(&worktree), |pid| pid == 4242);
    assert_eq!(answer.unwrap(), None);
    unsafe { std::env::remove_var(HUB_ENV) };
}

#[test]
fn a_process_not_under_the_worker_keeps_the_hub_it_was_told() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let worktree = checkout(&dir);
    register_worker_as(&worktree, 4242, "t");
    let mut record = read_json(&worker_record_path(&worktree)).unwrap();
    record["hub"] = json!("moved-to");
    write_json(&worker_record_path(&worktree), &record).unwrap();

    // A hub running a command inside the worktree.
    unsafe { std::env::set_var(HUB_ENV, "the-hub") };
    let answer = hub_id_with(None, Some(&worktree), |_| false);
    assert_eq!(answer.unwrap().as_deref(), Some("the-hub"));
    // A record that cannot be read does not take the environment's answer with it.
    std::fs::write(worker_record_path(&worktree), "[").unwrap();
    let answer = hub_id_with(None, Some(&worktree), |_| true);
    assert_eq!(answer.unwrap().as_deref(), Some("the-hub"));
    unsafe { std::env::remove_var(HUB_ENV) };
}

#[test]
fn a_process_is_itself_but_not_init_or_a_stranger() {
    assert!(is_self_or_descendant_of(std::process::id()));
    assert!(!is_self_or_descendant_of(1));
    let mut stranger = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    // A child is below this process, not above it.
    assert!(!is_self_or_descendant_of(stranger.id()));
    let _ = stranger.kill();
    let _ = stranger.wait();
}

#[test]
fn relinking_keeps_the_process_fields_and_sets_task_and_hub() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    register_worker(worktree, "try-retry", None, None, None).unwrap();
    let before = read_json(&worker_record_path(worktree)).unwrap();
    save_worker_session(worktree, "try-retry", None, None, "sid-1").unwrap();

    relink_worker(worktree, Some("WID-957"), "task-1", None).unwrap();
    let after = read_json(&worker_record_path(worktree)).unwrap();
    for key in ["pid", "psStarted", "startedAt", "title"] {
        assert_eq!(after[key], before[key], "{key}");
    }
    assert_eq!(after["task"], "task-1");
    assert_eq!(after["hub"], "WID-957");
    assert_eq!(after["phase"], "implement");
    assert!(after["phaseAt"].as_i64().is_some());
    let saved = worker_session(worktree).unwrap();
    assert_eq!(saved.session_id, "sid-1");
    assert_eq!(saved.hub.as_deref(), Some("WID-957"));
    assert_eq!(saved.task.as_deref(), Some("task-1"));

    // Back to the repository's own hub: no key at all, and a phase already there stays.
    set_worker_phase(worktree, "verify").unwrap();
    relink_worker(worktree, None, "task-2", None).unwrap();
    let after = read_json(&worker_record_path(worktree)).unwrap();
    assert!(after.get("hub").is_none());
    assert_eq!(after["phase"], "verify");
    assert_eq!(after["task"], "task-2");
    assert_eq!(worker_session(worktree).unwrap().hub, None);

    // A phase named by the caller is entered even when the record already has one.
    relink_worker(worktree, None, "task-2", Some("plan")).unwrap();
    let after = read_json(&worker_record_path(worktree)).unwrap();
    assert_eq!(after["phase"], "plan");
    assert_eq!(
        after["phases"].as_array().unwrap().last().unwrap()[0],
        "plan"
    );
}

/// Which hub an invocation is addressing, and the order the three answers are asked in.
///
/// The order is the whole design. A worker's agent is told never to write down an
/// address (`adj-report` promises it), so the record answers for it; a hub's agent
/// calls every tool with no arguments at all, so the environment answers for it; and
/// anything either of them is told outright has to beat both.
#[test]
fn the_hub_being_addressed_is_said_outright_inherited_or_read_from_the_worktree() {
    // Held for the environment lock rather than for the config: this test writes
    // `ADJUTANT_HUB`, and every other test's answers depend on it not being set.
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    // `current_worktree` asks git, so there has to be something for it to answer about.
    // Canonicalised because git reports the resolved path and macOS puts tempdirs
    // behind /private.
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success()
    );
    let worktree = std::fs::canonicalize(dir.path()).unwrap();
    let worktree = worktree.as_path();

    // Nobody said anything and nothing dispatched this: the repository's own hub.
    assert_eq!(hub_id(None, Some(worktree)).unwrap(), None);

    // The record alone. This is the worker's case, and the only one where the answer
    // comes from where the caller is standing rather than from what it was told.
    // Written as a worker that is not this process: `register_worker` writes this one's
    // own pid, and a process that is the recorded worker follows its record over the
    // environment (`hub_id_follows_the_record_for_the_worker_it_names`).
    register_worker(worktree, "WID-957", Some("from-record"), None, None).unwrap();
    as_another_process(worktree);
    assert_eq!(
        hub_id(None, Some(worktree)).unwrap().as_deref(),
        Some("from-record")
    );

    // The environment beats the record. A hub running a command inside one of its
    // workers' worktrees is still itself, and reading the worktree there would have it
    // addressing a hub on the strength of where it happened to `cd`.
    unsafe { std::env::set_var(HUB_ENV, "from-env") };
    assert_eq!(
        hub_id(None, Some(worktree)).unwrap().as_deref(),
        Some("from-env")
    );

    // And being told outright beats both.
    assert_eq!(
        hub_id(Some("from-flag"), Some(worktree))
            .unwrap()
            .as_deref(),
        Some("from-flag")
    );

    // Blank is silence at every one of the three, so the next answer down is taken. A
    // flag passed through without a value would otherwise build an address whose
    // identifier is the empty string — a hub nobody can name a second time.
    assert_eq!(
        hub_id(Some("   "), Some(worktree)).unwrap().as_deref(),
        Some("from-env")
    );
    unsafe { std::env::set_var(HUB_ENV, "") };
    assert_eq!(
        hub_id(None, Some(worktree)).unwrap().as_deref(),
        Some("from-record")
    );
    unsafe { std::env::remove_var(HUB_ENV) };

    // The record answers for the side that *addresses* a hub and never for the side
    // that starts or registers one. `adj hub` may be typed inside a worktree and `adj
    // worker` runs in a tab opened at the worktree it is about to register in, so
    // reading a record there is reading somebody else's answer, or one's own from a
    // previous life.
    register_worker(worktree, "WID-957", Some("from-record"), None, None).unwrap();
    as_another_process(worktree);
    assert_eq!(
        hub_id(None, Some(worktree)).unwrap().as_deref(),
        Some("from-record")
    );
    assert_eq!(hub_id_told(None), None);
    unsafe { std::env::set_var(HUB_ENV, "from-env") };
    assert_eq!(hub_id_told(None).as_deref(), Some("from-env"));
    assert_eq!(hub_id_told(Some("from-flag")).as_deref(), Some("from-flag"));
    unsafe { std::env::remove_var(HUB_ENV) };

    // A worker dispatched by a repository's own hub records no identifier at all, and
    // the key is absent rather than null: a record this version writes has to read the
    // same way to every other version of this tool on the machine.
    register_worker(worktree, "WID-957", None, None, None).unwrap();
    assert_eq!(hub_id(None, Some(worktree)).unwrap(), None);
    assert!(
        read_json(&worker_record_path(worktree))
            .unwrap()
            .get("hub")
            .is_none()
    );
}

/// A record that cannot be read is not a record that says nothing.
///
/// The two readings differ by exactly the failure this is all built to stop: falling
/// back to the repository's own hub in a worktree somebody dispatched sends every
/// report that worker files to an inbox that may have no hub reading it. So the damaged
/// record is raised, and the caller keeps the one way out that never touches it.
#[test]
fn a_record_that_cannot_be_read_is_not_read_as_the_repository_s_own_hub() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success()
    );
    let worktree = std::fs::canonicalize(dir.path()).unwrap();
    let worktree = worktree.as_path();

    // No record at all: nobody dispatched this, and the repository's own hub is the
    // right answer. This is the reading the damaged ones must not share.
    assert_eq!(hub_id(None, Some(worktree)).unwrap(), None);

    let path = worker_record_path(worktree);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // Truncated, the wrong shape, and the right shape with the wrong kind of name in
    // it: three ways a record stops naming a hub, none of them a repository's own.
    for damaged in [
        "{\"pid\": 1, \"hub\": \"wid-957",
        "[\"wid-957\"]",
        "{\"pid\": 1, \"hub\": 957}",
    ] {
        std::fs::write(&path, damaged).unwrap();
        let said = hub_id(None, Some(worktree)).unwrap_err();
        assert!(said.contains(&path.display().to_string()), "{said}");

        // And the way out, which is the reason this is an error and not a stop: being
        // told outright is settled before the record is opened.
        assert_eq!(
            hub_id(Some("from-flag"), Some(worktree))
                .unwrap()
                .as_deref(),
            Some("from-flag")
        );
        unsafe { std::env::set_var(HUB_ENV, "from-env") };
        assert_eq!(
            hub_id(None, Some(worktree)).unwrap().as_deref(),
            Some("from-env")
        );
        unsafe { std::env::remove_var(HUB_ENV) };
    }

    // A hub written as null is the absent key by another name, not damage: this
    // version writes no key at all, and a record has to read the same way to every
    // other version of this tool on the machine.
    std::fs::write(&path, "{\"pid\": 1, \"hub\": null}").unwrap();
    assert_eq!(hub_id(None, Some(worktree)).unwrap(), None);
}

#[test]
fn unregister_is_idempotent() {
    let _sandbox = Sandbox::empty();
    unregister_hub("acme-widget").unwrap();
    claim_ours("acme-widget", "adjutant-acme-widget");
    unregister_hub("acme-widget").unwrap();
    unregister_hub("acme-widget").unwrap();
    assert!(!hub_record_path("acme-widget").exists());
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

/// Five launches at once used to produce three hubs, and a repository with three hubs
/// makes it luck which one a report reaches.
#[test]
fn only_one_of_five_launches_becomes_the_hub() {
    let _sandbox = Sandbox::empty();
    let name = this_process_name();
    let outcomes: Vec<Result<Claim, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..5)
            .map(|_| {
                scope.spawn(|| claim_hub("acme-widget", &name, "/src/widget", true, None, None))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    // Every launch has to reach an *answer*. Counting only the winners would pass a
    // run where one won and the other four failed outright.
    assert_eq!(tally(&outcomes), (1, 4, 0), "{outcomes:?}");
    assert!(hub_status("acme-widget", &name).present);
}

/// A record whose process is gone is not a hub, and must not block the next one.
#[test]
fn a_dead_hubs_record_does_not_hold_the_name() {
    let _sandbox = Sandbox::empty();
    // A pid that really has exited, rather than this process wearing a wrong name —
    // "alive but not recognised" is a different thing entirely, and the test below is
    // the one that means it.
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    write_json(
        &hub_record_path("acme-widget"),
        &json!({
            "pid": dead,
            "hubName": "adjutant-acme-widget",
            "cwd": "/src/widget",
            "psStarted": "whenever it was",
            "nameInCommand": true,
        }),
    )
    .unwrap();

    let name = this_process_name();
    claim_ours("acme-widget", &name);
    assert!(hub_status("acme-widget", &name).present);
}

/// A hub whose name never reaches its own command line is still a hub.
///
/// The window between winning the claim and `exec`ing the agent is one way to be in
/// that state; a `hubRunner` like `env NAME={name} agent …`, where `exec` keeps none of
/// the wrapper's arguments, is a permanent one. Presence rests on the recorded start
/// time, which `exec` preserves and a recycled pid cannot match, so neither case is
/// mistaken for a session that has gone.
#[test]
fn a_hub_whose_name_is_not_in_its_command_line_is_still_present() {
    let _sandbox = Sandbox::empty();
    write_json(
        &hub_record_path("acme-widget"),
        &json!({
            "pid": std::process::id(),
            // Nothing on this machine answers to this name.
            "hubName": "adjutant-acme-widget",
            "cwd": "/src/widget",
            "psStarted": ps_started(std::process::id()),
            "nameInCommand": true,
        }),
    )
    .unwrap();
    let status = hub_status("acme-widget", "adjutant-acme-widget");
    assert!(status.present, "{status:?}");
    assert!(!status.stale);
    // And a launch arriving now is told someone holds it, rather than taking it.
    assert!(matches!(
        claim_hub(
            "acme-widget",
            "adjutant-acme-widget",
            "/src/widget",
            true,
            None,
            None
        ),
        Ok(Claim::Taken(_))
    ));
}

/// Taking over a dead hub's record is itself a race, and it used to be lost the same
/// way the original was: both launchers read the dead record, the first deletes it and
/// claims, and the second deletes *that* — a live record — and claims too.
#[test]
fn five_launches_inheriting_one_dead_record_still_leave_one_hub() {
    let _sandbox = Sandbox::empty();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    write_json(
        &hub_record_path("acme-widget"),
        &json!({
            "pid": dead,
            "hubName": "adjutant-acme-widget",
            "cwd": "/src/widget",
            "psStarted": "whenever it was",
        }),
    )
    .unwrap();

    let name = this_process_name();
    let outcomes: Vec<Result<Claim, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..5)
            .map(|_| {
                scope.spawn(|| claim_hub("acme-widget", &name, "/src/widget", true, None, None))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let (ours, taken, refused) = tally(&outcomes);
    // The property that matters is that two launches never both become the hub. A
    // launch may also legitimately reach no answer at all — `ps` is a process too, and
    // under load it can fail to start, which is deliberately read as "leave the name
    // alone" rather than as "the holder is gone".
    assert!(ours <= 1, "{outcomes:?}");
    assert_eq!(ours + taken + refused, 5, "{outcomes:?}");
    if refused == 0 {
        assert_eq!(ours, 1, "{outcomes:?}");
    }
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

/// The takeover lock is held by the operating system, so a second claimer is turned
/// away rather than left to decide for itself whether the first one is still alive.
#[test]
fn a_takeover_in_progress_turns_the_next_claim_away() {
    let _sandbox = Sandbox::empty();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    let record = hub_record_path("acme-widget");
    write_json(
        &record,
        &json!({"pid": dead, "hubName": "adjutant-acme-widget", "psStarted": "long ago"}),
    )
    .unwrap();

    // Someone else is inside the takeover.
    let lock_path = record.with_extension("claiming");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .unwrap();
    lock.lock().unwrap();

    let name = this_process_name();
    assert!(matches!(
        claim_hub("acme-widget", &name, "/src/widget", true, None, None),
        Ok(Claim::Taken(_))
    ));

    // And once they are done, the next claim gets it.
    drop(lock);
    claim_ours("acme-widget", &name);
}

// ── worker slots ─────────────────────────────────────────────────

#[test]
fn a_worker_that_is_there_holds_a_slot_and_a_dead_one_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    assert!(!holds_worker_slot(worktree, now_secs()));

    register_worker(worktree, "WID-957", None, None, None).unwrap();
    assert!(holds_worker_slot(worktree, now_secs()));

    // A pid that is not running. Counting it would hold the slot for good, since nothing
    // is left to send the `done` that would free it. A process that has been and gone
    // rather than a made-up number: `ps` refuses a pid out of range, which is `CannotTell`.
    let mut exited = std::process::Command::new("true").spawn().unwrap();
    let pid = exited.id();
    exited.wait().unwrap();
    write_json(
        &worker_record_path(worktree),
        &json!({"pid": pid, "title": "WID-957", "psStarted": "Thu Jan  1 00:00:00 1970"}),
    )
    .unwrap();
    assert!(!holds_worker_slot(worktree, now_secs()));

    // A running pid with no start time to check it against cannot be told from whatever
    // inherited the number. Counted busy: a free slot that was not is how a limit is
    // overshot, and this holds only until that pid stops.
    write_json(
        &worker_record_path(worktree),
        &json!({"pid": std::process::id(), "title": "WID-957"}),
    )
    .unwrap();
    assert!(holds_worker_slot(worktree, now_secs()));
}

#[test]
fn a_pid_out_of_range_is_nobody_to_the_status_as_well_as_to_the_slot_count() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    write_json(
        &worker_record_path(worktree),
        &json!({"pid": 4294967297u64, "title": "WID-957"}),
    )
    .unwrap();
    let status = worker_status(worktree);
    assert_eq!(status.pid, None);
    assert!(!status.present);
    assert!(!holds_worker_slot(worktree, now_secs()));
}

#[test]
fn a_worker_still_starting_holds_a_slot_until_it_registers_or_the_grace_runs_out() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    mark_worker_starting(worktree).unwrap();
    let now = now_secs();
    // Not registered yet, which is every worker in the seconds after `adj work` returns.
    assert!(holds_worker_slot(worktree, now));
    // A tab that never opened stops counting.
    assert!(!holds_worker_slot(worktree, now + STARTING_GRACE_SECS));
    // Nor does a marker from the future, which is a clock set back since the dispatch.
    assert!(!holds_worker_slot(worktree, now - 3600));

    // Registering takes the marker away: from then on the record answers.
    register_worker(worktree, "WID-957", None, None, None).unwrap();
    assert!(!starting_marker_path(worktree).exists());
    unregister_worker(worktree).unwrap();
    assert!(!holds_worker_slot(worktree, now));
}

#[test]
fn the_worktree_being_dispatched_into_does_not_count_against_itself() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    for worktree in [&a, &b] {
        mark_worker_starting(worktree).unwrap();
    }
    let listed: Vec<String> = [&a, &b]
        .iter()
        .map(|p| p.canonicalize().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(busy_worktrees(&listed, None).len(), 2);
    // Named the way a caller might hold it, not the way git prints it.
    assert_eq!(busy_worktrees(&listed, Some(&a)), vec![listed[1].clone()]);
}

#[test]
fn a_dispatch_lock_is_released_with_its_holder_and_keeps_others_out_meanwhile() {
    let dir = tempfile::tempdir().unwrap();
    // A lock file left on disk is not a lock: only a handle holding it is.
    let lock = dir.path().join(".claude").join("adjutant-dispatch.lock");
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    std::fs::File::create(&lock).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(with_dispatch_lock(dir.path(), || 7).unwrap(), 7);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));

    // While one caller is inside, another handle on the same file cannot take it.
    with_dispatch_lock(dir.path(), || {
        let other = std::fs::OpenOptions::new().write(true).open(&lock).unwrap();
        assert!(matches!(
            other.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
    })
    .unwrap();

    // And once it is out, the next one does not wait.
    let started = std::time::Instant::now();
    with_dispatch_lock(dir.path(), || ()).unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn every_phase_said_is_kept_in_order_and_carried_to_a_restarted_worker() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    register_worker(worktree, "WID-957", None, None, None).unwrap();
    set_worker_phase(worktree, "plan").unwrap();
    set_worker_phase(worktree, "plan").unwrap();
    set_worker_phase(worktree, "verify").unwrap();
    fn names(status: &WorkerStatus) -> Vec<&str> {
        status.phases.iter().map(|(p, _)| p.as_str()).collect()
    }
    let status = worker_status(worktree);
    assert_eq!(names(&status), ["plan", "plan", "verify"]);
    assert_eq!(status.phases[2].1, status.phase_at.unwrap());

    // Restarted: the current phase is gone, the timeline is not.
    register_worker(worktree, "WID-957", None, None, None).unwrap();
    let status = worker_status(worktree);
    assert_eq!(status.phase, None);
    assert_eq!(names(&status), ["plan", "plan", "verify"]);

    // The same worktree taken for another task does not inherit the first one's timeline.
    register_worker(worktree, "WID-957", None, Some("task-2"), None).unwrap();
    assert!(worker_status(worktree).phases.is_empty());
    register_worker(worktree, "WID-957", None, None, None).unwrap();
    assert!(worker_status(worktree).phases.is_empty());
    register_worker(worktree, "WID-957", None, None, None).unwrap();
    set_worker_phase(worktree, "plan").unwrap();
    register_worker(worktree, "WID-957", None, None, None).unwrap();
    assert_eq!(names(&worker_status(worktree)), ["plan"]);

    // A record from before the history was kept starts it from the phase it has.
    let path = worker_record_path(worktree);
    let mut record = read_json(&path).unwrap();
    let fields = record.as_object_mut().unwrap();
    fields.remove("phases");
    fields.insert("phase".into(), json!("pr"));
    fields.insert("phaseAt".into(), json!(1_700_000_000));
    write_json(&path, &record).unwrap();
    assert_eq!(
        worker_status(worktree).phases,
        [("pr".to_string(), 1_700_000_000)]
    );
    set_worker_phase(worktree, "review").unwrap();
    assert_eq!(names(&worker_status(worktree)), ["pr", "review"]);
}

#[test]
fn the_phase_history_drops_its_oldest_entries_past_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    register_worker(worktree, "WID-957", None, None, None).unwrap();
    for i in 0..PHASES_KEPT + 6 {
        set_worker_phase(worktree, PHASES[i % 2]).unwrap();
    }
    let phases = worker_status(worktree).phases;
    assert_eq!(phases.len(), PHASES_KEPT);
    assert_eq!(phases.last().unwrap().0, PHASES[(PHASES_KEPT + 5) % 2]);
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
fn a_phase_is_written_into_the_workers_own_record_and_a_typo_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    // Nobody registered: there is no run of a worker to describe.
    assert!(set_worker_phase(worktree, "plan").is_err());

    register_worker(worktree, "WID-957", None, None, None).unwrap();
    set_worker_phase(worktree, "implement").unwrap();
    let status = worker_status(worktree);
    assert_eq!(status.phase.as_deref(), Some("implement"));
    assert!(
        status
            .phase_at
            .is_some_and(|at| (now_secs() - at).abs() < 5)
    );
    assert!(
        status.present,
        "writing the phase must not disturb the record"
    );

    assert!(set_worker_phase(worktree, "implementing").is_err());
    // A worker started again writes its record fresh, and starts without a phase.
    register_worker(worktree, "WID-957", None, None, None).unwrap();
    assert_eq!(worker_status(worktree).phase, None);
}

#[test]
#[cfg(unix)]
fn a_worker_record_that_is_a_broken_symlink_is_unreadable_not_absent() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    let record = worker_record_path(worktree);
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(worktree.join("gone.json"), &record).unwrap();
    assert!(matches!(read_worker(worktree), WorkerRecord::Unreadable));
}

#[test]
#[cfg(unix)]
fn a_worker_record_that_is_a_broken_symlink_is_not_read_as_the_repository_s_own_hub() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success()
    );
    let worktree = std::fs::canonicalize(dir.path()).unwrap();
    let worktree = worktree.as_path();

    let path = worker_record_path(worktree);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(worktree.join("gone.json"), &path).unwrap();

    let err = hub_id(None, Some(worktree)).unwrap_err();
    assert!(err.contains(&path.display().to_string()));
    assert_eq!(
        hub_id(Some("from-flag"), Some(worktree))
            .unwrap()
            .as_deref(),
        Some("from-flag")
    );
}

#[test]
#[cfg(unix)]
fn a_hub_record_that_is_a_broken_symlink_is_cannot_tell_not_gone() {
    let _sandbox = Sandbox::empty();
    let slug = "acme-widget";
    assert_eq!(hub_liveness(slug), Liveness::Gone);
    let record = hub_record_path(slug);
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(record.with_file_name("gone.json"), &record).unwrap();
    assert_eq!(hub_liveness(slug), Liveness::CannotTell);
}

#[test]
fn all_repo_hubs_discovers_all_sources_and_sorts_repo_first() {
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

    // 1. Repo hub record in hubs/
    write_json(
        &hub_record_path("acme-widget"),
        &json!({
            "hubName": "adjutant-acme-widget",
            "cwd": main_path,
        }),
    )
    .unwrap();

    // 2. Parent task hub in hubs/
    let parent_slug = crate::kernel::identity::slug_for("acme/widget", Some("parent-task"));
    write_json(
        &hub_record_path(&parent_slug),
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
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let slug = crate::kernel::identity::slug_for("acme/widget", Some("WID-957"));
    save_hub_session(
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
    assert_eq!(hub_sessions_for("acme/widget").len(), 1);
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
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    write_json(&worker_record_path(dir.path()), &json!({"hub": "WID-957"})).unwrap();
    let slug = crate::kernel::identity::slug_for("acme/widget", Some("wid-957"));
    save_hub_session(
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
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    // Dispatched by A, moved to B: the record says B, the session still says A.
    write_json(&worker_record_path(dir.path()), &json!({"hub": "B"})).unwrap();
    save_worker_session(dir.path(), "t", Some("A"), None, "sid-1").unwrap();
    let a = crate::kernel::identity::slug_for("acme/widget", Some("A"));
    save_hub_session(&a, "acme/widget", Some("A"), "adjutant-a", "sess-a").unwrap();

    let hubs = all_repo_hubs(&widget_repo(dir.path()));
    let keys: Vec<_> = hubs.iter().map(|h| h.key.as_deref()).collect();
    assert_eq!(keys, vec![None, Some("B")], "{hubs:?}");
    assert_eq!(hubs[1].children, 1);
}

#[test]
fn worker_hub_key_prefers_the_record_and_skips_a_blank_one() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(worker_hub_key(dir.path()), None);
    save_worker_session(dir.path(), "t", Some(" A "), None, "sid-1").unwrap();
    assert_eq!(worker_hub_key(dir.path()).as_deref(), Some("A"));
    write_json(&worker_record_path(dir.path()), &json!({"hub": "  "})).unwrap();
    assert_eq!(worker_hub_key(dir.path()).as_deref(), Some("A"));
    write_json(&worker_record_path(dir.path()), &json!({"hub": "B"})).unwrap();
    assert_eq!(worker_hub_key(dir.path()).as_deref(), Some("B"));
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
    // Written by a version that did not record `hub`: the key comes back out of the slug.
    let readable = crate::kernel::identity::slug_for("acme/widget", Some("WID-100"));
    write_json(
        &hub_record_path(&readable),
        &json!({"hubName": format!("adjutant-{readable}"), "cwd": main_path}),
    )
    .unwrap();
    // And one whose key the slug cannot give back: still a parent hub, key unknown.
    let lossy = crate::kernel::identity::slug_for("acme/widget", Some("v1.2"));
    write_json(
        &hub_record_path(&lossy),
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
fn register_worker_records_where_the_worker_runs() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    let at = crate::infra::terminal::SessionTerminal {
        backend: "tmux".into(),
        socket: Some("/tmp/tmux-501/default".into()),
        session: Some("adjutant".into()),
        window: Some("@3".into()),
        pane: Some("%7".into()),
    };
    register_worker(worktree, "WID-957", None, None, Some(&at)).unwrap();
    let record = read_json(&worker_record_path(worktree)).unwrap();
    assert_eq!(
        record["terminal"],
        json!({
            "backend": "tmux",
            "socket": "/tmp/tmux-501/default",
            "session": "adjutant",
            "window": "@3",
            "pane": "%7",
        })
    );
    // A phase written later keeps it.
    set_worker_phase(worktree, "plan").unwrap();
    let record = read_json(&worker_record_path(worktree)).unwrap();
    assert_eq!(record["terminal"]["window"], "@3");
}

#[test]
fn register_worker_persists_task_id_when_provided() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    register_worker(
        worktree,
        "WID-957",
        Some("hub-1"),
        Some("task-wid-957"),
        None,
    )
    .unwrap();
    let record = read_json(&worker_record_path(worktree)).unwrap();
    assert_eq!(
        record.get("task").and_then(Value::as_str),
        Some("task-wid-957")
    );
    assert_eq!(record.get("hub").and_then(Value::as_str), Some("hub-1"));
    assert_eq!(record.get("title").and_then(Value::as_str), Some("WID-957"));
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
fn unregister_hub_if_unnamed_removes_only_a_record_naming_no_process() {
    let _sandbox = Sandbox::empty();
    let path = hub_record_path("acme-widget");
    assert!(unregister_hub_if_unnamed("acme-widget").unwrap());

    write_json(
        &path,
        &json!({"pid": 4242, "psStarted": "Mon Jan  1 00:00:00 2024"}),
    )
    .unwrap();
    assert!(!unregister_hub_if_unnamed("acme-widget").unwrap());
    assert!(path.exists());

    write_json(&path, &json!({"hubName": "adjutant-acme-widget"})).unwrap();
    assert!(unregister_hub_if_unnamed("acme-widget").unwrap());
    assert!(!path.exists());
}

#[test]
fn unregister_hub_if_leaves_a_record_naming_another_process() {
    let _sandbox = Sandbox::empty();
    let path = hub_record_path("acme-widget");
    let record = json!({"pid": 4242, "psStarted": "Mon Jan  1 00:00:00 2024"});
    write_json(&path, &record).unwrap();

    // Another start time is another process on a recycled pid, and another pid is
    // another hub: neither is this call's to clear.
    assert_eq!(
        unregister_hub_if("acme-widget", 4242, Some("later")),
        Ok(false)
    );
    assert_eq!(
        unregister_hub_if("acme-widget", 4243, Some("Mon Jan  1 00:00:00 2024")),
        Ok(false)
    );
    assert!(path.exists());

    assert_eq!(
        unregister_hub_if("acme-widget", 4242, Some("Mon Jan  1 00:00:00 2024")),
        Ok(true)
    );
    assert!(!path.exists());
    // Already gone is the same end state.
    assert_eq!(unregister_hub_if("acme-widget", 4242, None), Ok(true));
}

#[test]
fn a_claimed_hub_record_carries_where_it_runs() {
    let _sandbox = Sandbox::empty();
    let terminal = crate::infra::terminal::SessionTerminal {
        backend: "tmux".into(),
        socket: Some("scratch".into()),
        session: Some("adj".into()),
        window: Some("@1".into()),
        pane: Some("%3".into()),
    };
    match claim_hub(
        "acme-widget",
        "adjutant-acme-widget",
        "/src/widget",
        true,
        None,
        Some(&terminal),
    )
    .unwrap()
    {
        Claim::Ours => {}
        Claim::Taken(status) => panic!("{status:?}"),
    }
    let record = read_json(&hub_record_path("acme-widget")).unwrap();
    assert_eq!(record["terminal"]["socket"], "scratch");
    assert_eq!(record["terminal"]["pane"], "%3");
    assert_eq!(record["terminal"]["backend"], "tmux");

    // A launch that does not know where it is leaves the field out, as records always were.
    unregister_hub("acme-widget").unwrap();
    claim_ours("acme-widget", "adjutant-acme-widget");
    assert!(
        read_json(&hub_record_path("acme-widget"))
            .unwrap()
            .get("terminal")
            .is_none()
    );
}

#[test]
fn a_removing_mark_is_found_by_the_spelling_it_was_made_with_and_after_the_directory_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main");
    std::fs::create_dir_all(&main).unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir_all(real.join("wt")).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let mark = mark_worktree_removing(&main, &real.join("wt")).unwrap();
    // Another spelling of the same directory finds it, while it is there and once it is not.
    assert!(is_being_removed(&main, &link.join("wt")));
    std::fs::remove_dir(real.join("wt")).unwrap();
    assert!(is_being_removed(&main, &link.join("wt")));
    assert!(is_being_removed(&main, &real.join("wt")));

    drop(mark);
    assert!(!is_being_removed(&main, &link.join("wt")));
    assert!(!is_being_removed(&main, &real.join("wt")));
}
