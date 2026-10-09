use super::store::{starting_marker_path, worker_session_path};
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

fn claim_ours(root: &Path, slug: &str, hub_name: &str) {
    match claim_hub(root, slug, hub_name, "/src/widget", true, None, None).unwrap() {
        Claim::Ours => {}
        Claim::Taken(status) => panic!("expected to win the claim, but {status:?} holds it"),
    }
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
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    claim_ours(&root, "acme-widget", "adjutant-acme-widget");
    save_hub_session(
        &root,
        "acme-widget",
        "acme/widget",
        None,
        "adjutant-acme-widget",
        "sid-1",
    )
    .unwrap();
    unregister_hub(&root, "acme-widget").unwrap();
    let saved = hub_session(&root, "acme-widget").expect("the session went with the record");
    assert_eq!(saved.session_id, "sid-1");
    assert_eq!(saved.hub, None);
    assert_eq!(saved.hub_name.as_deref(), Some("adjutant-acme-widget"));
}

#[test]
fn a_repositorys_resumable_hubs_are_listed_its_own_first() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    save_hub_session(
        &root,
        "w-alpha",
        "acme/widget",
        Some("ALPHA-1"),
        "adjutant-w-alpha",
        "a",
    )
    .unwrap();
    save_hub_session(&root, "w", "acme/widget", None, "adjutant-w", "b").unwrap();
    save_hub_session(&root, "other", "acme/other", None, "adjutant-other", "c").unwrap();
    let found = hub_sessions_for(&root, "acme/widget");
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
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    assert_eq!(hub_last_alive(&root, "acme-widget", "sid-1"), None);
    touch_hub_session(&root, "acme-widget", "sid-1").unwrap();
    let last = hub_last_alive(&root, "acme-widget", "sid-1").expect("the beat was not recorded");
    assert!((now_secs() - last).abs() < 5, "{last}");
    // An old hub's server beating after a new hub saved its own session says nothing
    // about the new one.
    assert_eq!(hub_last_alive(&root, "acme-widget", "sid-2"), None);
    // And the beat is not a session: listing what can be resumed does not read it.
    assert!(hub_sessions_for(&root, "acme/widget").is_empty());
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
fn a_live_registration_reads_as_present() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    // The current process is alive by definition; its command line is the test binary,
    // so that is the name the record has to carry for the guard to pass.
    let name = std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    claim_ours(&root, "acme-widget", &name);
    let status = hub_status(&root, "acme-widget", &name);
    assert!(status.present, "{status:?}");
    assert!(!status.stale);
    assert_eq!(status.pid, Some(std::process::id()));
}

#[test]
fn a_recycled_pid_is_not_the_hub() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    // Alive, and the pid is right — but it started at a different moment, which is what
    // a recycled pid looks like and the one case that would otherwise report "present"
    // and drop the report on the floor.
    write_json(
        &hub_record_path(&root, "acme-widget"),
        &json!({
            "pid": std::process::id(),
            "hubName": this_process_name(),
            "cwd": "/src/widget",
            "psStarted": "Thu Jan  1 00:00:00 1970",
        }),
    )
    .unwrap();
    let status = hub_status(&root, "acme-widget", &this_process_name());
    assert!(!status.present);
    assert!(status.stale);
}

#[test]
fn no_record_at_all_is_absent_but_not_stale() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let status = hub_status(&root, "acme-widget", "adjutant-acme-widget");
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
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let ours = std::process::id();
    let name = this_process_name();
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();

    for blank in ["", "   "] {
        write_json(
            &hub_record_path(&root, "acme-widget"),
            &json!({"pid": ours, "hubName": name, "cwd": "/", "psStarted": blank}),
        )
        .unwrap();
        // The claim: with nothing saying this pid was recycled, the name stays taken.
        assert!(
            matches!(
                holder(&hub_record_path(&root, "acme-widget")),
                Liveness::Alive
            ),
            "{blank:?}"
        );
        match claim_hub(&root, "acme-widget", &name, "/", true, None, None).unwrap() {
            Claim::Taken(_) => {}
            Claim::Ours => panic!("{blank:?}: a live hub's name was taken away"),
        }

        // The presence check falls through to its other anchor — the name in the
        // command line — instead of reporting a running hub as gone.
        let status = hub_status(&root, "acme-widget", &name);
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
        let Recorded::Found(worker) = read_worker(worktree) else {
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
    assert!(matches!(read_worker(worktree), Recorded::Absent));

    let record = worker_record_path(worktree);
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    for content in [
        // Not JSON at all.
        "{ this is not json }".to_string(),
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
            matches!(read_worker(worktree), Recorded::Unreadable),
            "{content} was read as an answer"
        );
    }

    register_worker(worktree, "WID-957", None, None, None).unwrap();
    let Recorded::Found(worker) = read_worker(worktree) else {
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
        let Recorded::Found(read_back) = read_worker(worktree) else {
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
    let Recorded::Found(worker) = read_worker(worktree) else {
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
    let Recorded::Found(worker) = read_worker(worktree) else {
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
        "{\"pid\": 1, \"hub\": \"wid-957}",
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
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    unregister_hub(&root, "acme-widget").unwrap();
    claim_ours(&root, "acme-widget", "adjutant-acme-widget");
    unregister_hub(&root, "acme-widget").unwrap();
    unregister_hub(&root, "acme-widget").unwrap();
    assert!(!hub_record_path(&root, "acme-widget").exists());
}

/// Five launches at once used to produce three hubs, and a repository with three hubs
/// makes it luck which one a report reaches.
#[test]
fn only_one_of_five_launches_becomes_the_hub() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let name = this_process_name();
    let outcomes: Vec<Result<Claim, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..5)
            .map(|_| {
                scope.spawn(|| {
                    claim_hub(&root, "acme-widget", &name, "/src/widget", true, None, None)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    // Every launch has to reach an *answer*. Counting only the winners would pass a
    // run where one won and the other four failed outright.
    assert_eq!(tally(&outcomes), (1, 4, 0), "{outcomes:?}");
    assert!(hub_status(&root, "acme-widget", &name).present);
}

/// A record whose process is gone is not a hub, and must not block the next one.
#[test]
fn a_dead_hubs_record_does_not_hold_the_name() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    // A pid that really has exited, rather than this process wearing a wrong name —
    // "alive but not recognised" is a different thing entirely, and the test below is
    // the one that means it.
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    write_json(
        &hub_record_path(&root, "acme-widget"),
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
    claim_ours(&root, "acme-widget", &name);
    assert!(hub_status(&root, "acme-widget", &name).present);
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
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    write_json(
        &hub_record_path(&root, "acme-widget"),
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
    let status = hub_status(&root, "acme-widget", "adjutant-acme-widget");
    assert!(status.present, "{status:?}");
    assert!(!status.stale);
    // And a launch arriving now is told someone holds it, rather than taking it.
    assert!(matches!(
        claim_hub(
            &root,
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
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    write_json(
        &hub_record_path(&root, "acme-widget"),
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
                scope.spawn(|| {
                    claim_hub(&root, "acme-widget", &name, "/src/widget", true, None, None)
                })
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

/// The takeover lock is held by the operating system, so a second claimer is turned
/// away rather than left to decide for itself whether the first one is still alive.
#[test]
fn a_takeover_in_progress_turns_the_next_claim_away() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    let record = hub_record_path(&root, "acme-widget");
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
        claim_hub(&root, "acme-widget", &name, "/src/widget", true, None, None),
        Ok(Claim::Taken(_))
    ));

    // And once they are done, the next claim gets it.
    drop(lock);
    claim_ours(&root, "acme-widget", &name);
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
    assert!(matches!(read_worker(worktree), Recorded::Unreadable));
}

#[test]
fn a_worker_record_rewrite_keeps_unknown_keys_and_unreadable_phases() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    let original = json!({
        "pid": 4242,
        "title": "t",
        "psStarted": null,
        "hub": null,
        "task": 7,
        "x-unknown": {"a": [1]},
        "phases": [["plan", 1], {"newer": "shape"}, "x"],
        "terminal": {"backend": "tmux", "socket": null, "x-newer": 1},
    });
    write_json(&worker_record_path(worktree), &original).unwrap();
    set_worker_phase(worktree, "verify").unwrap();

    let after = read_json(&worker_record_path(worktree)).unwrap();
    assert_eq!(after["x-unknown"], original["x-unknown"]);
    assert_eq!(after["psStarted"], Value::Null);
    assert_eq!(after["hub"], Value::Null);
    assert_eq!(after["task"], 7);
    assert_eq!(after["terminal"], original["terminal"]);
    for i in 0..3 {
        assert_eq!(after["phases"][i], original["phases"][i]);
    }
    assert_eq!(after["phases"][3][0], "verify");
    let phases = worker_status(worktree).phases;
    assert_eq!(phases.len(), 2);
    assert_eq!(phases[0], ("plan".to_string(), 1));
    assert_eq!(phases[1].0, "verify");
}

#[test]
fn a_saved_session_rewrite_keeps_unknown_keys() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    write_json(
        &worker_record_path(worktree),
        &json!({"pid": 4242, "title": "t", "hub": "ALPHA-1", "task": "task-1"}),
    )
    .unwrap();
    write_json(
        &worker_session_path(worktree),
        &json!({
            "sessionId": "sid-1",
            "title": "t",
            "hub": "ALPHA-1",
            "task": 5,
            "savedAt": "2026-01-01T00:00:00Z",
            "x-unknown": {"a": [1]},
        }),
    )
    .unwrap();
    assert_eq!(
        worker_session(worktree).unwrap().other["x-unknown"],
        json!({"a": [1]})
    );

    relink_worker(worktree, None, "task-2", None).unwrap();
    let after = read_json(&worker_session_path(worktree)).unwrap();
    assert_eq!(after["x-unknown"], json!({"a": [1]}));
    assert_eq!(after["sessionId"], "sid-1");
    assert_eq!(after["title"], "t");
    assert_eq!(after["task"], "task-2");
    assert!(after.get("hub").is_none());

    let saved = worker_session(worktree).unwrap();
    rewrite_worker_session(worktree, &saved, "t", Some("B"), Some("task-2")).unwrap();
    let after = read_json(&worker_session_path(worktree)).unwrap();
    assert_eq!(after["x-unknown"], json!({"a": [1]}));
    assert_eq!(after["hub"], "B");
}

#[test]
fn a_worker_record_key_of_the_wrong_type_reads_as_absent_and_the_rest_still_reads() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    write_json(
        &worker_record_path(worktree),
        &json!({"pid": "4242", "title": "t", "hub": 5}),
    )
    .unwrap();
    let Recorded::Found(record) = read_worker_record(worktree) else {
        panic!("a record with one bad key is still a record");
    };
    assert_eq!(record.pid, None);
    assert_eq!(record.other["pid"], "4242");
    assert_eq!(record.title.as_deref(), Some("t"));
    assert!(record.hub_is_not_a_name());
    assert!(matches!(read_worker(worktree), Recorded::Unreadable));

    write_json(&worker_record_path(worktree), &json!({"hub": null})).unwrap();
    let Recorded::Found(record) = read_worker_record(worktree) else {
        panic!("a record with a null hub is still a record");
    };
    assert!(!record.hub_is_not_a_name());
}

#[test]
fn relinking_to_the_repository_hub_drops_a_hub_of_any_type() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    write_json(&worker_record_path(worktree), &json!({"pid": 1, "hub": 5})).unwrap();
    relink_worker(worktree, None, "t", None).unwrap();
    let after = read_json(&worker_record_path(worktree)).unwrap();
    assert!(after.get("hub").is_none(), "{after}");
    assert_eq!(after["phase"], "implement");
}

#[test]
fn a_worker_record_that_is_not_an_object_is_unreadable_not_absent() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    assert!(matches!(read_worker_record(worktree), Recorded::Absent));
    let path = worker_record_path(worktree);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for text in ["[1,2]", "{ not json }"] {
        std::fs::write(&path, text).unwrap();
        assert!(
            matches!(read_worker_record(worktree), Recorded::Unreadable),
            "{text}"
        );
    }
}

#[test]
fn a_record_that_is_not_json_is_not_reported_as_no_worker() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    let path = worker_record_path(worktree);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ not json }").unwrap();
    for error in [
        set_worker_phase(worktree, "verify").unwrap_err(),
        relink_worker(worktree, None, "t", None).unwrap_err(),
    ] {
        assert!(error.contains("cannot read the worker record"), "{error}");
        assert!(!error.contains("no worker is registered"), "{error}");
    }
}

#[test]
fn register_worker_writes_the_keys_it_always_wrote() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    register_worker(worktree, "t", None, None, None).unwrap();
    let written = read_json(&worker_record_path(worktree)).unwrap();
    // `psStarted` is there only when `ps` could say when this process started.
    let keys: Vec<&str> = written
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let expected: &[&str] = match ps_started(std::process::id()) {
        Some(_) => &["pid", "psStarted", "startedAt", "title"],
        None => &["pid", "startedAt", "title"],
    };
    assert_eq!(keys, expected);
    let Recorded::Found(record) = read_worker_record(worktree) else {
        panic!("just registered");
    };
    assert_eq!(record.to_value(), written);
}

#[test]
fn a_restarted_worker_carries_phases_it_cannot_read() {
    let _sandbox = Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    let old = json!({"pid": 1, "task": "t", "phases": [["plan", 1], {"x": 1}]});
    write_json(&worker_record_path(worktree), &old).unwrap();
    register_worker(worktree, "w", None, Some("t"), None).unwrap();
    let after = read_json(&worker_record_path(worktree)).unwrap();
    assert_eq!(after["phases"], old["phases"]);
    assert!(after.get("phase").is_none());

    write_json(&worker_record_path(worktree), &old).unwrap();
    register_worker(worktree, "w", None, Some("other"), None).unwrap();
    let after = read_json(&worker_record_path(worktree)).unwrap();
    assert!(after.get("phases").is_none());
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
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let slug = "acme-widget";
    assert_eq!(hub_liveness(&root, slug), Liveness::Gone);
    let record = hub_record_path(&root, slug);
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(record.with_file_name("gone.json"), &record).unwrap();
    assert_eq!(hub_liveness(&root, slug), Liveness::CannotTell);
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
fn unregister_hub_if_unnamed_removes_only_a_record_naming_no_process() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let path = hub_record_path(&root, "acme-widget");
    assert!(unregister_hub_if_unnamed(&root, "acme-widget").unwrap());

    write_json(
        &path,
        &json!({"pid": 4242, "psStarted": "Mon Jan  1 00:00:00 2024"}),
    )
    .unwrap();
    assert!(!unregister_hub_if_unnamed(&root, "acme-widget").unwrap());
    assert!(path.exists());

    write_json(&path, &json!({"hubName": "adjutant-acme-widget"})).unwrap();
    assert!(unregister_hub_if_unnamed(&root, "acme-widget").unwrap());
    assert!(!path.exists());
}

#[test]
fn unregister_hub_if_leaves_a_record_naming_another_process() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let path = hub_record_path(&root, "acme-widget");
    let record = json!({"pid": 4242, "psStarted": "Mon Jan  1 00:00:00 2024"});
    write_json(&path, &record).unwrap();

    // Another start time is another process on a recycled pid, and another pid is
    // another hub: neither is this call's to clear.
    assert_eq!(
        unregister_hub_if(&root, "acme-widget", 4242, Some("later")),
        Ok(false)
    );
    assert_eq!(
        unregister_hub_if(&root, "acme-widget", 4243, Some("Mon Jan  1 00:00:00 2024")),
        Ok(false)
    );
    assert!(path.exists());

    assert_eq!(
        unregister_hub_if(&root, "acme-widget", 4242, Some("Mon Jan  1 00:00:00 2024")),
        Ok(true)
    );
    assert!(!path.exists());
    // Already gone is the same end state.
    assert_eq!(
        unregister_hub_if(&root, "acme-widget", 4242, None),
        Ok(true)
    );
}

#[test]
fn a_claimed_hub_record_carries_where_it_runs() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let terminal = crate::infra::terminal::SessionTerminal {
        backend: "tmux".into(),
        socket: Some("scratch".into()),
        session: Some("adj".into()),
        window: Some("@1".into()),
        pane: Some("%3".into()),
    };
    match claim_hub(
        &root,
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
    let record = read_json(&hub_record_path(&root, "acme-widget")).unwrap();
    assert_eq!(record["terminal"]["socket"], "scratch");
    assert_eq!(record["terminal"]["pane"], "%3");
    assert_eq!(record["terminal"]["backend"], "tmux");
    let keys: Vec<&str> = record
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "cwd",
            "hubName",
            "nameInCommand",
            "pid",
            "psStarted",
            "startedAt",
            "terminal"
        ]
    );
    let Recorded::Found(read) = read_hub_record(&root, "acme-widget") else {
        panic!("the record just written is readable");
    };
    assert_eq!(read.to_value(), record);

    // A launch that does not know where it is leaves the field out, as records always were.
    unregister_hub(&root, "acme-widget").unwrap();
    claim_ours(&root, "acme-widget", "adjutant-acme-widget");
    assert!(
        read_json(&hub_record_path(&root, "acme-widget"))
            .unwrap()
            .get("terminal")
            .is_none()
    );
}

/// Writes `text` as the hub record of `slug`, creating the directory a claim would have.
fn write_hub_record(root: &Path, slug: &str, text: &str) -> PathBuf {
    let path = hub_record_path(root, slug);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn a_hub_record_key_of_the_wrong_type_reads_as_absent_and_the_rest_still_reads() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let path = write_hub_record(
        &root,
        "acme-widget",
        r#"{"pid":"4242","hubName":"adjutant-x","cwd":7,"startedAt":"s","psStarted":"   ","nameInCommand":"yes","terminal":{"socket":"s"}}"#,
    );
    let Recorded::Found(record) = read_hub_record(&root, "acme-widget") else {
        panic!("a record with a wrong-typed key is still a record");
    };
    assert_eq!(record.pid, None);
    assert_eq!(record.cwd, None);
    assert_eq!(record.hub_name.as_deref(), Some("adjutant-x"));
    assert_eq!(record.name_in_command, None);
    assert_eq!(record.terminal, None);
    assert_eq!(recorded_anchor(&record), None);
    assert_eq!(record.other["pid"], "4242");

    let status = hub_status(&root, "acme-widget", "adjutant-acme-widget");
    assert!(status.stale);
    assert_eq!(status.pid, None);
    assert_eq!(status.cwd, None);
    assert_eq!(status.started_at.as_deref(), Some("s"));

    // A pid of the wrong type is still somebody's claim to the name.
    assert_eq!(unregister_hub_if_unnamed(&root, "acme-widget"), Ok(false));
    assert!(path.exists());
}

#[test]
fn an_unknown_key_in_a_hub_record_is_kept_and_the_file_left_alone() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let path = write_hub_record(
        &root,
        "acme-widget",
        r#"{"pid":4242,"hubName":"adjutant-x","cwd":"/src/widget","startedAt":"s","psStarted":"p","nameInCommand":true,"x-unknown":{"a":[1]}}"#,
    );
    let before = std::fs::read(&path).unwrap();
    let Recorded::Found(record) = read_hub_record(&root, "acme-widget") else {
        panic!("readable");
    };
    assert_eq!(record.other["x-unknown"], json!({"a": [1]}));
    hub_status(&root, "acme-widget", "adjutant-acme-widget");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(record.to_value(), read_json(&path).unwrap());
}

#[test]
fn a_hub_record_that_is_not_an_object_is_unreadable_not_absent() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    assert_eq!(read_hub_record(&root, "acme-widget"), Recorded::Absent);

    write_hub_record(&root, "acme-widget", "[1,2]");
    assert_eq!(read_hub_record(&root, "acme-widget"), Recorded::Unreadable);
    assert_eq!(
        holder(&hub_record_path(&root, "acme-widget")),
        Liveness::CannotTell
    );
    assert!(!hub_status(&root, "acme-widget", "adjutant-acme-widget").stale);

    write_hub_record(&root, "acme-widget", "{ not json }");
    assert_eq!(read_hub_record(&root, "acme-widget"), Recorded::Unreadable);

    write_hub_record(&root, "other", r#"{"cwd":"/src/other"}"#);
    let found = hub_records(&root);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, "other");
    assert_eq!(found[0].1.cwd.as_deref(), Some("/src/other"));
}

#[test]
fn claim_hub_writes_the_record_it_always_wrote() {
    let terminal = crate::infra::terminal::SessionTerminal {
        backend: "tmux".into(),
        socket: Some("scratch".into()),
        session: None,
        window: None,
        pane: Some("%3".into()),
    };
    let full = HubRecord {
        pid: Some(4242),
        hub_name: Some("adjutant-x".into()),
        cwd: Some("/src/widget".into()),
        started_at: Some("2026-01-01T00:00:00Z".into()),
        ps_started: None,
        hub: Some("task-1".into()),
        name_in_command: Some(true),
        terminal: Some(terminal.clone()),
        other: serde_json::Map::new(),
    };
    let literal = json!({
        "pid": 4242u32,
        "hubName": "adjutant-x",
        "cwd": "/src/widget",
        "startedAt": "2026-01-01T00:00:00Z",
        "psStarted": null,
        "nameInCommand": true,
        "hub": "task-1",
        "terminal": serde_json::to_value(&terminal).unwrap(),
    });
    assert_eq!(
        serde_json::to_string_pretty(&full.to_value()).unwrap(),
        serde_json::to_string_pretty(&literal).unwrap()
    );

    let bare = HubRecord {
        hub: None,
        terminal: None,
        ..full
    };
    let literal = json!({
        "pid": 4242u32,
        "hubName": "adjutant-x",
        "cwd": "/src/widget",
        "startedAt": "2026-01-01T00:00:00Z",
        "psStarted": null,
        "nameInCommand": true,
    });
    assert_eq!(
        serde_json::to_string_pretty(&bare.to_value()).unwrap(),
        serde_json::to_string_pretty(&literal).unwrap()
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

/// A relative state directory is the main checkout's, wherever the command was typed: the
/// directory `adj hub` reads.
#[test]
fn a_context_takes_a_relative_state_dir_under_its_main_checkout() {
    let sandbox = Sandbox::empty();
    let checkout = tempfile::tempdir().unwrap();
    // An absolute one is itself, whatever the anchor.
    assert_eq!(state_root(Some(checkout.path())), sandbox.state());
    let _state =
        crate::testing::EnvVar::set(&sandbox, crate::infra::env::STATE_DIR_ENV, "state-here");
    let repo = crate::kernel::identity::RepoInfo {
        main: checkout.path().to_string_lossy().to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let ctx = context_of(repo).unwrap();
    assert_eq!(ctx.state, checkout.path().join("state-here"));
    assert_eq!(
        state_root(None),
        std::env::current_dir().unwrap().join("state-here")
    );
}

/// A caller that already holds the root it reads gets a context over exactly that.
#[test]
fn a_context_at_a_root_reads_that_root() {
    let sandbox = Sandbox::empty();
    let checkout = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let _state =
        crate::testing::EnvVar::set(&sandbox, crate::infra::env::STATE_DIR_ENV, "state-here");
    let repo = crate::kernel::identity::RepoInfo {
        main: checkout.path().to_string_lossy().to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let other_dir = other.path().join("elsewhere");
    let ctx = context_at(repo, other_dir.clone()).unwrap();
    assert_eq!(ctx.state, other_dir);
}

#[test]
fn a_record_is_removed_only_while_it_names_the_stopped_server() {
    let old = (7, Some("Mon Jan  1 00:00:00 2024".to_string()));
    assert!(names_resident(
        Some(&old),
        7,
        Some("Mon Jan  1 00:00:00 2024")
    ));
    // A supervisor's restart has written another pid, or the same pid started later.
    assert!(!names_resident(
        Some(&old),
        8,
        Some("Mon Jan  1 00:00:00 2024")
    ));
    assert!(!names_resident(Some(&old), 7, Some("later")));
    assert!(!names_resident(None, 7, None));
}

#[test]
fn forgetting_a_board_removes_only_that_slug() {
    let sandbox = crate::testing::Sandbox::empty();
    let root = sandbox.state();
    std::fs::create_dir_all(boards_dir(&root)).unwrap();
    for slug in ["acme-widget-a", "acme-widget-b"] {
        std::fs::write(boards_dir(&root).join(format!("{slug}.json")), "{}").unwrap();
    }
    forget_board(&root, "acme-widget-a").unwrap();
    assert!(!boards_dir(&root).join("acme-widget-a.json").exists());
    assert!(boards_dir(&root).join("acme-widget-b.json").exists());
    // Nothing to forget is not an error.
    forget_board(&root, "acme-widget-a").unwrap();
}

#[test]
fn a_board_address_keeps_keys_it_does_not_write() {
    let sandbox = crate::testing::Sandbox::empty();
    let root = sandbox.state();
    let checkout = tempfile::tempdir().unwrap();
    let repo = crate::kernel::identity::RepoInfo {
        main: checkout.path().to_string_lossy().to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let path = boards_dir(&root).join("acme-widget.json");
    std::fs::create_dir_all(boards_dir(&root)).unwrap();

    let same = json!({"main": repo.main, "nwo": "acme/widget", "hub": null, "x-unknown": 1});
    std::fs::write(&path, same.to_string()).unwrap();
    let before = std::fs::read(&path).unwrap();
    note_board(&root, &repo);
    assert_eq!(std::fs::read(&path).unwrap(), before);

    let moved = json!({"main": "/elsewhere", "nwo": "acme/widget", "hub": null, "x-unknown": 1});
    std::fs::write(&path, moved.to_string()).unwrap();
    note_board(&root, &repo);
    let after = read_json(&path).unwrap();
    assert_eq!(after["main"], repo.main);
    assert_eq!(after["x-unknown"], 1);
}

#[test]
fn the_dashboards_record_is_written_whole() {
    let sandbox = crate::testing::Sandbox::empty();
    assert_eq!(record(&sandbox.state(), "acme-widget", 4321), Ok(true));
    let dir = sandbox.state().join("dashboards");
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["acme-widget.json"]);
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("acme-widget.json")).unwrap())
            .unwrap();
    let pid = std::process::id();
    assert_eq!(written["pid"], pid);
    assert_eq!(written["port"], 4321);
    assert_eq!(
        written["psStarted"],
        json!(crate::registry::ps_started(pid))
    );
    assert_eq!(
        dashboards_running(&sandbox.state(), "acme-widget"),
        Some(4321)
    );
}

#[test]
fn the_resident_is_preferred_over_a_dedicated_board() {
    assert_eq!(prefer(Some(1), Some(2)), Some(Served::Resident(1)));
    assert_eq!(prefer(Some(1), None), Some(Served::Resident(1)));
    assert_eq!(prefer(None, Some(2)), Some(Served::Dedicated(2)));
    assert_eq!(prefer(None, None), None);
}

#[test]
fn a_worker_status_and_identity_expose_where_it_was_started() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path();
    let at = crate::infra::terminal::SessionTerminal {
        backend: "tmux".into(),
        socket: Some("/tmp/rec.sock".into()),
        session: Some("adjutant".into()),
        window: None,
        pane: Some("%7".into()),
    };
    register_worker(worktree, "WID-957", None, None, Some(&at)).unwrap();
    assert_eq!(worker_status(worktree).terminal, Some(at.clone()));
    let Recorded::Found(worker) = read_worker(worktree) else {
        panic!("a registered worker is readable");
    };
    assert_eq!(worker.terminal, Some(at));

    // A record from before the location was kept has none, and one that does not read as a
    // location is none as well.
    write_json(
        &worker_record_path(worktree),
        &json!({"pid": std::process::id(), "terminal": "tmux"}),
    )
    .unwrap();
    assert_eq!(worker_status(worktree).terminal, None);
    let Recorded::Found(worker) = read_worker(worktree) else {
        panic!("a record with a pid is readable");
    };
    assert_eq!(worker.terminal, None);
}

#[test]
fn a_hub_status_exposes_where_it_was_started() {
    let sandbox = Sandbox::empty();
    let root = sandbox.state();
    let name = this_process_name();
    let terminal = crate::infra::terminal::SessionTerminal {
        backend: "iterm2".into(),
        socket: None,
        session: None,
        window: None,
        pane: None,
    };
    match claim_hub(
        &root,
        "acme-widget",
        &name,
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
    assert_eq!(
        hub_status(&root, "acme-widget", &name).terminal,
        Some(terminal)
    );
}
