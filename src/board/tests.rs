use std::collections::HashSet;
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::gate;
use crate::task;

use super::view::{
    WorkerSeen, board_counts, board_session, branch_of, cut_chars, history_id, history_of,
    socket_key_in, state, with_records, worker_session_ids,
};
use super::*;

/// A relative state directory is the main checkout's of the repository the server command was
/// typed in, and outside any repository the working directory's.
#[test]
fn the_resident_root_is_taken_against_the_checkout_it_was_typed_in() {
    let sandbox = crate::testing::Sandbox::empty();
    let _state =
        crate::testing::EnvVar::set(&sandbox, crate::infra::env::STATE_DIR_ENV, "relative-state");
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
    assert_eq!(
        resident_root(Some(&repo)),
        checkout.path().join("relative-state")
    );
    assert_eq!(
        resident_root(None),
        std::env::current_dir().unwrap().join("relative-state")
    );
}

#[test]
fn a_board_url_carries_its_path() {
    assert_eq!(
        resident_board_url(4577, "acme-x-1", "tok"),
        "http://127.0.0.1:4577/b/acme-x-1/?token=tok"
    );
    assert_eq!(board_url(4577, "tok"), "http://127.0.0.1:4577/?token=tok");
}

#[test]
fn the_server_log_is_readable_by_its_owner_alone() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.log");
    drop(private_log(&path).unwrap());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // A log an older version made with the default mask is tightened, not trusted.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    drop(private_log(&path).unwrap());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn a_taken_port_falls_back_to_a_free_one() {
    let taken = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = taken.local_addr().unwrap().port();
    let second = bind_preferring(port).unwrap();
    let got = second.local_addr().unwrap().port();
    assert_ne!(got, port);
    assert_ne!(got, 0);
}

#[test]
fn a_pane_is_read_again_only_when_it_has_moved_and_the_last_read_is_old() {
    let lines = LastLines::default();
    let t0 = Instant::now();
    let reads = std::cell::Cell::new(0);
    // The read is made at epoch second 1000 unless a case says otherwise.
    let look_at = |activity, at: Instant, secs| {
        lines.look("pane", Some(activity), at, secs, || {
            reads.set(reads.get() + 1);
            Some(format!("read {}", reads.get()))
        })
    };
    let look = |activity, at| look_at(activity, at, 1000);
    assert_eq!(look(10, t0).as_deref(), Some("read 1"));
    // Nothing moved: the answer stands, however old.
    assert_eq!(
        look(10, t0 + Duration::from_secs(60)).as_deref(),
        Some("read 1")
    );
    // Moved, but read a moment ago.
    assert_eq!(
        look(11, t0 + Duration::from_secs(2)).as_deref(),
        Some("read 1")
    );
    // Moved and read long enough ago.
    assert_eq!(look(11, t0 + LAST_LINE_MIN_AGE).as_deref(), Some("read 2"));
    assert_eq!(reads.get(), 2);
    // Activity has one-second resolution: output in the second of the read may have come
    // after it, so that read is not trusted once the age is up.
    assert_eq!(
        look_at(2000, t0 + Duration::from_secs(20), 2000).as_deref(),
        Some("read 3")
    );
    assert_eq!(
        look_at(2000, t0 + Duration::from_secs(21), 2000).as_deref(),
        Some("read 3")
    );
    assert_eq!(
        look_at(2000, t0 + Duration::from_secs(30), 2001).as_deref(),
        Some("read 4")
    );
    assert_eq!(
        look_at(2000, t0 + Duration::from_secs(40), 2005).as_deref(),
        Some("read 4")
    );
    // A read that found nothing, or a window with no known activity, is tried again.
    let nothing = |activity, at: Instant| lines.look("empty", activity, at, 1000, || None);
    assert_eq!(nothing(Some(5), t0), None);
    let found = |activity, at: Instant| {
        lines.look("empty", activity, at, 1000, || Some("late".to_string()))
    };
    assert_eq!(found(Some(5), t0 + Duration::from_secs(1)), None);
    assert_eq!(
        found(Some(5), t0 + LAST_LINE_MIN_AGE).as_deref(),
        Some("late")
    );
    assert_eq!(
        found(None, t0 + LAST_LINE_MIN_AGE * 2).as_deref(),
        Some("late")
    );
    // A pane that is no longer listed is forgotten.
    lines.keep_only(&HashSet::new());
    assert_eq!(
        look(11, t0 + Duration::from_secs(70)).as_deref(),
        Some("read 5")
    );
}

#[test]
fn binding_the_resident_records_this_process_and_refuses_a_second_while_the_lock_is_held() {
    let sandbox = crate::testing::Sandbox::empty();
    let root = sandbox.state();
    let first = bind_resident(&root, 0).unwrap();
    assert_ne!(first.bound, 0);
    let record = crate::infra::fs::read_json(&root.join("server.json")).unwrap();
    assert_eq!(record["pid"], json!(std::process::id()));
    assert_eq!(record["port"], json!(first.bound));
    for key in ["psStarted", "startedAt", "version"] {
        assert!(record.get(key).is_some(), "{key} in {record}");
    }
    assert_eq!(
        crate::registry::live_resident(&root),
        Some((std::process::id(), first.bound))
    );
    let Err(refused) = bind_resident(&root, 0) else {
        panic!("a second resident bound while the first held the lock")
    };
    assert!(
        refused.starts_with("another adj server is running"),
        "{refused}"
    );
}

/// The resident is detached and stands elsewhere, so it is handed the root it was given, not
/// whatever its own working directory would make of a relative one.
#[test]
fn the_resident_is_started_with_the_root_it_was_given() {
    use std::process::Stdio;
    let sandbox = crate::testing::Sandbox::empty();
    let _state =
        crate::testing::EnvVar::set(&sandbox, crate::infra::env::STATE_DIR_ENV, "relative-state");
    let root = crate::registry::state_root(Some(std::path::Path::new("/src/widget")));
    let command = resident_command(
        std::path::Path::new("adj"),
        0,
        &root,
        Stdio::null(),
        Stdio::null(),
    );
    let value = command
        .get_envs()
        .find(|(name, _)| *name == crate::infra::env::STATE_DIR_ENV)
        .and_then(|(_, value)| value);
    assert_eq!(
        value,
        Some(std::path::Path::new("/src/widget/relative-state").as_os_str())
    );
}

#[test]
fn a_long_focus_is_cut_on_a_character_boundary() {
    assert_eq!(cut_chars("短い", 400), "短い");
    assert_eq!(cut_chars("あいうえお", 3), "あいう…");
}

#[test]
fn worker_ids_are_unique_within_a_board() {
    let paths: Vec<String> = ["/w/a/app", "/w/b/app", "/w/c/main", "/w/d/solo"]
        .iter()
        .map(|p| p.to_string())
        .collect();
    let ids = worker_session_ids(&paths, true);
    // A name nobody else has keeps the id it always had.
    assert_eq!(ids[3], "worker-solo");
    // The rest gain a digest of their path, so two of one name are two ids, and
    // `worker-main` stays the main checkout's.
    assert!(ids[0].starts_with("worker-app-") && ids[1].starts_with("worker-app-"));
    assert!(ids[2].starts_with("worker-main-"));
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
    assert_eq!(ids, worker_session_ids(&paths, true));
    assert_eq!(ids[0].len(), "worker-app-".len() + 8);
}

#[test]
fn a_worktree_called_main_keeps_its_id_unless_the_main_checkout_has_it() {
    let paths = vec!["/w/c/main".to_string(), "/w/d/solo".to_string()];
    assert_eq!(
        worker_session_ids(&paths, false),
        ["worker-main", "worker-solo"]
    );
    assert!(worker_session_ids(&paths, true)[0].starts_with("worker-main-"));
}

#[test]
fn a_worktree_s_branch_is_its_own_whatever_git_dir_names() {
    let sandbox = crate::testing::Sandbox::empty();
    let here = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    crate::testing::init_repo(here.path(), "mine");
    crate::testing::init_repo(other.path(), "theirs");

    let _var = crate::testing::EnvVar::set(&sandbox, "GIT_DIR", other.path().join(".git"));
    assert_eq!(
        branch_of(&here.path().to_string_lossy()).as_deref(),
        Some("mine")
    );
}

fn a_task(id: &str, status: task::Status) -> task::Task {
    task::Task {
        id: id.to_string(),
        kind: task::Kind::Start,
        title: id.to_string(),
        body: String::new(),
        issue_url: None,
        done_when: task::DoneWhen::Pr,
        stop_at: task::StopAt::Plan,
        executor: task::Executor::Worker,
        base: None,
        parent: None,
        worktree_name: None,
        auto_start: true,
        order: 0,
        status,
        worktree: None,
        issue: None,
        pr: None,
        jules_session: None,
        jules_by: None,
        relayed: Vec::new(),
        announced: Vec::new(),
        relay_rounds: 0,
        note: None,
        instruction: None,
        gate_answered_at: None,
        issue_snapshot: None,
        title_pending: false,
        pr_status: None,
        created_at: "20260922T000000Z".to_string(),
        updated_at: "20260922T000000Z".to_string(),
        extra: serde_json::Map::new(),
    }
}

fn a_gate(id: &str, kind: gate::Kind, task: &str) -> gate::Gate {
    serde_json::from_value(json!({
        "id": id,
        "kind": kind,
        "worktree": "/tmp/wt",
        "task": task,
        "title": id,
        "openedAt": "20260922T010000Z",
    }))
    .unwrap()
}

fn counts(tasks: &[task::Task], gates: &[gate::Gate], phase: Option<&str>) -> (usize, usize) {
    board_counts(tasks, gates, |_| {
        phase.map(|p| WorkerSeen {
            task: None,
            phase: Some(p.to_string()),
        })
    })
}

fn on_pr(id: &str, status: task::Status) -> task::Task {
    let mut t = a_task(id, status);
    t.pr = Some("https://example.com/pull/1".to_string());
    t.worktree = Some("/tmp/wt".to_string());
    t
}

#[test]
fn a_gate_on_a_task_makes_it_wait_and_not_work() {
    let t = a_task("t1", task::Status::Dispatched);
    assert_eq!(
        counts(&[t], &[a_gate("g", gate::Kind::Plan, "t1")], None),
        (1, 0)
    );
}

#[test]
fn a_gate_with_no_task_on_the_board_waits_by_itself() {
    let t = a_task("t1", task::Status::Queued);
    let mut loose = a_gate("g1", gate::Kind::Question, "t1");
    loose.task = None;
    let unknown = a_gate("g2", gate::Kind::Question, "gone");
    assert_eq!(counts(&[t], &[loose, unknown], None), (2, 0));
}

#[test]
fn a_pull_request_waits_on_a_person_only_in_the_phase_that_hands_it_over() {
    let t = on_pr("t1", task::Status::Pr);
    assert_eq!(counts(std::slice::from_ref(&t), &[], Some("pr")), (1, 0));
    assert_eq!(
        counts(std::slice::from_ref(&t), &[], Some("pr-bots")),
        (0, 1)
    );
    // No worker record: the task's own status says whose ball it is.
    assert_eq!(counts(&[t], &[], None), (1, 0));
    let dispatched = on_pr("t2", task::Status::Dispatched);
    assert_eq!(counts(&[dispatched], &[], None), (0, 1));
}

/// A card with a PR the last refresh read: `review` and the counts of checks that are
/// failing and pending, under whichever phase the worker is in.
fn read_pr(state: &str, review: &str, fail: u32, pending: u32) -> task::Task {
    let mut t = on_pr("t1", task::Status::Pr);
    t.pr_status = Some(task::PrStatus {
        state: state.to_string(),
        title: String::new(),
        review: review.to_string(),
        ci: task::CheckCounts {
            pass: 1,
            fail,
            pending,
        },
    });
    t
}

#[test]
fn a_pull_request_waits_when_it_is_the_persons_turn_and_works_when_it_is_not() {
    // Changes asked for, approved, a failed check, closed without merging: the person's.
    for t in [
        read_pr("open", "changes", 0, 0),
        read_pr("open", "approved", 0, 0),
        read_pr("open", "none", 1, 0),
        read_pr("closed", "none", 0, 0),
    ] {
        let status = t.pr_status.clone();
        assert_eq!(counts(&[t], &[], Some("pr")), (1, 0), "{status:?}");
    }
    // Another reviewer's, still running checks, or merged: not the person's.
    for t in [
        read_pr("open", "required", 0, 0),
        read_pr("open", "none", 0, 2),
        read_pr("merged", "approved", 0, 0),
    ] {
        let status = t.pr_status.clone();
        assert_eq!(counts(&[t], &[], Some("pr")), (0, 1), "{status:?}");
    }
}

#[test]
fn the_persons_turn_outranks_the_bots_phase_but_not_a_worker_still_at_work() {
    let changes = read_pr("open", "changes", 0, 0);
    assert_eq!(
        counts(std::slice::from_ref(&changes), &[], Some("pr-bots")),
        (1, 0)
    );
    assert_eq!(counts(&[changes], &[], Some("review")), (0, 1));
}

#[test]
fn a_record_naming_another_task_is_no_record() {
    let t = on_pr("t1", task::Status::Pr);
    let (waiting, working) = board_counts(&[t], &[], |_| {
        Some(WorkerSeen {
            task: Some("other".to_string()),
            phase: Some("pr-bots".to_string()),
        })
    });
    assert_eq!((waiting, working), (1, 0));
}

#[test]
fn dispatched_tasks_work_and_finished_ones_do_neither() {
    let tasks = [
        a_task("t1", task::Status::Dispatched),
        a_task("t2", task::Status::Done),
        a_task("t3", task::Status::Cancelled),
        a_task("t4", task::Status::Backlog),
        on_pr("t5", task::Status::Done),
    ];
    assert_eq!(counts(&tasks, &[], None), (0, 1));
}

#[test]
fn a_jules_task_with_a_pull_request_waits() {
    let mut t = on_pr("t1", task::Status::Pr);
    t.jules_session = Some("s1".to_string());
    assert_eq!(counts(&[t], &[], Some("pr-bots")), (1, 0));
}

#[test]
fn a_jules_task_follows_its_prs_turn_and_waits_when_the_turn_says_nothing() {
    let jules = |mut t: task::Task| {
        t.jules_session = Some("s1".to_string());
        t
    };
    for t in [
        read_pr("open", "changes", 0, 0),
        read_pr("open", "approved", 0, 0),
        read_pr("closed", "none", 0, 0),
        read_pr("open", "none", 0, 0),
    ] {
        assert_eq!(counts(&[jules(t)], &[], None), (1, 0));
    }
    for t in [
        read_pr("open", "required", 0, 0),
        read_pr("open", "none", 0, 2),
        read_pr("merged", "approved", 0, 0),
    ] {
        assert_eq!(counts(&[jules(t)], &[], None), (0, 1));
    }
}

#[test]
fn a_live_task_carries_its_records_and_the_plan_approved_last() {
    let mut record = a_gate("r1", gate::Kind::Diff, "t1");
    record.wait = false;
    let others = a_gate("r2", gate::Kind::Verify, "t2");
    let answered = |id: &str, decision: &str, at: &str| {
        let mut g = a_gate(id, gate::Kind::Plan, "t1");
        g.decision = Some(decision.to_string());
        g.answered_at = Some(at.to_string());
        g
    };
    let tasks = with_records(
        vec![a_task("t1", task::Status::Dispatched)],
        vec![record, others],
        vec![
            answered("p-old", "approve", "20260922T020000Z"),
            answered("p-new", "approve", "20260922T040000Z"),
            // Sent back later still: not what was approved.
            answered("p-sent-back", "changes", "20260922T050000Z"),
        ],
    );
    let ids: Vec<&str> = tasks[0]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["r1"]);
    assert!(tasks[0]["records"][0].get("diff").is_none());
    assert!(tasks[0]["records"][0].get("diffSize").is_none());
    assert_eq!(tasks[0]["approvedPlan"]["id"], "p-new");
    assert_eq!(tasks[0]["approvedPlan"]["answeredAt"], "20260922T040000Z");
}

#[test]
fn a_record_carries_the_size_of_its_diff_and_not_the_diff() {
    let mut record = a_gate("r1", gate::Kind::Diff, "t1");
    record.wait = false;
    record.diff = Some("diff --git a/ü b/ü\n+é\n".to_string());
    let size = record.diff.as_ref().unwrap().len();
    let tasks = with_records(
        vec![a_task("t1", task::Status::Dispatched)],
        vec![record.clone()],
        Vec::new(),
    );
    let carried = &tasks[0]["records"][0];
    assert!(carried.get("diff").is_none(), "{carried}");
    assert_eq!(carried["diffSize"], size);
    // The history is where the diff is read from, whole.
    let history = history_of("t1", Vec::new(), vec![record]);
    assert_eq!(
        history["records"][0]["diff"].as_str().map(str::len),
        Some(size)
    );
}

#[test]
fn a_task_with_no_approved_plan_says_so_and_a_finished_one_carries_nothing() {
    let tasks = with_records(
        vec![
            a_task("t1", task::Status::Queued),
            a_task("t2", task::Status::Done),
        ],
        vec![a_gate("r2", gate::Kind::Diff, "t2")],
        Vec::new(),
    );
    assert_eq!(tasks[0]["records"], json!([]));
    assert!(tasks[0]["approvedPlan"].is_null());
    assert!(tasks[1].get("records").is_none(), "{}", tasks[1]);
}

#[test]
fn a_task_s_history_is_its_own_answered_gates_and_records_of_every_kind() {
    let mut diff = a_gate("20260922T010000Z-diff", gate::Kind::Diff, "t1");
    diff.decision = Some("changes".to_string());
    let mut plan = a_gate("20260922T000000Z-plan", gate::Kind::Plan, "t1");
    plan.opened_at = "20260922T000000Z".to_string();
    let theirs = a_gate("20260922T020000Z-verify", gate::Kind::Verify, "t2");
    let mut record = a_gate("20260922T030000Z-verify-record", gate::Kind::Verify, "t1");
    record.wait = false;

    let history = history_of("t1", vec![plan, diff, theirs.clone()], vec![record, theirs]);
    let ids = |key: &str| -> Vec<String> {
        history[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        ids("answered"),
        ["20260922T000000Z-plan", "20260922T010000Z-diff"]
    );
    assert_eq!(ids("records"), ["20260922T030000Z-verify-record"]);
}

#[test]
fn a_history_path_names_one_task() {
    assert_eq!(history_id("/api/tasks/t1/history"), Some("t1"));
    for bad in [
        "/api/tasks//history",
        "/api/tasks/a/b/history",
        "/api/tasks/t1",
    ] {
        assert_eq!(history_id(bad), None, "{bad}");
    }
}

/// One session asked for on its own is the entry the whole list holds for it, for every
/// shape of id: the two can only differ if a field is gathered in one path and not the other.
#[test]
fn one_session_is_the_entry_the_whole_list_holds() {
    let sandbox = crate::testing::Sandbox::empty();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let main = root.join("repo");
    std::fs::create_dir_all(&main).unwrap();
    crate::testing::init_repo(&main, "main");
    let git = |args: &[&str]| {
        let mut full = vec!["-c", "user.name=t", "-c", "user.email=t@example.com"];
        full.extend_from_slice(args);
        let out = crate::infra::git::git(&full, Some(&main)).unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["commit", "--allow-empty", "-q", "-m", "first"]);
    // Two worktrees called `foo` and one called `main` are the ids that carry a digest.
    let worktrees = ["a/foo", "b/foo", "bar", "c/main"];
    for (n, rel) in worktrees.iter().enumerate() {
        let path = root.join(rel);
        git(&[
            "worktree",
            "add",
            "-q",
            "-b",
            &format!("b{n}"),
            path.to_str().unwrap(),
        ]);
    }
    let record = |worktree: &Path, body: Value| {
        let path = crate::registry::worker_record_path(worktree);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body.to_string()).unwrap();
    };
    let me = std::process::id();
    // A recorded tmux window on a socket no server answers on: read through the lazy view,
    // which finds no window there.
    record(
        &root.join("bar"),
        json!({"pid": me, "title": "bar work", "task": "WID-2", "hub": "WID-1",
               "startedAt": "2026-01-01T00:00:00Z",
               "terminal": {"backend": "tmux", "socket": "adj-unit-none", "session": "s",
                            "window": "@1", "pane": "%1"}}),
    );
    record(
        &root.join("a/foo"),
        json!({"pid": 4294967295u64, "task": "WID-3"}),
    );
    record(
        &main,
        json!({"pid": me, "task": "WID-4", "title": "on main"}),
    );
    // A hub record for the parent-task hub, and a gate it has open for `bar`.
    let hub_slug = crate::kernel::identity::slug_for("acme/widget", Some("WID-1"));
    let hub_record = crate::registry::hub_record_path(&sandbox.state(), &hub_slug);
    std::fs::create_dir_all(hub_record.parent().unwrap()).unwrap();
    std::fs::write(
        &hub_record,
        json!({"pid": me, "cwd": main, "hub": "WID-1", "startedAt": "2026-01-01T00:00:00Z"})
            .to_string(),
    )
    .unwrap();
    // The task `bar` is on, written under the hub it reports to.
    let tasks = task::dir(&sandbox.state(), &hub_slug);
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        tasks.join("WID-2.json"),
        json!({"id": "WID-2", "kind": "investigate", "title": "Retry the upload",
               "doneWhen": "report-only", "autoStart": true, "status": "dispatched",
               "createdAt": "20260101T000000Z", "updatedAt": "20260101T000000Z"})
        .to_string(),
    )
    .unwrap();
    let gates = sandbox.state().join("gates").join(&hub_slug);
    std::fs::create_dir_all(&gates).unwrap();
    std::fs::write(
        gates.join("g1.json"),
        json!({"id": "g1", "kind": "question", "worktree": root.join("bar"),
               "title": "which", "openedAt": "20991231T000000Z", "wait": true})
        .to_string(),
    )
    .unwrap();

    let repo = crate::kernel::identity::RepoInfo {
        main: main.to_string_lossy().to_string(),
        nwo: "acme/widget".to_string(),
        repo: "widget".to_string(),
        hub: None,
        slug: "acme-widget".to_string(),
        hub_name: "adjutant-acme-widget".to_string(),
        nwo_source: "dirname",
    };
    let server = Server {
        ctx: crate::registry::context_of(repo).unwrap(),
        token: String::new(),
        port: 0,
        resident: false,
        jules: Arc::default(),
        hub_titles: Arc::default(),
        last_lines: Arc::default(),
        tmux: None,
        terminals: Arc::default(),
        pr_poll: None,
    };
    let settings = settings_now(&server);

    // What the page is sent, which is the whole list.
    let listed = state(&server, true, false)["sessions"]
        .as_array()
        .unwrap()
        .clone();
    let ids: Vec<&str> = listed.iter().map(|s| s["id"].as_str().unwrap()).collect();
    let digest =
        |rel: &str| crate::kernel::identity::short_digest(root.join(rel).to_str().unwrap());
    for expected in [
        "hub".to_string(),
        "hub-WID-1".to_string(),
        "worker-bar".to_string(),
        "worker-main".to_string(),
        format!("worker-foo-{}", digest("a/foo")),
        format!("worker-foo-{}", digest("b/foo")),
        format!("worker-main-{}", digest("c/main")),
    ] {
        assert!(
            ids.contains(&expected.as_str()),
            "{expected} not in {ids:?}"
        );
    }
    for session in &listed {
        let id = session["id"].as_str().unwrap();
        let one = board_session(&server, &settings, id).unwrap();
        assert_eq!(&serde_json::to_value(&one).unwrap(), session, "{id}");
    }
    let waiting = |id: &str| listed.iter().find(|s| s["id"] == id).unwrap()["waiting"].clone();
    assert_eq!(waiting("worker-bar")["id"], "g1", "{listed:?}");
    // The worker's task title is the record's, and a worker whose task has no record, or
    // that has none, names none; the tab's own title is left alone.
    let task_title = |id: &str| listed.iter().find(|s| s["id"] == id).unwrap()["taskTitle"].clone();
    assert_eq!(task_title("worker-bar"), "Retry the upload");
    assert_eq!(task_title("worker-main"), Value::Null);
    let bar = listed.iter().find(|s| s["id"] == "worker-bar").unwrap();
    assert_eq!(bar["title"], "bar work");
    assert_eq!(waiting("hub-WID-1"), Value::Null);
    assert_eq!(board_session(&server, &settings, "worker-nope"), None);
    assert_eq!(board_session(&server, &settings, "hub-nope"), None);
}

#[test]
fn one_tmux_server_is_one_key_however_a_session_names_its_socket() {
    // A directory that is not there, so nothing is resolved and nothing is read from the
    // environment: the keys are what the spellings alone make of them.
    let key = |socket| socket_key_in(socket, None, Some("/nonexistent-tmux-dir"), 501);
    let default_path = key(None).to_string_lossy().to_string();
    assert_eq!(default_path, "/nonexistent-tmux-dir/tmux-501/default");
    assert_eq!(key(Some(&default_path)), key(None));
    assert_eq!(key(Some("  ")), key(None));
    assert_eq!(key(Some("default")), key(None));
    assert_ne!(key(Some("another")), key(None));
}
