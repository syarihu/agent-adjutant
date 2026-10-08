use std::collections::BTreeSet;

use crate::board::view::Lines;
use crate::infra::http::{self, Request};

use super::assets::{UI_HTML, vendor_asset};
use super::auth::{is_own_origin, refuse};
use super::resident::split_board_path;
use super::routes::{HubAction, Route, SessionAction};

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
fn the_page_shows_the_parent_and_adds_a_child_through_the_new_task_form() {
    for piece in [
        // The row, and the button that opens the form with the parent filled in.
        "function parentRowHtml",
        "data-add-child=",
        "function openChildForm",
        "子タスクを足す",
        "記録と違う",
        // The page merging several boards keeps their parents.
        "parents: parts.flatMap(p => tag(p.data.parents, p.slug))",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    // The button carries its parent in an attribute and is wired by a listener, not inline.
    assert!(!UI_HTML.contains("onclick=\"openChildForm"));
}

#[test]
fn the_page_has_one_task_panel_and_no_drawer() {
    for piece in [
        "id=\"task-panel\"",
        "id=\"tp-term-host\"",
        "data-pane=",
        "panelDock",
        "panelDialog",
        "rail-icons",
        "function renderTaskPanel",
        "function openTaskPanel",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    for gone in [
        "id=\"task-drawer\"",
        "function renderDrawer",
        "let panelPop",
        "sidesheet-header",
        // The card's button for the terminal tab outside the board. The review view keeps
        // its own, which have a `style=` between the class and the title.
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
fn the_task_panel_has_the_tabs_and_the_full_view_is_gone() {
    for piece in [
        "const PANES = ['detail', 'term', 'review', 'check', 'history']",
        "id=\"tp-tabs\"",
        "m3-segmented-tabs",
        "タスクサマリ",
        "コードレビュー",
        "動作確認",
        "function drawTaskPane",
        "function showPanelTab",
        "function openTask(",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    // The task's own full view was retired into these tabs.
    for gone in [
        "renderTaskView",
        "backToBoard",
        "redrawTaskView",
        "id=\"task-view\"",
        "view === 'task'",
        "view !== 'task'",
        "taskView",
        "TASK_TABS",
        "#task-view",
        "with-rail",
    ] {
        assert!(!UI_HTML.contains(gone), "{gone}");
    }
    // The panel's decision buttons go through `decideAct`; `bindDecide` would bind `data-ide` a
    // second time next to the panel's own delegated handler.
    let panel = include_str!("../../ui/task-panel.js");
    assert!(panel.contains("decideAct("));
    assert!(!panel.contains("bindDecide("));
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
fn the_page_wires_no_inline_handlers() {
    // A name in an attribute string only fails when clicked; the table is checked in one place.
    for attr in ["onclick=\"", "onchange=\"", "oninput=\"", "onsubmit=\""] {
        assert!(!UI_HTML.contains(attr), "{attr}");
    }
    assert_eq!(UI_HTML.matches("const ACTIONS = {").count(), 1);
    let table = UI_HTML.split("const ACTIONS = {").nth(1).unwrap();
    let table = &table[..table.find("\n};").unwrap()];
    let keys: Vec<&str> = table
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .map(|l| l.split(':').next().unwrap().trim().trim_matches('\''))
        .collect();
    assert!(!keys.is_empty());
    let keys_set: BTreeSet<&str> = keys.iter().copied().collect();
    assert_eq!(keys.len(), keys_set.len(), "a key twice");
    let used: BTreeSet<&str> = UI_HTML
        .split("data-action=\"")
        .skip(1)
        .map(|s| &s[..s.find('"').unwrap()])
        .collect();
    assert_eq!(keys_set, used);
}

#[test]
fn the_hub_terminal_bar_leads_with_the_reset() {
    assert!(UI_HTML.contains("function hubResetButton"));
    let helper = UI_HTML.split("function hubResetButton").nth(1).unwrap();
    let helper = &helper[..helper.find("\nfunction ").unwrap()];
    assert!(helper.contains("s.kind === 'hub'"));
    assert!(helper.contains("hubStartWhy(h) || hubWaitWhy(h)"));
    let buttons = UI_HTML.split("function sessionButtons").nth(1).unwrap();
    let buttons = &buttons[..buttons.find("\n\u{7d}\n").unwrap()];
    assert!(buttons.contains("hubResetButton(s)"));
    let bar = UI_HTML.split("function termBarHtml").nth(1).unwrap();
    let bar = &bar[..bar.find("\n\u{7d}\n").unwrap()];
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
        "`&session=$\u{7b}",
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
        "<span class=\"card-worker-cmd\">${esc(agentText(s))}</span>",
        ".session-card .card-worker-status { flex-wrap: wrap; }",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    for gone in ["function ghostEl", ".card.ghost"] {
        assert!(!UI_HTML.contains(gone), "{gone}");
    }
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

fn parsed(method: &str, path: &str) -> Result<Option<Route>, String> {
    Route::parse(&request(method, path, &[]))
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
fn a_hub_route_names_an_id_and_an_action() {
    fn route(path: &str) -> Option<(String, HubAction)> {
        match parsed("POST", path) {
            Ok(Some(Route::Hub(id, action))) => Some((id, action)),
            _ => None,
        }
    }
    assert_eq!(
        route("/api/hubs/hub/start"),
        Some(("hub".into(), HubAction::Start))
    );
    assert_eq!(
        route("/api/hubs/hub-wid-957/stop"),
        Some(("hub-wid-957".into(), HubAction::Stop))
    );
    // As `encodeURIComponent` sends them.
    assert_eq!(
        route("/api/hubs/hub-foo%2Fbar/start"),
        Some(("hub-foo/bar".into(), HubAction::Start))
    );
    assert_eq!(
        route("/api/hubs/hub-%E8%A6%AA%20%E3%82%AD%E3%83%BC/stop"),
        Some(("hub-親 キー".into(), HubAction::Stop))
    );
    assert_eq!(
        route("/api/hubs/hub-wid-957/close"),
        Some(("hub-wid-957".into(), HubAction::Close))
    );
    assert_eq!(
        route("/api/hubs/hub-wid-957/reset"),
        Some(("hub-wid-957".into(), HubAction::Reset))
    );
    assert_eq!(
        route("/api/hubs/hub-wid-957/restart"),
        Some(("hub-wid-957".into(), HubAction::Restart))
    );
    assert_eq!(
        route("/api/hubs/a+b/start"),
        Some(("a+b".into(), HubAction::Start))
    );
    // An encoding that is wrong is the caller's mistake to be told, not another route.
    for bad in [
        "/api/hubs/hub-%zz/start",
        "/api/hubs/hub-%2/start",
        "/api/hubs/%FF/start",
    ] {
        assert!(parsed("POST", bad).is_err(), "{bad}");
    }
    for refused in [
        "/api/hubs/hub/rewind",
        "/api/hubs//start",
        "/api/hubs/a/b/start",
        "/api/hubs/hub",
    ] {
        assert_eq!(parsed("POST", refused), Ok(None), "{refused}");
    }
    assert_eq!(
        parsed("POST", "/api/tasks/hub/start"),
        Ok(Some(Route::UpdateTask("start".into())))
    );
    // The hub actions are posts.
    assert_eq!(parsed("GET", "/api/hubs/hub/start"), Ok(None));
}

#[test]
fn the_page_places_a_pr_by_its_turn_and_says_when_the_poll_has_stopped() {
    for piece in [
        "t.waitsOnPerson ? 'prreview'",
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

/// The body of one table of the page: what lies between `open` and the first `close` after it.
fn page_table(open: &str, close: &str) -> &'static str {
    assert_eq!(UI_HTML.matches(open).count(), 1, "{open}");
    let body = &UI_HTML[UI_HTML.find(open).unwrap() + open.len()..];
    &body[..body
        .find(close)
        .unwrap_or_else(|| panic!("{open} has no {close}"))]
}

/// Whether `body` has `name` as a key, `name:` or `'name':`, and not as the end of a longer word.
fn has_key(body: &str, name: &str) -> bool {
    body.contains(&format!("'{name}':"))
        || body.match_indices(&format!("{name}:")).any(|(i, _)| {
            body[..i]
                .chars()
                .last()
                .is_none_or(|c| !(c.is_alphanumeric() || "_-'".contains(c)))
        })
}

fn serde_name(v: impl serde::Serialize) -> String {
    serde_json::to_value(v)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn the_page_has_a_word_for_every_value_the_server_sends() {
    use crate::board::view::{HumanCol, Progress};
    use crate::gate::Kind;
    use crate::task::PrTurn;

    // Each list sits beside a `match` with no wildcard arm: a new variant does not compile until
    // it is added to the list beside it.
    let kinds = [
        Kind::Plan,
        Kind::Diff,
        Kind::Verify,
        Kind::Dispatch,
        Kind::Issue,
        Kind::Question,
        Kind::Result,
        Kind::Relay,
    ];
    let _ = |k: Kind| match k {
        Kind::Plan
        | Kind::Diff
        | Kind::Verify
        | Kind::Dispatch
        | Kind::Issue
        | Kind::Question
        | Kind::Result
        | Kind::Relay => {}
    };
    let turns = [
        PrTurn::Draft,
        PrTurn::Unrequested,
        PrTurn::OtherReviewer,
        PrTurn::Checks,
        PrTurn::Changes,
        PrTurn::Merge,
        PrTurn::CiFailed,
        PrTurn::Merged,
        PrTurn::Closed,
    ];
    let _ = |t: PrTurn| match t {
        PrTurn::Draft
        | PrTurn::Unrequested
        | PrTurn::OtherReviewer
        | PrTurn::Checks
        | PrTurn::Changes
        | PrTurn::Merge
        | PrTurn::CiFailed
        | PrTurn::Merged
        | PrTurn::Closed => {}
    };
    let columns = [
        HumanCol::Dispatch,
        HumanCol::Plan,
        HumanCol::Diff,
        HumanCol::Verify,
        HumanCol::PrReview,
        HumanCol::Question,
    ];
    let _ = |c: HumanCol| match c {
        HumanCol::Dispatch
        | HumanCol::Plan
        | HumanCol::Diff
        | HumanCol::Verify
        | HumanCol::PrReview
        | HumanCol::Question => {}
    };

    let progress = [
        Progress::Merged,
        Progress::Done,
        Progress::Pr,
        Progress::Working,
        Progress::NotStarted,
    ];
    let _ = |p: Progress| match p {
        Progress::Merged
        | Progress::Done
        | Progress::Pr
        | Progress::Working
        | Progress::NotStarted => {}
    };

    let kinds_table = page_table("const KINDS = {", "};");
    let phase_label = page_table("const PHASE_LABEL = {", "};");
    let phase_col = page_table("const AGENT_COL_OF_PHASE = {", "};");
    let decision_label = page_table("const DECISION_LABEL = {", "};");
    let human_columns = page_table("const HUMAN_COLUMNS = [", "];");
    let turn_why = page_table("const PR_TURN_WHY = {", "};");
    let turn_pill = page_table("const PR_TURN = {", "};");
    let work_progress = page_table("const WORK_PROGRESS = {", "};");

    for kind in kinds {
        let name = serde_name(kind);
        assert!(has_key(kinds_table, &name), "KINDS has no gate kind {name}");
        for option in kind.default_options() {
            assert!(
                has_key(decision_label, &option),
                "DECISION_LABEL has no {option}, an option of a {name} gate"
            );
        }
    }
    for phase in crate::registry::PHASES {
        assert!(has_key(phase_label, phase), "PHASE_LABEL has no {phase}");
        assert!(
            has_key(phase_col, phase),
            "AGENT_COL_OF_PHASE has no {phase}"
        );
    }
    for turn in turns {
        let name = serde_name(turn);
        // A PR nobody was asked to review has no pill of its own, by design.
        if turn != PrTurn::Unrequested {
            assert!(has_key(turn_pill, &name), "PR_TURN has no turn {name}");
        }
        if turn.persons() {
            assert!(
                has_key(turn_why, &name),
                "PR_TURN_WHY has no turn {name}, which waits on the person"
            );
        }
    }
    for step in progress {
        let name = serde_name(step);
        assert!(
            has_key(work_progress, &name),
            "WORK_PROGRESS has no progress {name}"
        );
    }
    for column in columns {
        let name = serde_name(column);
        assert!(
            human_columns.contains(&format!("id:'{name}'")),
            "HUMAN_COLUMNS has no column {name}"
        );
    }
}

#[test]
fn the_page_paths_are_the_page_and_nothing_under_them() {
    for path in ["/", "/index.html", "/review"] {
        assert_eq!(parsed("GET", path), Ok(Some(Route::Page)), "{path}");
    }
    for path in ["/api/boards", "/review/x", "/b/x/"] {
        assert_eq!(parsed("GET", path), Ok(None), "{path}");
    }
    assert!(matches!(
        parsed("GET", "/api/state"),
        Ok(Some(Route::State { .. }))
    ));
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

const TOKEN: &str = "s3cret";
const PORT: u16 = 4577;

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
        match parsed("GET", path) {
            Ok(Some(Route::Terminal(id))) => Some(id),
            _ => None,
        }
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
        assert!(parsed("GET", bad).is_err(), "{bad}");
    }
    for other in [
        "/api/sessions//terminal",
        "/api/sessions/a/b/terminal",
        "/api/sessions/hub",
        "/api/sessions/hub/terminal/x",
        "/api/hubs/hub/terminal",
        "/terminal",
    ] {
        assert_eq!(parsed("GET", other), Ok(None), "{other}");
    }
    assert_eq!(parsed("GET", "/"), Ok(Some(Route::Page)));
    // Any method: the handshake never looked at it.
    assert!(matches!(
        parsed("POST", "/api/sessions/hub/terminal"),
        Ok(Some(Route::Terminal(_)))
    ));
}

#[test]
fn a_session_route_names_a_session_id_and_an_action() {
    fn method(action: &str) -> &'static str {
        if action == "git" { "GET" } else { "POST" }
    }
    fn route(id: String, action: &str) -> Route {
        match action {
            "link" => Route::LinkSession(id),
            "git" => Route::SessionGit(id),
            "resume" => Route::Session(id, SessionAction::Resume),
            "restart" => Route::Session(id, SessionAction::Restart),
            "open" => Route::Session(id, SessionAction::Open),
            _ => Route::Session(id, SessionAction::CleanUp),
        }
    }
    for action in ["link", "git", "resume", "restart", "open", "cleanup"] {
        let method = method(action);
        assert_eq!(
            parsed(method, &format!("/api/sessions/worker-x/{action}")),
            Ok(Some(route("worker-x".into(), action))),
            "{action}"
        );
        // As `encodeURIComponent` sends an id with a slash or a space in it.
        assert_eq!(
            parsed(method, &format!("/api/sessions/worker-a%2Fb%20c/{action}")),
            Ok(Some(route("worker-a/b c".into(), action))),
            "{action}"
        );
        assert!(
            parsed(method, &format!("/api/sessions/%FF/{action}")).is_err(),
            "{action}"
        );
        for other in [
            format!("/api/sessions//{action}"),
            format!("/api/sessions/a/b/{action}"),
            format!("/api/sessions/worker-x/{action}/x"),
        ] {
            for method in ["GET", "POST"] {
                assert_eq!(parsed(method, &other), Ok(None), "{method} {other}");
            }
        }
        let task_path = format!("/api/tasks/x/{action}");
        assert_eq!(parsed("GET", &task_path), Ok(None), "{task_path}");
        assert_eq!(
            parsed("POST", &task_path),
            Ok(Some(Route::UpdateTask(action.into()))),
            "{task_path}"
        );
    }
    assert_eq!(parsed("GET", "/api/sessions"), Ok(None));
    assert_eq!(
        parsed("POST", "/api/sessions"),
        Ok(Some(Route::StartSession))
    );
    // An action is not another's route, and the terminal is not one of these.
    assert_eq!(parsed("POST", "/api/sessions/worker-x/git"), Ok(None));
    assert_eq!(
        parsed("POST", "/api/sessions/worker-x/terminal"),
        Ok(Some(Route::Terminal("worker-x".into())))
    );
    assert_eq!(parsed("POST", "/api/sessions/worker-x/remove"), Ok(None));
}

#[test]
fn a_task_route_names_a_task_id() {
    assert_eq!(
        parsed("GET", "/api/tasks/t1/history"),
        Ok(Some(Route::TaskHistory("t1".into())))
    );
    for bad in ["/api/tasks//history", "/api/tasks/a/b/history"] {
        assert_eq!(parsed("GET", bad), Err("no such task".to_string()), "{bad}");
    }
    assert_eq!(parsed("GET", "/api/tasks/t1"), Ok(None));
    // The id is the segment as written, never decoded.
    assert_eq!(
        parsed("GET", "/api/tasks/a%2Fb/history"),
        Ok(Some(Route::TaskHistory("a%2Fb".into())))
    );
    assert_eq!(
        parsed("POST", "/api/tasks/a/b"),
        Ok(Some(Route::UpdateTask("b".into())))
    );
    assert_eq!(
        parsed("POST", "/api/worktrees/a/b"),
        Ok(Some(Route::Worktree("b".into())))
    );
}

#[test]
fn only_the_resident_serves_what_reaches_outside_the_repository() {
    for (method, path) in [
        ("POST", "/api/sessions/x/resume"),
        ("POST", "/api/sessions/x/open"),
        ("POST", "/api/hubs"),
        ("POST", "/api/hubs/x/start"),
        ("GET", "/api/sessions/x/terminal"),
        ("GET", "/vendor/xterm.js"),
    ] {
        let route = parsed(method, path).unwrap().unwrap();
        assert!(route.resident_only(), "{path}");
    }
    for (method, path) in [
        ("GET", "/"),
        ("GET", "/api/state"),
        ("GET", "/api/tasks/t1/history"),
        ("POST", "/api/tasks"),
        ("POST", "/api/sessions"),
        ("POST", "/api/sessions/x/link"),
        ("GET", "/api/sessions/x/git"),
        ("POST", "/api/hub/next"),
        ("POST", "/api/hub/wake"),
        ("POST", "/api/worktrees/focus"),
        ("POST", "/api/gates/g"),
    ] {
        let route = parsed(method, path).unwrap().unwrap();
        assert!(!route.resident_only(), "{path}");
    }
}

#[test]
fn the_terminal_assets_are_only_served_by_the_resident_server() {
    assert!(matches!(
        parsed("GET", "/vendor/xterm.js"),
        Ok(Some(r @ Route::Asset(..))) if r.resident_only()
    ));
    assert!(matches!(
        parsed("GET", "/vendor/xterm.css"),
        Ok(Some(r @ Route::Asset(..))) if r.resident_only()
    ));
    let (kind, js) = vendor_asset("/vendor/xterm.js").unwrap();
    assert!(kind.starts_with("text/javascript"));
    assert!(js.starts_with("/*! xterm.js - MIT License"));
    assert!(js.contains("Permission is hereby granted"));
    // The library and both addons come in the one script, each defining its global.
    for global in ["Terminal", "FitAddon", "Unicode11Addon"] {
        assert!(js.contains(global), "{global}");
    }
    assert!(!js.contains("sourceMappingURL"));
    let (kind, css) = vendor_asset("/vendor/xterm.css").unwrap();
    assert!(kind.starts_with("text/css"));
    assert!(css.starts_with("/*! xterm.js - MIT License"));
    assert!(css.contains(".xterm"));
    assert!(vendor_asset("/vendor/other.js").is_none());
    assert_eq!(parsed("GET", "/vendor/other.js"), Ok(None));
}

/// The library is fetched by a page that opens a terminal, not carried by every page.
#[test]
fn the_page_does_not_carry_the_terminal_library() {
    assert!(!UI_HTML.contains("Permission is hereby granted"));
}

#[test]
fn the_views_register_themselves() {
    // `render` and `switchBoard` go through the views' table, never their functions or caches.
    fn body_of(head: &str) -> &'static str {
        assert_eq!(UI_HTML.matches(head).count(), 1, "{head}");
        let rest = UI_HTML.split(head).nth(1).unwrap();
        &rest[..rest.find("\n\u{7d}\n").unwrap()]
    }
    let bodies = [
        body_of("function render() {"),
        body_of("function switchBoard() {"),
    ];
    for name in [
        "renderColumns",
        "renderBoardRows",
        "renderTitle",
        "renderSessionsTab",
        "openPendingSession",
        "renderSessionsView",
        "renderTaskPanel",
        "redrawReview",
        "hideTaskPanelState",
        "disposeTermSlot",
        "sessView",
        "histories",
        "historyFailed",
        "openReplies",
    ] {
        for body in bodies {
            assert!(!body.contains(name), "{name}");
        }
    }
    assert_eq!(UI_HTML.matches("const VIEW_ORDER = [").count(), 1);
    let order = UI_HTML.split("const VIEW_ORDER = [").nth(1).unwrap();
    let order = &order[..order.find(']').unwrap()];
    let names: Vec<&str> = order
        .split(',')
        .map(|n| n.trim().trim_matches('\''))
        .filter(|n| !n.is_empty())
        .collect();
    assert_eq!(names.len(), 12);
    assert_eq!(
        names.iter().collect::<BTreeSet<_>>().len(),
        names.len(),
        "a name twice"
    );
    for name in &names {
        assert_eq!(
            UI_HTML.matches(&format!("registerView('{name}'")).count(),
            1,
            "{name}"
        );
    }
    // No registration outside the order (it would throw at load).
    assert_eq!(UI_HTML.matches("registerView('").count(), names.len());
}

#[test]
fn a_session_waiting_on_a_prompt_is_listed_in_the_queue_the_person_checks() {
    assert!(
        !UI_HTML.contains("要対応レビュー"),
        "the queue is named 要対応 everywhere"
    );
    for piece in [
        // The merged state of every board carries the waits.
        "waits: parts.flatMap(p => tag(p.data.waits, p.slug))",
        // The queue lists them, and its button opens the session's terminal.
        "const waitRef = w =>",
        "const itemRef = x =>",
        "data-rv-open-wait",
        "openWait(w)",
        "permissionLabel({ agentSession: { request: w.request } })",
        // The badges count them beside the gates.
        "(b.waits || []).length",
        "(state.waits || []).length",
        "const boardWaiting = b =>",
        // A quiet wait is listed without a desktop notification.
        "w.quiet",
    ] {
        assert!(UI_HTML.contains(piece), "the page lacks {piece}");
    }
    assert!(UI_HTML.matches("boardWaiting(").count() >= 5);
}

#[test]
fn the_page_has_the_work_view_beside_the_review_queue() {
    for piece in [
        // The sidebar entry, the three-column view and the middle terminal's host.
        "id=\"nav-work\"",
        "data-action=\"work\"",
        "id=\"work-view\"",
        "id=\"wk-term-host\"",
        "id=\"wk-list-resize\"",
        "id=\"rail-resize\"",
        // The list is the resident's own document, not a merge made in the page.
        "boardApi('', '/api/work')",
        "view=work",
        "const DEFAULT_MULTI_VIEW = 'agent'",
        "const PARENT_REF = 'parent:'",
        // 「ターミナルで話す」 goes to the middle terminal in this view.
        "if (view === 'work') return focusWorkTerm();",
    ] {
        assert!(UI_HTML.contains(piece), "the page lacks {piece}");
    }
    // A board served alone has no list of boards to read the work from.
    assert!(UI_HTML.contains(".single-board #nav-work { display: none; }"));
    // The panel has no terminal tab in this view.
    assert!(UI_HTML.contains("const term = view === 'work' ? ''"));
}

#[test]
fn every_icon_only_button_has_a_name() {
    // A Material Symbols ligature alone names the button after the icon ("close").
    const ICON: &str = "<span class=\"material-symbols-outlined\"";
    let mut seen = 0;
    for part in UI_HTML.split("<button").skip(1) {
        // A `>` inside a `${...}` template expression does not end the tag.
        let (mut depth, mut gt) = (0, None);
        for (i, c) in part.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                '>' if depth == 0 => {
                    gt = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let (Some(gt), Some(end)) = (gt, part.find("</button>")) else {
            continue;
        };
        let (tag, inner) = (&part[..gt], &part[gt + 1..end]);
        let Some(at) = inner.find(ICON) else {
            continue;
        };
        let close = at + inner[at..].find("</span>").unwrap() + "</span>".len();
        // #btn-notify gets its visible label from updateNotifyButton on the first render.
        if !(inner[..at].trim().is_empty() && inner[close..].trim().is_empty())
            || tag.contains("id=\"btn-notify\"")
        {
            continue;
        }
        seen += 1;
        assert!(
            tag.contains("aria-label=\"") && !tag.contains("aria-label=\"\""),
            "<button{tag}>"
        );
        assert!(
            inner[at..close].contains("aria-hidden=\"true\""),
            "<button{tag}>"
        );
    }
    assert!(seen >= 18, "{seen}");
}

#[test]
fn waking_the_hub_is_a_post_to_its_own_route() {
    assert!(matches!(
        parsed("POST", "/api/hub/wake"),
        Ok(Some(Route::WakeHub))
    ));
    // Not something a page may only look at.
    assert!(matches!(parsed("GET", "/api/hub/wake"), Ok(None)));
}

#[test]
fn the_state_asks_for_the_last_lines_of_all_sessions_or_of_the_hubs_only() {
    let lines_of = |query: Option<&str>| {
        let mut req = request("GET", "/api/state", &[]);
        if let Some(value) = query {
            req.query.push(("lines".to_string(), value.to_string()));
        }
        match Route::parse(&req) {
            Ok(Some(Route::State { lines, .. })) => lines,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(lines_of(None), Lines::None);
    assert_eq!(lines_of(Some("1")), Lines::All);
    assert_eq!(lines_of(Some("hub")), Lines::Hubs);
    assert_eq!(lines_of(Some("0")), Lines::None);
    assert_eq!(lines_of(Some("hubs")), Lines::None);
}

#[test]
fn the_page_has_the_hubs_entry_and_its_wake_button() {
    for piece in [
        "id=\"hub-strip-slot\"",
        "data-action=\"wake-hub\"",
        "data-action=\"hub-strip-open\"",
        "function renderHubStrip",
        "/api/hub/wake",
        "/api/state?lines=hub",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    // Above the agent columns, not inside them: a redraw of the columns must not be needed
    // to move it.
    let at = |piece: &str| UI_HTML.find(piece).unwrap();
    assert!(at("id=\"pane-agent\"") < at("id=\"hub-strip-slot\""));
    assert!(at("id=\"hub-strip-slot\"") < at("id=\"board-agent\""));
}

#[test]
fn card_issue_and_pr_chips_sit_together_and_agent_columns_are_320px() {
    assert!(UI_HTML.contains("<span class=\"card-gh-chips\">${issue}${pr}</span>"));
    assert!(UI_HTML.contains("PR #${esc(prNumber)}"));
    assert!(UI_HTML.contains("#board-agent .col { flex: 0 0 320px; max-width: 320px; }"));
}

#[test]
fn the_panel_head_links_the_task_issue_and_pr() {
    assert!(UI_HTML.contains("function ghHeadLinksHtml"));
    assert!(UI_HTML.contains("function issueLabelOf"));
    let head = UI_HTML.split("function panelHeadHtml").nth(1).unwrap();
    let head = &head[..head.find("\n\u{7d}\n").unwrap()];
    let links = head.find("ghHeadLinksHtml(task)").unwrap();
    let origin = head.find("origin-chip").unwrap();
    let pill = head.find("${pill}").unwrap();
    assert!(links < origin && origin < pill);
    for other in ["function hubPanelHeadHtml", "function sessPanelHeadHtml"] {
        let f = UI_HTML.split(other).nth(1).unwrap();
        assert!(!f[..f.find("\n\u{7d}\n").unwrap()].contains("ghHeadLinksHtml"));
    }
    let bar = UI_HTML.split("function termBarHtml").nth(1).unwrap();
    let bar = &bar[..bar.find("\n\u{7d}\n").unwrap()];
    assert!(!bar.contains("ghHeadLinksHtml") && !bar.contains("ghBarLinksHtml"));
    assert!(UI_HTML.contains("termBarHtml(s, reviewTerm, false)"));
    assert!(!UI_HTML.contains("ghBarLinksHtml"));
    assert!(!UI_HTML.contains("tp-bar-link"));
}

#[test]
fn the_panel_head_does_not_show_the_task_record_id() {
    let head = UI_HTML.split("function panelHeadHtml").nth(1).unwrap();
    let head = &head[..head.find("\n\u{7d}\n").unwrap()];
    assert!(!head.contains("tp-key"));
    assert!(!head.contains("task.id"));
}
