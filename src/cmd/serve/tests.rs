use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::infra::http::{self, Request};
use crate::{gate, messaging, task};

use super::assets::{UI_HTML, vendor_asset};
use super::auth::{is_own_origin, refuse};
use super::daemon::{names_resident, private_log};
use super::index::{WorkerSeen, board_counts};
use super::registry::{Served, boards_dir, prefer, resident_board_url};
use super::resident::split_board_path;
use super::routes::{hub_route, is_page_path, session_route, session_route_for, terminal_route};
use super::state::{
    LAST_LINE_MIN_AGE, branch_of, cut_chars, history_id, history_of, socket_key_in, state,
    with_records, worker_session_ids,
};
use super::*;

#[test]
fn forgetting_a_board_removes_only_that_slug() {
    let _sandbox = crate::testing::Sandbox::empty();
    std::fs::create_dir_all(boards_dir()).unwrap();
    for slug in ["acme-widget-a", "acme-widget-b"] {
        std::fs::write(boards_dir().join(format!("{slug}.json")), "{}").unwrap();
    }
    forget_board("acme-widget-a").unwrap();
    assert!(!boards_dir().join("acme-widget-a.json").exists());
    assert!(boards_dir().join("acme-widget-b.json").exists());
    // Nothing to forget is not an error.
    forget_board("acme-widget-a").unwrap();
}

#[test]
fn the_dashboards_record_is_written_whole() {
    let sandbox = crate::testing::Sandbox::empty();
    assert_eq!(record("acme-widget", 4321), Ok(true));
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
    assert_eq!(written["psStarted"], json!(messaging::ps_started(pid)));
    assert_eq!(dashboards_running("acme-widget"), Some(4321));
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
fn the_page_pieces_join_into_one_document() {
    // A piece left out or put out of order shows here rather than as a blank page.
    assert!(UI_HTML.starts_with("<!DOCTYPE html>"));
    assert!(UI_HTML.trim_end().ends_with("</html>"));
    for tag in [
        "<style>",
        "</style>",
        "<script>",
        "</script>",
        "<body>",
        "</body>",
    ] {
        assert_eq!(UI_HTML.matches(tag).count(), 1, "{tag}");
    }
    let at = |tag: &str| UI_HTML.find(tag).unwrap();
    assert!(at("<style>") < at("</style>"));
    assert!(at("</style>") < at("<body>"));
    assert!(at("<script>") < at("</script>"));
    assert!(at("</script>") < at("</body>"));
}

#[test]
fn the_page_has_one_task_panel_and_no_drawer() {
    for piece in [
        "id=\"task-panel\"",
        "id=\"tp-term-host\"",
        "data-pane=",
        "panelSide",
        "rail-icons",
        "function renderTaskPanel",
        "function openTaskPanel",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    for gone in [
        "id=\"task-drawer\"",
        "function renderDrawer",
        "sidesheet-header",
        // The card's button for the terminal tab outside the board. The task view and the
        // review view keep theirs, which have a `style=` between the class and the title.
        "class=\"m3-icon-button\" title=\"ターミナルのworkerタブを前面表示\"",
    ] {
        assert!(!UI_HTML.contains(gone), "{gone}");
    }
    // The terminal's host sits inside the panel, which sits between the sidebar and the page.
    let at = |piece: &str| UI_HTML.find(piece).unwrap();
    assert!(at("id=\"nav-rail\"") < at("id=\"task-panel\""));
    assert!(at("id=\"task-panel\"") < at("id=\"tp-term-host\""));
    assert!(at("id=\"tp-term-host\"") < at("id=\"app-main\""));
}

#[test]
fn the_page_lists_sessions_in_a_view_and_has_no_overlay() {
    for piece in [
        "id=\"sessions-view\"",
        "id=\"tab-sessions\"",
        "SESS_REF = 'session:'",
        "function boardOfSession",
        "mountSessionTerminal(",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    for gone in [
        "term-overlay",
        "openTerminalOverlay",
        "closeTerminalOverlay",
    ] {
        assert!(!UI_HTML.contains(gone), "{gone}");
    }
    // The script uses what `terminal.js` and `actions.js` define, and `main.js` calls it.
    let at = |piece: &str| UI_HTML.find(piece).unwrap();
    assert!(at("function mountSessionTerminal") < at("function sessionState"));
    assert!(at("function sessionState") < at("setInterval(refresh, 2000)"));
}

#[test]
fn the_hub_terminal_bar_leads_with_the_reset() {
    assert!(UI_HTML.contains("function hubResetButton"));
    let helper = UI_HTML.split("function hubResetButton").nth(1).unwrap();
    let helper = &helper[..helper.find("\nfunction ").unwrap()];
    assert!(helper.contains("s.kind === 'hub'"));
    assert!(helper.contains("hubStartWhy(h) || hubWaitWhy(h)"));
    let buttons = UI_HTML.split("function sessionButtons").nth(1).unwrap();
    let buttons = &buttons[..buttons.find("\n}\n").unwrap()];
    assert!(buttons.contains("hubResetButton(s)"));
    let bar = UI_HTML.split("function termBarHtml").nth(1).unwrap();
    let bar = &bar[..bar.find("\n}\n").unwrap()];
    let gap = bar.find("tp-bar-gap").unwrap();
    assert!(bar.contains("actions ? hubResetButton(s) : null"));
    let reset = bar.find("actionButtonHtml(reset)").unwrap();
    let again = bar.find("data-tp-reconnect").unwrap();
    let acts = bar.find("sessionButtons(s).bar").unwrap();
    assert!(gap < reset && reset < again && again < acts);
}

#[test]
fn the_sessions_view_keeps_its_session_actions_and_dialogs() {
    for piece in [
        "id=\"sess-notice\"",
        "id=\"cleanup-dialog\"",
        "hub-reset",
        "function hubReset",
        "id=\"hub-stop-icon\"",
        "function hubRestart",
        "function sessRestart",
        "id=\"sess-restart-dialog\"",
        "セッションを再起動",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
}

#[test]
fn the_sessions_tab_patches_its_list() {
    for piece in [
        "function patchSessionList",
        "data-gid=",
        "function sessionTitle",
        "function hubTitle",
        "sessionLabel(s, false, g.data), sessionTip(s, st, g.data)",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    // The tab's title is for the tooltip: it does not stand in for the task's.
    assert!(!UI_HTML.contains("if (s.title) return s.title"));
    // A row's own words are what a redraw follows, not the selected session's task.
    assert!(!UI_HTML.contains("JSON.stringify([s.task, s.phase"));
}

#[test]
fn the_tab_title_names_the_board() {
    for piece in [
        "function boardTitle",
        "function pageHub",
        "+ boardTitle()",
        "— adj`",
        "'すべて — adj'",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    assert!(!UI_HTML.contains("+ 'adj';"));
}

#[test]
fn a_session_with_no_task_opens_in_the_task_panel() {
    for piece in [
        "const SESS_REF = 'session:'",
        "const sessOfRef",
        "function panelRefOf",
        "function sessDetailHtml",
        "function sessPanelHeadHtml",
        "タスクのないセッション",
        "data-side-act=\"${act}\"",
        "sideBtn('link-new'",
        "const task = SESS_REF + s.id",
        "function boardApi",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    // The Sessions view has no terminal area or sidebar of its own any more.
    for gone in [
        "id=\"sess-term-host\"",
        "id=\"sess-side\"",
        "id=\"sess-context\"",
        "function mountSelected",
        "function renderSessionSidebar",
        "sess-term-open",
        "`&session=${",
        "heldBySessions",
    ] {
        assert!(!UI_HTML.contains(gone), "{gone}");
    }
    // The panel's detail is defined before `main.js` starts polling.
    let at = |piece: &str| UI_HTML.find(piece).unwrap();
    assert!(at("function sessDetailHtml") < at("setInterval(refresh, 2000)"));
}

#[test]
fn the_sessions_view_can_start_and_link_sessions() {
    for piece in [
        "id=\"sess-add\"",
        "aria-haspopup=\"menu\"",
        "id=\"sess-add-menu\"",
        "id=\"hubkey-dialog\"",
        "id=\"start-dialog\"",
        "id=\"link-dialog\"",
        "function proposeName",
        "function worktreeNameProblem",
        "function sessionPendingRows",
        "function openLinkDialog",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    // The script that draws the tree calls into the one that knows the pending rows, which
    // is defined after it and before `main.js` starts polling.
    let at = |piece: &str| UI_HTML.find(piece).unwrap();
    assert!(at("function sessDetailHtml") < at("function sessionPendingRows"));
    assert!(at("function sessionPendingRows") < at("setInterval(refresh, 2000)"));
}

#[test]
fn a_session_with_no_task_is_a_card_on_the_agent_board() {
    for piece in [
        "function sessionCard",
        "data-sess-link=",
        "data-sess-open=",
        "タスクなし",
        "lastOutputText(s)",
        "+ bareIn.length;",
        "!displayItems.length && !bareIn.length",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    for gone in ["function ghostEl", ".card.ghost"] {
        assert!(!UI_HTML.contains(gone), "{gone}");
    }
}

#[test]
fn a_long_focus_is_cut_on_a_character_boundary() {
    assert_eq!(cut_chars("短い", 400), "短い");
    assert_eq!(cut_chars("あいうえお", 3), "あいう…");
}

fn request(method: &str, path: &str, headers: &[(&str, &str)]) -> Request {
    Request {
        method: method.to_string(),
        path: path.to_string(),
        query: match path.split_once("?token=") {
            Some((_, token)) => vec![("token".to_string(), token.to_string())],
            None => Vec::new(),
        },
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: Vec::new(),
    }
}

#[test]
fn a_board_path_names_one_slug() {
    assert_eq!(
        split_board_path("/b/acme-x-1/api/state"),
        Some(("acme-x-1", "/api/state"))
    );
    assert_eq!(split_board_path("/b/acme-x-1"), Some(("acme-x-1", "/")));
    assert_eq!(split_board_path("/b/acme-x-1/"), Some(("acme-x-1", "/")));
    for refused in [
        "/b//x",
        "/b/../x",
        "/b/ACME/",
        "/b/",
        "/api/state",
        "/board/x",
    ] {
        assert_eq!(split_board_path(refused), None, "{refused}");
    }
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
fn the_resident_is_preferred_over_a_dedicated_board() {
    assert_eq!(prefer(Some(1), Some(2)), Some(Served::Resident(1)));
    assert_eq!(prefer(Some(1), None), Some(Served::Resident(1)));
    assert_eq!(prefer(None, Some(2)), Some(Served::Dedicated(2)));
    assert_eq!(prefer(None, None), None);
}

#[test]
fn a_post_to_a_board_path_needs_the_same_origin() {
    let path = "/b/acme-x-1/api/tasks";
    let with = |origin: Option<&str>| {
        let mut headers = vec![("X-Adjutant-Token", "t")];
        headers.extend(origin.map(|o| ("Origin", o)));
        refuse("t", 4577, &request("POST", path, &headers))
    };
    assert_eq!(with(Some("http://127.0.0.1:4577")), None);
    assert_eq!(with(Some("http://localhost:4577")), None);
    assert_eq!(
        with(Some("http://127.0.0.1:9999")),
        Some((403, "cross-origin request"))
    );
    assert_eq!(
        with(Some("https://example.com")),
        Some((403, "cross-origin request"))
    );
    assert_eq!(with(None), Some((403, "no Origin header")));
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
fn a_hub_route_names_an_id_and_an_action() {
    fn route(path: &str) -> Option<(String, &str)> {
        hub_route(path).map(|(id, action)| (id.unwrap(), action))
    }
    assert_eq!(route("/api/hubs/hub/start"), Some(("hub".into(), "start")));
    assert_eq!(
        route("/api/hubs/hub-wid-957/stop"),
        Some(("hub-wid-957".into(), "stop"))
    );
    // As `encodeURIComponent` sends them.
    assert_eq!(
        route("/api/hubs/hub-foo%2Fbar/start"),
        Some(("hub-foo/bar".into(), "start"))
    );
    assert_eq!(
        route("/api/hubs/hub-%E8%A6%AA%20%E3%82%AD%E3%83%BC/stop"),
        Some(("hub-親 キー".into(), "stop"))
    );
    assert_eq!(
        route("/api/hubs/hub-wid-957/close"),
        Some(("hub-wid-957".into(), "close"))
    );
    assert_eq!(
        route("/api/hubs/hub-wid-957/reset"),
        Some(("hub-wid-957".into(), "reset"))
    );
    assert_eq!(
        route("/api/hubs/hub-wid-957/restart"),
        Some(("hub-wid-957".into(), "restart"))
    );
    assert_eq!(route("/api/hubs/a+b/start"), Some(("a+b".into(), "start")));
    // An encoding that is wrong is the caller's mistake to be told, not another route.
    for bad in [
        "/api/hubs/hub-%zz/start",
        "/api/hubs/hub-%2/start",
        "/api/hubs/%FF/start",
    ] {
        assert!(hub_route(bad).is_some_and(|(id, _)| id.is_err()), "{bad}");
    }
    for refused in [
        "/api/hubs/hub/rewind",
        "/api/hubs//start",
        "/api/hubs/a/b/start",
        "/api/hubs/hub",
        "/api/tasks/hub/start",
    ] {
        assert!(hub_route(refused).is_none(), "{refused}");
    }
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
fn the_page_places_a_pr_by_its_turn_and_says_when_the_poll_has_stopped() {
    for piece in [
        "function prWaitsOnPerson",
        "if (prWaitsOnPerson(t, workerOf(t, data))) return 'prreview';",
        "'other-reviewer': ['レビュー待ち（他の人）'",
        "changes: ['修正の依頼あり'",
        "merge: ['マージ待ち'",
        "'ci-failed': ['CI 失敗'",
        "closed: ['閉じられた'",
        "PR の自動確認が止まっています",
        "title=\"PR の状態を読み直す\"",
    ] {
        assert!(UI_HTML.contains(piece), "the page lacks {piece}");
    }
}

#[test]
fn the_page_paths_are_the_page_and_nothing_under_them() {
    for path in ["/", "/index.html", "/review"] {
        assert!(is_page_path(path), "{path}");
    }
    for path in ["/api/state", "/api/boards", "/review/x", "/b/x/"] {
        assert!(!is_page_path(path), "{path}");
    }
}

#[test]
fn the_sidebar_lists_boards_and_has_no_hub_footer() {
    for piece in [
        "id=\"board-rows\"",
        "function renderBoardRows",
        "function parseUrl",
        "function urlOf",
        "history.pushState",
        "popstate",
        "function mergeStates",
        "id=\"f-board\"",
        "'?token='",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    for piece in ["id=\"hub-rows\"", "自律 hub", "function renderHubRows"] {
        assert!(!UI_HTML.contains(piece), "{piece}");
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

const TOKEN: &str = "s3cret";
const PORT: u16 = 4577;

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
fn the_page_opens_with_the_token_in_the_url() {
    let req = request("GET", "/?token=s3cret", &[]);
    assert_eq!(refuse(TOKEN, PORT, &req), None);
}

#[test]
fn no_token_is_refused() {
    let req = request("GET", "/", &[]);
    assert_eq!(
        refuse(TOKEN, PORT, &req),
        Some((403, "bad or missing token"))
    );
}

#[test]
fn a_wrong_token_is_refused() {
    let req = request("GET", "/?token=guess", &[]);
    assert_eq!(
        refuse(TOKEN, PORT, &req),
        Some((403, "bad or missing token"))
    );
}

/// The page's own calls carry the token in a header, which is also the half of the
/// CSRF defence a cross-site form cannot reproduce.
#[test]
fn a_post_from_our_own_page_is_allowed() {
    let req = request(
        "POST",
        "/api/tasks",
        &[
            ("X-Adjutant-Token", TOKEN),
            ("Origin", "http://127.0.0.1:4577"),
        ],
    );
    assert_eq!(refuse(TOKEN, PORT, &req), None);
}

/// The attack this exists for: a page on another site knows the token (it leaked
/// through a log, a screenshot, a shell history) and submits a form. It cannot set the
/// header, so it dies here even holding the secret.
#[test]
fn a_post_carrying_the_token_only_in_the_url_is_refused() {
    let req = request(
        "POST",
        "/api/tasks?token=s3cret",
        &[("Origin", "https://evil.example")],
    );
    assert_eq!(
        refuse(TOKEN, PORT, &req),
        Some((
            403,
            "state-changing requests must send the token as a header"
        ))
    );
}

#[test]
fn a_post_from_another_origin_is_refused() {
    let req = request(
        "POST",
        "/api/tasks",
        &[
            ("X-Adjutant-Token", TOKEN),
            ("Origin", "https://evil.example"),
        ],
    );
    assert_eq!(
        refuse(TOKEN, PORT, &req),
        Some((403, "cross-origin request"))
    );
}

/// A non-browser client (curl, a script) sends no `Origin`. It is refused for the
/// state-changing routes rather than trusted: there is no way to tell it from a
/// browser that stripped the header.
#[test]
fn a_post_with_no_origin_is_refused() {
    let req = request("POST", "/api/tasks", &[("X-Adjutant-Token", TOKEN)]);
    assert_eq!(refuse(TOKEN, PORT, &req), Some((403, "no Origin header")));
}

#[test]
fn localhost_and_the_loopback_address_are_the_same_origin() {
    for origin in [
        "http://127.0.0.1:4577",
        "http://localhost:4577",
        "http://[::1]:4577",
    ] {
        assert!(is_own_origin(origin, PORT), "{origin}");
    }
    assert!(!is_own_origin("http://127.0.0.1:4578", PORT));
    assert!(!is_own_origin("https://127.0.0.1:4577", PORT));
    assert!(!is_own_origin("http://127.0.0.1:4577.evil.example", PORT));
}

/// Refused before the body is read, so a caller cannot make this process allocate a
/// gigabyte by saying it is about to send one.
#[test]
fn an_oversized_body_is_refused_before_the_token_is_even_checked() {
    let huge = (http::MAX_BODY + 1).to_string();
    let req = request("POST", "/api/tasks", &[("Content-Length", &huge)]);
    assert_eq!(refuse(TOKEN, PORT, &req), Some((413, "body too large")));
}

fn upgrade_headers(origin: Option<&str>) -> Vec<(&str, &str)> {
    let mut headers = vec![
        ("Connection", "Upgrade"),
        ("Upgrade", "websocket"),
        ("Sec-WebSocket-Version", "13"),
        ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
    ];
    headers.extend(origin.map(|o| ("Origin", o)));
    headers
}

#[test]
fn a_handshake_with_no_origin_is_refused_even_with_the_token() {
    let req = request(
        "GET",
        "/b/x/api/sessions/hub/terminal?token=s3cret",
        &upgrade_headers(None),
    );
    assert_eq!(refuse(TOKEN, PORT, &req), Some((403, "no Origin header")));
}

#[test]
fn a_handshake_from_another_site_is_refused() {
    for origin in [
        "http://evil.example",
        "http://127.0.0.1:4578",
        "https://127.0.0.1:4577",
        "null",
    ] {
        let req = request(
            "GET",
            "/b/x/api/sessions/hub/terminal?token=s3cret",
            &upgrade_headers(Some(origin)),
        );
        assert_eq!(
            refuse(TOKEN, PORT, &req),
            Some((403, "cross-origin request")),
            "{origin}"
        );
    }
}

#[test]
fn a_handshake_needs_the_token_however_good_its_origin() {
    for path in [
        "/b/x/api/sessions/hub/terminal",
        "/b/x/api/sessions/hub/terminal?token=guess",
    ] {
        let req = request("GET", path, &upgrade_headers(Some("http://127.0.0.1:4577")));
        assert_eq!(
            refuse(TOKEN, PORT, &req),
            Some((403, "bad or missing token")),
            "{path}"
        );
    }
}

/// A browser cannot set headers on a WebSocket, so the token rides in the URL.
#[test]
fn a_handshake_from_the_boards_own_origin_may_carry_the_token_in_the_url() {
    for origin in ["http://127.0.0.1:4577", "http://localhost:4577"] {
        let req = request(
            "GET",
            "/b/x/api/sessions/hub/terminal?token=s3cret",
            &upgrade_headers(Some(origin)),
        );
        assert_eq!(refuse(TOKEN, PORT, &req), None, "{origin}");
    }
}

/// The origin rule is for handshakes: the page and its polling GETs are as they were.
#[test]
fn a_plain_get_is_still_asked_for_no_origin() {
    let req = request("GET", "/b/x/api/state?token=s3cret", &[]);
    assert_eq!(refuse(TOKEN, PORT, &req), None);
}

#[test]
fn a_terminal_route_names_a_session_id() {
    fn id(path: &str) -> Option<String> {
        terminal_route(path).map(|id| id.unwrap())
    }
    assert_eq!(id("/api/sessions/hub/terminal"), Some("hub".into()));
    assert_eq!(
        id("/api/sessions/worker-widget/terminal"),
        Some("worker-widget".into())
    );
    // As `encodeURIComponent` sends a hub id with a slash or a space in it.
    assert_eq!(
        id("/api/sessions/hub-foo%2Fbar/terminal"),
        Some("hub-foo/bar".into())
    );
    assert_eq!(
        id("/api/sessions/hub-%E8%A6%AA%20%E3%82%AD%E3%83%BC/terminal"),
        Some("hub-親 キー".into())
    );
    for bad in [
        "/api/sessions/hub-%zz/terminal",
        "/api/sessions/%FF/terminal",
    ] {
        assert!(terminal_route(bad).is_some_and(|id| id.is_err()), "{bad}");
    }
    for other in [
        "/api/sessions//terminal",
        "/api/sessions/a/b/terminal",
        "/api/sessions/hub",
        "/api/sessions/hub/terminal/x",
        "/api/hubs/hub/terminal",
        "/terminal",
        "/",
    ] {
        assert!(terminal_route(other).is_none(), "{other}");
    }
}

#[test]
fn a_session_route_names_a_session_id_and_an_action() {
    fn id(path: &str, action: &str) -> Option<String> {
        session_route_for(path, action).map(|id| id.unwrap())
    }
    for action in ["link", "git", "resume", "restart", "open", "cleanup"] {
        assert_eq!(
            id(&format!("/api/sessions/worker-x/{action}"), action),
            Some("worker-x".into()),
            "{action}"
        );
        // As `encodeURIComponent` sends an id with a slash or a space in it.
        assert_eq!(
            id(&format!("/api/sessions/worker-a%2Fb%20c/{action}"), action),
            Some("worker-a/b c".into()),
            "{action}"
        );
        assert!(
            session_route_for(&format!("/api/sessions/%FF/{action}"), action)
                .is_some_and(|id| id.is_err()),
            "{action}"
        );
        for other in [
            "/api/sessions".to_string(),
            format!("/api/sessions//{action}"),
            format!("/api/sessions/a/b/{action}"),
            format!("/api/sessions/worker-x/{action}/x"),
            format!("/api/tasks/x/{action}"),
        ] {
            assert!(session_route(&other).is_none(), "{other}");
        }
    }
    // An action is not another's route, and the terminal is not one of these.
    assert!(session_route_for("/api/sessions/worker-x/git", "link").is_none());
    assert!(session_route("/api/sessions/worker-x/terminal").is_none());
    assert!(session_route("/api/sessions/worker-x/remove").is_none());
}

#[test]
fn the_terminal_assets_are_only_served_by_the_resident_server() {
    assert!(vendor_asset("/vendor/xterm.js", false).is_none());
    assert!(vendor_asset("/vendor/xterm.css", false).is_none());
    let (kind, js) = vendor_asset("/vendor/xterm.js", true).unwrap();
    assert!(kind.starts_with("text/javascript"));
    assert!(js.starts_with("/*! xterm.js - MIT License"));
    assert!(js.contains("Permission is hereby granted"));
    // The library and both addons come in the one script, each defining its global.
    for global in ["Terminal", "FitAddon", "Unicode11Addon"] {
        assert!(js.contains(global), "{global}");
    }
    assert!(!js.contains("sourceMappingURL"));
    let (kind, css) = vendor_asset("/vendor/xterm.css", true).unwrap();
    assert!(kind.starts_with("text/css"));
    assert!(css.starts_with("/*! xterm.js - MIT License"));
    assert!(css.contains(".xterm"));
    assert!(vendor_asset("/vendor/other.js", true).is_none());
}

/// The library is fetched by a page that opens a terminal, not carried by every page.
#[test]
fn the_page_does_not_carry_the_terminal_library() {
    assert!(!UI_HTML.contains("Permission is hereby granted"));
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
        let path = messaging::worker_record_path(worktree);
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
    let hub_record = messaging::hub_record_path(&sandbox.state(), &hub_slug);
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
        ctx: super::super::context_of(repo).unwrap(),
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
