use std::collections::BTreeSet;

use crate::board::view::Lines;
use crate::infra::http::{self, Request};

use super::assets::{UI_HTML, vendor_asset};
use super::auth::{is_own_origin, page_rings, refuse};
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
fn old_addresses_are_read_through_nav_legacy_before_the_page_parses_its_own() {
    // `/review`, `view=sessions`, `#gate/<id>` and `#session/<id>` land where `legacyNav` says: the address
    // is parsed through it, and a gate it names is opened once the work document is in.
    for piece in [
        "function legacyNav",
        "const legacy = legacyNav(loc, multiBoard);",
        "pendingGate = legacy?.pendingGate || null;",
        "function openPendingGate",
        "if (pendingGate && view === 'work') openPendingGate();",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    let at = |piece: &str| UI_HTML.find(piece).unwrap();
    assert!(at("function legacyNav") < at("function parseUrl"));
    assert!(at("function parseUrl") < at("setInterval(refresh, 2000)"));
    // The server still answers the old path with the page, so that the page can redirect it.
    assert!(UI_HTML.contains("path === '/review'"));
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
        // The card's button for the terminal tab outside the board.
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
fn the_page_shows_a_tasks_read_through_in_the_middle() {
    // The tabs and the read-through are one each: a second copy would be a second place to type into.
    for id in [
        "id=\"wk-mid-row\"",
        "id=\"wk-mid-lead\"",
        "id=\"wk-mid-tabs\"",
        "id=\"wk-term-pane\"",
        "id=\"task-read\"",
        "id=\"task-read-index\"",
        "id=\"task-read-body\"",
        "id=\"task-read-band\"",
        "id=\"task-read-content\"",
    ] {
        assert_eq!(UI_HTML.matches(id).count(), 1, "{id}");
    }
    // The row is always shown: its lead holds the sidebar's toggle, and only the tabs go away for what is not a task.
    assert!(!UI_HTML.contains("id=\"wk-mid-row\" hidden"));
    assert_eq!(UI_HTML.matches("id=\"rail-toggle\"").count(), 1);
    let (lead, toggle, tabs) = (
        UI_HTML.find("id=\"wk-mid-lead\"").unwrap(),
        UI_HTML.find("id=\"rail-toggle\"").unwrap(),
        UI_HTML.find("id=\"wk-mid-tabs\"").unwrap(),
    );
    assert!(
        lead < toggle && toggle < tabs,
        "the toggle goes in the lead, before the tabs"
    );
    assert!(UI_HTML.contains(".wk-mid-tabs[hidden] { display: none; }"));
    for function in [
        "function taskReadTabState(",
        "function taskReadTabsHtml(",
        "function taskReadGroups(",
        "function taskReaderHeadHtml(",
        "function taskReaderTabsHtml(",
        "function taskReadShown(",
        "function taskReadRefresh(",
        "function taskReadAsk(",
        "function renderTaskReadTabs(",
        "function renderTaskReadMiddle(",
        "function renderTaskReaderPanel(",
        "function prShown(",
        "function prDraftsPane(",
        "function prSaveSettled(",
        "function taskReadHand(",
        "function prRecOf(",
        "function markIndexCurrent(",
        "function scrollToCard(",
        "function keepReadingPlace(",
        "function syncReaderTerm(",
        "function showMidTabs(",
        "function drawReadMiddle(",
        "function drawReaderTerm(",
        "function readPaneGroups(",
        "function renderPrRead(",
        "function renderPrPanel(",
    ] {
        assert_eq!(UI_HTML.matches(function).count(), 1, "{function}");
    }
    // What a later change builds on.
    for piece in [
        "const OWN_HOOKS = { drafts: null }",
        "OWN_HOOKS.drafts = {",
        "data-drafts=\"off\"",
        "id=\"tp-read-drafts\"",
        "const READ_PANES = ['ai', 'ai-drafts']",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    // The scripts share one scope and run in order: the pure half after the diff's, the page's half after the diff's page half and
    // before the entry point, which draws the first screen.
    let at = |piece: &str| UI_HTML.find(piece).unwrap();
    assert!(at("function prParseDiff(") < at("function taskReadTabState("));
    assert!(at("function taskReadTabState(") < at("function prShown("));
    assert!(at("function prdGoto(") < at("function taskReadShown("));
    assert!(at("function taskReadShown(") < at("function setView("));
    // The drafts' hook is set after the view that declares it, and before the entry point that draws the first screen.
    assert!(at("const OWN_HOOKS = {") < at("OWN_HOOKS.drafts = {"));
    assert!(at("OWN_HOOKS.drafts = {") < at("function setView("));
    // The tabs are above the head of the middle, and the read-through is beside the terminal's pane, not inside it.
    assert!(at("id=\"wk-mid-row\"") < at("id=\"wk-term-head\""));
    assert!(at("id=\"wk-term-pane\"") < at("id=\"task-read\""));
    let pane = at("id=\"wk-term-pane\"");
    let read = at("id=\"task-read\"");
    assert!(!UI_HTML[pane..read].contains("</section>"));
    assert!(at(".prd-bar {") < at(".wk-mid-row {"));
    // The tabs control panels that name them back.
    for piece in [
        "id=\"wk-term-pane\" role=\"tabpanel\" aria-labelledby=\"wk-mid-tab-task\"",
        "id=\"task-read\" role=\"tabpanel\" aria-labelledby=\"wk-mid-tab-ai\"",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
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
fn a_session_opens_in_the_panel_and_the_sessions_tab_is_gone() {
    for piece in [
        "SESS_REF = 'session:'",
        "function boardOfSession",
        "mountSessionTerminal(",
        "function openSessionRef",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    for gone in [
        "term-overlay",
        "openTerminalOverlay",
        "closeTerminalOverlay",
        // The tab and everything that drew its list.
        "id=\"sessions-view\"",
        "id=\"tab-sessions\"",
        "id=\"sess-groups\"",
        "data-tab=\"sessions\"",
        "function sessionTree",
        "function sessionGroups",
        "function renderSessionsView",
        "function patchSessionList",
        "function showSessions",
        "sessionsFolded:",
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
fn the_session_actions_and_dialogs_stay() {
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
    // The notice is in the flow above whichever view is on, not inside one of them.
    let at = |piece: &str| UI_HTML.find(piece).unwrap();
    assert!(at("id=\"sess-notice\"") < at("<main>"));
}

#[test]
fn a_session_is_named_by_its_title_and_not_by_the_tabs() {
    for piece in [
        "function sessionTitle",
        "function hubTitle",
        "function sessionTip",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    // The tab's title is for the tooltip: it does not stand in for the task's.
    assert!(!UI_HTML.contains("if (s.title) return s.title"));
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
        "owned ? owned.id : SESS_REF + s.id",
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
fn the_hub_panel_can_start_and_link_sessions() {
    for piece in [
        "data-tp-hub=\"${act}\"",
        "addBtn('add-session'",
        "addBtn('add-parent-hub'",
        "case 'add-session': return openStartDialog();",
        "case 'add-parent-hub': return openHubKeyDialog();",
        "id=\"hubkey-dialog\"",
        "id=\"start-dialog\"",
        "id=\"link-dialog\"",
        "function proposeName",
        "function worktreeNameProblem",
        "function sessionPendingRows",
        "function openLinkDialog",
        "起動を依頼中",
        "function idleWorktreesHtml",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    // The + menu of the セッション tab is gone with it.
    for gone in [
        "id=\"sess-add\"",
        "id=\"sess-add-menu\"",
        "function renderAddMenu",
    ] {
        assert!(!UI_HTML.contains(gone), "{gone}");
    }
    // The script that draws the panel calls into the one that knows the pending rows, which
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
    let park_labels = page_table("const PARK_REASON_LABEL = {", "};");
    for reason in crate::task::PARK_REASONS {
        assert!(
            has_key(park_labels, reason),
            "PARK_REASON_LABEL has no park reason {reason}"
        );
    }
}

#[test]
fn the_page_parks_a_task_from_one_dialog_and_wires_it_without_inline_handlers() {
    for piece in [
        "id=\"park-dialog\"",
        "aria-labelledby=\"park-title\"",
        "data-park-task=",
        "data-park-off",
        "置くのをやめる",
        "function setPark",
        "function parkClick",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
    assert!(!UI_HTML.contains("onclick=\"openParkDialog"));
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
    // The sidebar keeps one count, the number of 新着: no 要対応 entry, no gate count, and no
    // 「あなたの確認待ち」 badge on a board's row.
    for gone in [
        "id=\"nav-review\"",
        "id=\"gate-count\"",
        "function renderGateCount",
        "mini-badge",
        "id=\"sessions-waiting\"",
    ] {
        assert!(!UI_HTML.contains(gone), "{gone}");
    }
    assert!(UI_HTML.contains("id=\"work-new-count\""));
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
        if matches!(action, "git" | "message") {
            "GET"
        } else {
            "POST"
        }
    }
    fn route(id: String, action: &str) -> Route {
        match action {
            "link" => Route::LinkSession(id),
            "git" => Route::SessionGit(id),
            "message" => Route::SessionMessage(id),
            "resume" => Route::Session(id, SessionAction::Resume),
            "restart" => Route::Session(id, SessionAction::Restart),
            "open" => Route::Session(id, SessionAction::Open),
            _ => Route::Session(id, SessionAction::CleanUp),
        }
    }
    for action in [
        "link", "git", "message", "resume", "restart", "open", "cleanup",
    ] {
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
    assert_eq!(parsed("POST", "/api/sessions/worker-x/message"), Ok(None));
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
fn the_task_read_routes_come_before_the_task_update() {
    assert_eq!(
        parsed("GET", "/api/tasks/t1/ai"),
        Ok(Some(Route::TaskRead("t1".into())))
    );
    assert_eq!(
        parsed("POST", "/api/tasks/t1/ai"),
        Ok(Some(Route::AskTaskRead("t1".into())))
    );
    // The id is the segment as written, never decoded.
    assert_eq!(
        parsed("POST", "/api/tasks/a%2Fb/ai"),
        Ok(Some(Route::AskTaskRead("a%2Fb".into())))
    );
    // A task named `ai` is still updated at its own address, and has no read at one segment.
    assert_eq!(
        parsed("POST", "/api/tasks/ai"),
        Ok(Some(Route::UpdateTask("ai".into())))
    );
    assert_eq!(parsed("GET", "/api/tasks/ai"), Ok(None));
    assert_eq!(parsed("GET", "/api/tasks/t1/ai/x"), Ok(None));
    // Other methods are no route.
    assert_eq!(parsed("PUT", "/api/tasks/t1/ai"), Ok(None));
}

#[test]
fn the_task_read_routes_are_resident_only() {
    for method in ["GET", "POST"] {
        let route = parsed(method, "/api/tasks/t1/ai").unwrap().unwrap();
        assert!(route.resident_only(), "{method}");
    }
}

#[test]
fn the_hand_over_route_comes_before_the_task_update_and_is_resident_only() {
    let route = parsed("POST", "/api/tasks/t1/hand").unwrap().unwrap();
    assert_eq!(route, Route::HandToWorker("t1".into()));
    assert!(route.resident_only());
    // The id is the segment as written, never decoded.
    assert_eq!(
        parsed("POST", "/api/tasks/a%2Fb/hand"),
        Ok(Some(Route::HandToWorker("a%2Fb".into())))
    );
    // A task named `hand` is still updated at its own address.
    assert_eq!(
        parsed("POST", "/api/tasks/hand"),
        Ok(Some(Route::UpdateTask("hand".into())))
    );
    assert_eq!(parsed("GET", "/api/tasks/t1/hand"), Ok(None));
}

#[test]
fn each_refusal_to_hand_the_drafts_over_has_its_status_and_why() {
    use super::handlers::hand_status;
    use crate::board::review::HandRefused as R;
    use crate::gate::Kind;
    for (why, status, because) in [
        (R::NoTask, 404, None),
        (R::Nothing, 400, None),
        (R::NoPr, 409, Some("no-pr")),
        (R::PrChanged, 409, Some("pr-changed")),
        (R::PrDone, 409, Some("pr-done")),
        (R::NoRecord, 409, Some("no-record")),
        (R::NoWorker, 409, Some("no-worker")),
        (R::NoGate, 409, Some("no-gate")),
        (R::GateWaiting(Kind::Plan), 409, Some("gate-waiting")),
        (R::GateChanged, 409, Some("gate-changed")),
        (R::Busy, 409, Some("busy")),
        (R::Unreadable("x".into()), 500, None),
        (R::Failed("x".into()), 500, None),
    ] {
        assert_eq!(hand_status(&why), (status, because), "{why:?}");
    }
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
        ("GET", "/api/sessions/x/message"),
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
        "renderWorkView",
        "renderTaskPanel",
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
    assert_eq!(names.len(), 8);
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
fn a_session_waiting_on_a_prompt_is_rung_for_and_opens_in_the_work_view() {
    assert!(
        !UI_HTML.contains("要対応レビュー"),
        "the queue is gone, and so is its name"
    );
    for piece in [
        // The merged state of every board carries the waits.
        "waits: parts.flatMap(p => tag(p.data.waits, p.slug))",
        // The notification's click opens the session in 「いまの仕事」, on its board.
        "function openWait(w)",
        "task: SESS_REF + w.session",
        "permissionLabel(asked)",
        // A board served alone counts them in its tab title, beside what waits on the person.
        "(state.waits || []).length",
        // A quiet wait is listed without a desktop notification.
        "w.quiet",
        // The page says on every request whether it rings, so the server rings only when it does not.
        "'X-Adjutant-Notify'",
        "notifyPageRings(",
    ] {
        assert!(UI_HTML.contains(piece), "the page lacks {piece}");
    }
    for gone in ["const waitRef =", "data-rv-open-wait", "const boardWaiting"] {
        assert!(!UI_HTML.contains(gone), "{gone}");
    }
}

#[test]
fn a_request_says_whether_its_page_rings_notifications() {
    let rings = |headers: &[(&str, &str)]| page_rings(&request("GET", "/api/boards", headers));
    assert!(rings(&[("X-Adjutant-Notify", "1")]));
    assert!(rings(&[("x-adjutant-notify", "1")]));
    assert!(!rings(&[("x-adjutant-notify", "0")]));
    assert!(!rings(&[]));
    assert!(!rings(&[("X-Adjutant-Notify", "yes")]));
}

#[test]
fn the_page_has_the_work_view_and_no_review_queue() {
    for piece in [
        // The sidebar entry, the three-column view and the middle terminal's host.
        "id=\"nav-work\"",
        "data-action=\"work\"",
        "id=\"work-view\"",
        "id=\"wk-term-host\"",
        // The page's own notification settings, and the end-of-turn notifications read from the document.
        "id=\"notify-dialog\"",
        "function notifyEndEvents",
        "id=\"rail-resize\"",
        // The sidebar is hidden from the toggle at the head of the middle, in this view only, and the choice is kept.
        "aria-controls=\"nav-rail\"",
        "data-action=\"rail-toggle\"",
        "body.view-work.rail-hidden #nav-rail { display: none; }",
        "body.view-work.rail-hidden { --rail-w: 0px; }",
        "prefs.railHidden = prefs.railHidden === true",
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
    assert_eq!(UI_HTML.matches("id=\"rail-toggle-count\"").count(), 1);
    // A board served alone has no list of boards to read the work from.
    assert!(UI_HTML.contains(".single-board #nav-work { display: none; }"));
    // One sidebar with a switch at its top: the boards, and the list of the work under way beside the
    // terminal's column, not inside it.
    for piece in [
        "id=\"side-tab-boards\"",
        "data-action=\"boards\"",
        "role=\"tablist\"",
        "aria-controls=\"wk-list\"",
        ".single-board .side-tabs { display: none; }",
    ] {
        assert!(UI_HTML.contains(piece), "the page lacks {piece}");
    }
    let at = |piece: &str| {
        UI_HTML
            .find(piece)
            .unwrap_or_else(|| panic!("the page lacks {piece}"))
    };
    assert!(
        at("id=\"nav-rail\"") < at("id=\"wk-groups\"")
            && at("id=\"wk-groups\"") < at("id=\"task-panel\"")
    );
    assert!(!UI_HTML.contains("rail-global"));
    // One width for both tabs: the sidebar's handle is the only one.
    assert!(!UI_HTML.contains("wk-list-resize"));
    // The panel has no terminal tab in this view.
    assert!(UI_HTML.contains("const term = view === 'work' ? ''"));
    // What 要対応 did is done in the panel: a gate with no task card is judged in a panel of its own,
    // an old link lands in the list, and 「処理したら次へ」 is a box in the 新着 header.
    for piece in [
        "function gateJudgeHtml",
        "function gateDockHtml",
        "function legacyNav",
        "function workNextNew",
        "data-wk-advance",
    ] {
        assert!(UI_HTML.contains(piece), "the page lacks {piece}");
    }
    for gone in [
        "id=\"review\"",
        "id=\"nav-review\"",
        "id=\"rv-judge\"",
        "function renderReview",
        "function goToQueue",
        "reviewTerm",
        "go({ view: 'review'",
        "setView('review')",
    ] {
        assert!(!UI_HTML.contains(gone), "{gone}");
    }
    // The address `/review` still opens the page, which sends it on.
    assert!(UI_HTML.contains("path === '/review'"));
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
fn reading_the_hub_s_title_is_a_post_that_a_hub_s_own_board_answers() {
    let route = parsed("POST", "/api/hub/title");
    assert!(matches!(route, Ok(Some(Route::ReadHubTitle))));
    assert!(matches!(parsed("GET", "/api/hub/title"), Ok(None)));
    // Like waking it: a board served alone is where it lives, not only the resident.
    assert!(!Route::<String>::ReadHubTitle.resident_only());
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
fn the_wake_button_is_in_the_hub_panel_and_beside_the_chip_in_the_work_view() {
    for piece in [
        "data-tp-hub=\"wake\"",
        "data-wk-wake=",
        "function hubWakeBlocked",
        "function redrawHubWake",
    ] {
        assert!(UI_HTML.contains(piece), "{piece}");
    }
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
    assert!(UI_HTML.contains("termBarHtml(own, workTerm, true)"));
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

#[test]
fn the_card_dialog_is_in_the_page_after_what_it_calls() {
    // 「拡大」 (card-dialog.js) calls what decide.js and task-panel.js define, and is wired by
    // the panel's own answer path, not by `bindDecide`.
    let at = |s: &str| UI_HTML.find(s).unwrap_or_else(|| panic!("missing: {s}"));
    assert!(UI_HTML.contains("id=\"card-dialog\""));
    assert!(UI_HTML.contains("data-card-dialog-close"));
    assert!(at("function decideAct") < at("function openCardDialog"));
    assert!(at("const tp = id =>") < at("function openCardDialog"));
    assert!(!include_str!("../../ui/card-dialog.js").contains("bindDecide("));
    // The whole task is drawn from what the panel draws it with, and the page it sits in is the index and the grid.
    for needed in [
        "function gatesOf",
        "function timelineHtml",
        "function decideHtml",
        "function detailsKvHtml",
        "function gateWaitHeadHtml",
    ] {
        assert!(at(needed) < at("function openCardDialog"), "{needed}");
    }
    // Called when the dialog is opened, not when the page loads, so it may come after.
    assert!(UI_HTML.contains("function lastMessageHtml"));
    assert!(UI_HTML.contains("class=\"card-dialog-index\""));
    assert!(UI_HTML.contains("class=\"card-dialog-update\""));
    assert!(UI_HTML.contains("function cardDialogTaskGroups"));
    // The head's button is the panel head's own, and carries no task id.
    let head = UI_HTML.split("function panelHeadHtml").nth(1).unwrap();
    let head = &head[..head.find("\n}\n").unwrap()];
    assert!(head.contains("data-expand-task"));
    assert!(!head.contains("task.id"));
    // The panel draws, then the dialog follows.
    let render = UI_HTML.split("function renderTaskPanel()").nth(1).unwrap();
    let render = &render[..render.find("\n}\n").unwrap()];
    assert!(render.contains("renderTaskPanelNow()") && render.contains("cardDialogFollow"));
}

// ── the drafts of a PR, and the terminal of its reader ──────────────

mod others_drafts {
    use super::super::resident::{route_resident, upgrade_resident};
    use super::*;
    use crate::board::Resident;
    use serde_json::{Value, json};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    fn resident(root: &std::path::Path) -> Resident {
        Resident {
            root: root.to_path_buf(),
            token: TOKEN.to_string(),
            port: PORT,
            boards: Default::default(),
            tmux: None,
            terminals: Default::default(),
            pr_poll: Default::default(),
            waits: Default::default(),
            work_cache: Default::default(),
        }
    }

    fn put_record(root: &std::path::Path) {
        let record = json!({
            "id": "acme/widget#7", "repo": "Acme/Widget", "number": 7,
            "url": "https://github.com/acme/widget/pull/7",
            "registered": true, "title": "t", "base": "main", "head": "f", "headSha": "h1",
            "prState": "open", "additions": 1, "deletions": 1, "changedFiles": 1,
            "requested": true, "requestedAt": "20261001T000000Z",
            "firstSeenAt": "20261001T000000Z", "readAt": "20261001T000000Z"
        });
        let dir = root.join("others");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("acme~widget~7.json"), record.to_string()).unwrap();
    }

    /// The status and JSON body of the answer to one request.
    fn ask(
        resident: &Resident,
        method: &str,
        id: Option<&str>,
        body: Option<&Value>,
    ) -> (u16, Value) {
        let mut req = request(method, "/api/others/drafts", &[]);
        if let Some(id) = id {
            req.query = vec![("id".to_string(), id.to_string())];
        }
        if let Some(body) = body {
            req.body = body.to_string().into_bytes();
        }
        let mut out = Vec::new();
        route_resident(resident, &req, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let status = text.split(' ').nth(1).unwrap().parse().unwrap();
        let body = text.split_once("\r\n\r\n").unwrap().1;
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }

    #[test]
    fn the_resident_serves_xterm_at_its_root_for_the_terminal_of_a_reader() {
        let root = tempfile::tempdir().unwrap();
        let r = resident(root.path());
        for (path, kind) in [
            ("/vendor/xterm.js", "text/javascript"),
            ("/vendor/xterm.css", "text/css"),
        ] {
            let mut out = Vec::new();
            route_resident(&r, &request("GET", path, &[]), &mut out).unwrap();
            let text = String::from_utf8(out).unwrap();
            assert!(text.starts_with("HTTP/1.1 200"), "{path}: {text:.80}");
            assert!(text.contains(kind), "{path}");
            assert!(text.len() > 1000, "{path}");
        }
        let mut out = Vec::new();
        route_resident(&r, &request("GET", "/vendor/other.js", &[]), &mut out).unwrap();
        assert!(String::from_utf8(out).unwrap().starts_with("HTTP/1.1 404"));
    }

    fn drafts() -> Value {
        json!({
            "version": 1, "fromSha": null,
            "drafts": [{"key": "r1", "source": "reader", "path": "src/a.rs", "line": 4,
                        "side": "right", "body": "why not a match?"}],
            "overall": {"body": "looks fine"}
        })
    }

    #[test]
    fn drafts_are_read_back_as_saved_and_404_before_the_first_save() {
        let root = tempfile::tempdir().unwrap();
        put_record(root.path());
        let r = resident(root.path());
        let (status, body) = ask(&r, "GET", Some("acme/widget#7"), None);
        assert_eq!((status, body["error"].as_str()), (404, Some("no drafts")));

        let (status, saved) = ask(
            &r,
            "POST",
            None,
            Some(&json!({"id": "acme/widget#7", "drafts": drafts()})),
        );
        assert_eq!(status, 200, "{saved}");
        assert_eq!(saved["id"], "acme/widget#7");
        assert!(saved["savedAt"].is_string());
        assert_eq!(saved["drafts"][0]["body"], "why not a match?");

        let (status, read) = ask(&r, "GET", Some("acme/widget#7"), None);
        assert_eq!((status, &read), (200, &saved));
    }

    #[test]
    fn a_get_for_a_bad_id_is_400() {
        let root = tempfile::tempdir().unwrap();
        let r = resident(root.path());
        for id in [None, Some("widget#7"), Some("not a pr")] {
            assert_eq!(ask(&r, "GET", id, None).0, 400, "{id:?}");
        }
    }

    #[test]
    fn a_save_that_is_not_valid_is_400_with_every_problem() {
        let root = tempfile::tempdir().unwrap();
        put_record(root.path());
        let r = resident(root.path());
        let mut bad = drafts();
        bad["drafts"][0]["line"] = json!(0);
        bad["drafts"][0]["key"] = json!("a b");
        let (status, body) = ask(
            &r,
            "POST",
            None,
            Some(&json!({"id": "acme/widget#7", "drafts": bad})),
        );
        assert_eq!(status, 400);
        assert_eq!(body["error"], "the drafts are not valid");
        assert_eq!(body["problems"].as_array().unwrap().len(), 2, "{body}");
        // Nothing was written.
        assert_eq!(ask(&r, "GET", Some("acme/widget#7"), None).0, 404);
    }

    #[test]
    fn what_a_get_answered_is_not_accepted_back_as_the_drafts() {
        let root = tempfile::tempdir().unwrap();
        put_record(root.path());
        let r = resident(root.path());
        let post = |drafts: &Value| {
            ask(
                &r,
                "POST",
                None,
                Some(&json!({"id": "acme/widget#7", "drafts": drafts})),
            )
        };
        let (_, saved) = post(&drafts());
        // `id` and `savedAt` are the server's, so echoing them is an unknown key.
        let (status, body) = post(&saved);
        assert_eq!(status, 400);
        assert!(
            body["problems"][0]
                .as_str()
                .unwrap()
                .contains("unknown field")
        );
    }

    #[test]
    fn a_save_needs_an_id_an_object_and_a_record() {
        let root = tempfile::tempdir().unwrap();
        let r = resident(root.path());
        let cases = [
            (json!({"drafts": drafts()}), 400),
            (json!({"id": "nope", "drafts": drafts()}), 400),
            (json!({"id": "acme/widget#7"}), 400),
            (json!({"id": "acme/widget#7", "drafts": []}), 400),
            // A PR adj has no record of.
            (json!({"id": "acme/widget#7", "drafts": drafts()}), 404),
        ];
        for (body, want) in cases {
            assert_eq!(ask(&r, "POST", None, Some(&body)).0, want, "{body}");
        }
        let mut req = request("POST", "/api/others/drafts", &[]);
        req.body = b"not json".to_vec();
        let mut out = Vec::new();
        route_resident(&r, &req, &mut out).unwrap();
        assert!(String::from_utf8(out).unwrap().starts_with("HTTP/1.1 400"));
    }

    /// `POST /api/others/pending` with `body`. Only refusals are asked for here: an answer that
    /// would reach GitHub would run the real `gh`.
    fn push(resident: &Resident, body: &Value) -> (u16, Value) {
        let mut req = request("POST", "/api/others/pending", &[]);
        req.body = body.to_string().into_bytes();
        let mut out = Vec::new();
        route_resident(resident, &req, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let status = text.split(' ').nth(1).unwrap().parse().unwrap();
        let body = text.split_once("\r\n\r\n").unwrap().1;
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }

    #[test]
    fn putting_drafts_into_a_pending_review_is_refused_before_any_call_to_github() {
        let root = tempfile::tempdir().unwrap();
        let r = resident(root.path());
        // No id, a bad id.
        assert_eq!(push(&r, &json!({})).0, 400);
        assert_eq!(push(&r, &json!({"id": "nope"})).0, 400);
        // A PR adj has no record of.
        assert_eq!(push(&r, &json!({"id": "acme/widget#7"})).0, 404);

        put_record(root.path());
        // A record with no drafts.
        let (status, body) = push(&r, &json!({"id": "acme/widget#7"}));
        assert_eq!(
            (status, body["error"].as_str()),
            (404, Some("there are no drafts to send"))
        );

        // Drafts with nothing that can be sent: a comment on no file, and a blank overall comment.
        let saved = json!({
            "version": 1, "fromSha": null,
            "drafts": [{"key": "p1", "source": "person", "side": "right", "body": "on the PR"}],
            "overall": {"body": "  "}
        });
        assert_eq!(
            ask(
                &r,
                "POST",
                None,
                Some(&json!({"id": "acme/widget#7", "drafts": saved}))
            )
            .0,
            200
        );
        let (status, body) = push(&r, &json!({"id": "acme/widget#7"}));
        assert_eq!(status, 400);
        assert_eq!(body["nothing"], true, "{body}");
    }

    #[test]
    fn stopping_the_code_review_hub_with_none_running_says_so() {
        let sandbox = crate::testing::Sandbox::empty();
        let r = resident(&sandbox.state());
        let req = request("POST", "/api/others/hub/stop", &[]);
        let mut out = Vec::new();
        route_resident(&r, &req, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.split(' ').nth(1), Some("200"), "{text}");
        let body: Value = serde_json::from_str(text.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body, json!({"stopped": true, "wasRunning": false}));
    }

    /// `POST /api/others/reader/close` with `body`.
    fn close(resident: &Resident, body: &Value) -> (u16, Value) {
        let mut req = request("POST", "/api/others/reader/close", &[]);
        req.body = body.to_string().into_bytes();
        let mut out = Vec::new();
        route_resident(resident, &req, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let status = text.split(' ').nth(1).unwrap().parse().unwrap();
        let body = text.split_once("\r\n\r\n").unwrap().1;
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }

    #[test]
    fn closing_a_reader_answers_by_what_it_came_to() {
        let root = tempfile::tempdir().unwrap();
        let r = resident(root.path());
        assert_eq!(close(&r, &json!({})).0, 400);
        assert_eq!(close(&r, &json!({"id": "nope"})).0, 400);
        // A PR adj has no record of.
        assert_eq!(close(&r, &json!({"id": "acme/widget#7"})).0, 404);

        // A record with no read, so no reader.
        put_record(root.path());
        let (status, body) = close(&r, &json!({"id": "acme/widget#7"}));
        assert_eq!((status, body["why"].as_str()), (409, Some("no-reader")));
        assert!(body["error"].is_string());

        // A reader whose process is gone (a child that has been waited for) is closed and the read
        // stays; a running one would fail.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let gone = child.id();
        child.wait().unwrap();
        let path = root.path().join("others/acme~widget~7.json");
        let mut record: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        record["ai"] = json!({"status": "ready", "sessionId": "sid-a", "pid": gone,
                              "psStarted": "Thu Oct  1 00:00:02 2026"});
        std::fs::write(&path, record.to_string()).unwrap();
        let (status, body) = close(&r, &json!({"id": "acme/widget#7"}));
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["record"]["ai"]["status"], "ready");
        assert!(body["record"]["ai"]["readerClosedAt"].is_string());
        assert!(body["warning"].is_null());
        // Closed already: nothing open to close.
        assert_eq!(close(&r, &json!({"id": "acme/widget#7"})).0, 409);
    }

    /// Send a handshake to `upgrade_resident` over a real socket and say what came back.
    fn upgrade(resident: &Resident, path: &str, id: Option<&str>) -> Vec<u8> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        let mut req = request("GET", path, &upgrade_headers(None));
        if let Some(id) = id {
            req.query = vec![("id".to_string(), id.to_string())];
        }
        let reader = std::io::BufReader::new(server.try_clone().unwrap());
        upgrade_resident(resident, &req, server, reader).unwrap();
        client.flush().unwrap();
        let mut got = Vec::new();
        client.read_to_end(&mut got).unwrap();
        got
    }

    #[test]
    fn the_reader_terminal_route_refuses_a_bad_id_before_the_handshake() {
        let root = tempfile::tempdir().unwrap();
        let r = resident(root.path());
        for id in [None, Some("not a pr")] {
            let got = String::from_utf8(upgrade(&r, "/api/others/terminal", id)).unwrap();
            assert!(got.starts_with("HTTP/1.1 400"), "{id:?}: {got}");
        }
    }

    #[test]
    fn the_reader_terminal_route_says_why_there_is_no_window_as_a_close_code() {
        let root = tempfile::tempdir().unwrap();
        put_record(root.path());
        let mut r = resident(root.path());
        r.tmux = Some((3, 4));
        // A record that nobody asked a read of has no reader: the handshake is accepted, and the
        // socket then closes with 4404 and the reason.
        let got = upgrade(&r, "/api/others/terminal", Some("acme/widget#7"));
        let at = got.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let frame = &got[at..];
        assert_eq!(frame[0], 0x88);
        assert_eq!(&frame[2..4], &4404u16.to_be_bytes());
        assert_eq!(&frame[4..], b"no reader window for this pull request");
    }
}
