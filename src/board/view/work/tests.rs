use super::*;
use crate::board::view::parents::{ParentChild, Progress};
use crate::board::view::{OwnerHub, with_records};
use serde_json::json;

const REPO: &str = "acme-widget";
const FEATURE: &str = "acme-widget-wid-957";

fn address(slug: &str, nwo: &str, hub: Option<&str>) -> Address {
    Address {
        slug: slug.to_string(),
        main: "/m".to_string(),
        nwo: nwo.to_string(),
        hub: hub.map(str::to_string),
    }
}

fn session(
    id: &str,
    kind: &str,
    hub: Option<&str>,
    key: Option<&str>,
    task: Option<&str>,
) -> Session {
    serde_json::from_value(json!({
        "id": id,
        "kind": kind,
        "agent": "claude",
        "terminal": {"backend": "tmux"},
        "hub": hub,
        "key": key,
        "worktree": format!("/w/{id}"),
        "task": task,
        "present": true,
        "stale": false,
    }))
    .unwrap()
}

fn hub(id: &str, slug: &str, parent: bool) -> RepoHub {
    serde_json::from_value(json!({
        "id": id,
        "parent": parent,
        "key": null,
        "name": id,
        "slug": slug,
        "state": {"present": true, "stale": false, "pid": null, "startedAt": null},
        "inboxCount": 0,
    }))
    .unwrap()
}

fn card(id: &str, title: &str) -> TaskCard {
    let mut t = task::Task::new(
        id.to_string(),
        task::Kind::Start,
        title.to_string(),
        task::DoneWhen::Pr,
        "20261001T000000Z",
    );
    t.status = task::Status::Dispatched;
    with_records(vec![t], Vec::new(), Vec::new()).remove(0)
}

fn carried(slug: &str) -> Carried {
    Carried {
        slug: slug.to_string(),
        sessions: Vec::new(),
        hubs: vec![hub("hub", REPO, false), hub("hub-wid-957", FEATURE, true)],
        tasks: Vec::new(),
        hub_tasks: Vec::new(),
        parents: Vec::new(),
        gates: Vec::new(),
        hub_gates: Vec::new(),
    }
}

fn gate_card(id: &str, task: Option<&str>, wait: bool) -> GateCard {
    GateCard::of(
        serde_json::from_value(json!({
            "id": id,
            "kind": "plan",
            "worktree": "/w/x",
            "task": task,
            "title": format!("Gate {id}"),
            "wait": wait,
            "openedAt": "20261001T000100Z",
        }))
        .unwrap(),
    )
}

#[test]
fn the_repository_board_answers_for_its_repository_else_each_parent_task_board() {
    let all = [
        address("zed-app", "zed/app", None),
        address("acme-widget-wid-957", "acme/widget", Some("wid-957")),
        address("acme-widget", "acme/widget", None),
        address("acme-gadget-a", "acme/gadget", Some("a")),
        address("acme-gadget-b", "acme/gadget", Some("b")),
    ];
    assert_eq!(
        carriers(&all),
        [
            (
                "acme/gadget".to_string(),
                vec!["acme-gadget-a".to_string(), "acme-gadget-b".to_string()]
            ),
            ("acme/widget".to_string(), vec!["acme-widget".to_string()]),
            ("zed/app".to_string(), vec!["zed-app".to_string()]),
        ]
    );
}

#[test]
fn a_finished_parent_task_board_is_not_a_carrier() {
    let all = vec![
        address("acme-widget-wid-957", "acme/widget", Some("wid-957")),
        address("acme-widget-wid-958", "acme/widget", Some("wid-958")),
    ];
    let live = unfinished(all, &["acme-widget-wid-957".to_string()]);
    assert_eq!(
        carriers(&live),
        [(
            "acme/widget".to_string(),
            vec!["acme-widget-wid-958".to_string()]
        )]
    );
}

#[test]
fn a_worker_is_joined_to_its_task_in_the_hub_that_owns_it() {
    let mut c = carried(REPO);
    c.sessions = vec![
        session("hub", "hub", None, None, None),
        session("hub-wid-957", "hub", None, Some("wid-957"), None),
        session("worker-own", "worker", Some("hub"), None, Some("1")),
        session(
            "worker-feature",
            "worker",
            Some("hub-wid-957"),
            None,
            Some("1"),
        ),
        session("worker-bare", "worker", Some("hub"), None, None),
        session("worker-lost", "worker", Some("hub"), None, Some("404")),
    ];
    c.tasks = vec![card("1", "Own task")];
    let mut other = card("1", "Feature task");
    other.owner_hub = Some(OwnerHub {
        slug: FEATURE.to_string(),
        key: None,
        human_col: None,
    });
    c.hub_tasks = vec![other];
    let repo = repo_of("acme/widget".to_string(), vec![c]);
    let seen: Vec<(&str, &str, Option<&str>)> = repo
        .rows
        .iter()
        .map(|r| {
            (
                r.session.id.as_str(),
                r.board.as_str(),
                r.task.as_ref().map(|t| t.title.as_str()),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            ("hub", REPO, None),
            ("worker-own", REPO, Some("Own task")),
            ("worker-feature", FEATURE, Some("Feature task")),
            ("worker-bare", REPO, None),
            ("worker-lost", REPO, None),
        ]
    );
    // A parent-task hub is no row, and its session is kept for its parent's terminal.
    assert_eq!(repo.hub_sessions.len(), 1);
    assert_eq!(repo.hub_sessions[0].board, FEATURE);
    assert_eq!(repo.hub_sessions[0].session.id, "hub-wid-957");
    assert_eq!(repo.carrier, REPO);
}

#[test]
fn a_parent_task_hub_is_told_by_the_hubs_list_when_its_session_has_no_key() {
    let mut c = carried(REPO);
    // A hub started before its record carried the key: `parent` is all that says what it is.
    c.sessions = vec![session("hub-wid-957", "hub", None, None, None)];
    let repo = repo_of("acme/widget".to_string(), vec![c]);
    assert!(repo.rows.is_empty());
    assert_eq!(repo.hub_sessions.len(), 1);
    assert_eq!(repo.hub_sessions[0].board, FEATURE);
}

#[test]
fn what_two_carriers_both_list_is_listed_once() {
    let mut a = carried("acme-widget-wid-957");
    a.sessions = vec![session("worker-x", "worker", Some("hub"), None, None)];
    let mut b = carried("acme-widget-wid-958");
    b.sessions = vec![
        session("worker-x", "worker", Some("hub"), None, None),
        session("worker-y", "worker", Some("hub"), None, None),
    ];
    let repo = repo_of("acme/widget".to_string(), vec![a, b]);
    let ids: Vec<&str> = repo.rows.iter().map(|r| r.session.id.as_str()).collect();
    assert_eq!(ids, ["worker-x", "worker-y"]);
    assert_eq!(repo.hubs.len(), 2);
    assert_eq!(repo.carrier, "acme-widget-wid-957");
}

fn group(key: &str, children: usize) -> ParentGroup {
    ParentGroup {
        key: key.to_string(),
        url: String::new(),
        number: None,
        title: None,
        children: (0..children)
            .map(|i| ParentChild {
                hub: REPO.to_string(),
                id: i.to_string(),
                title: format!("Child {i}"),
                merged: false,
                progress: Progress::NotStarted,
                pr_turn_at: None,
                branch: None,
                base: None,
                on: None,
                on_hub: None,
            })
            .collect(),
        merged: 0,
        total: children,
        stacked: false,
        hub: REPO.to_string(),
    }
}

#[test]
fn a_parent_two_carriers_know_is_the_one_that_has_more_of_its_children() {
    let mut a = carried("acme-widget-wid-957");
    a.parents = vec![group("acme/widget#9", 1), group("acme/widget#2", 1)];
    let mut b = carried("acme-widget-wid-958");
    b.parents = vec![group("acme/widget#9", 3)];
    let repo = repo_of("acme/widget".to_string(), vec![a, b]);
    let seen: Vec<(&str, usize)> = repo
        .parents
        .iter()
        .map(|p| (p.key.as_str(), p.children.len()))
        .collect();
    assert_eq!(seen, [("acme/widget#2", 1), ("acme/widget#9", 3)]);
}

#[test]
fn a_repository_no_board_of_answers_says_so() {
    let repo = repo_of("acme/widget".to_string(), Vec::new());
    assert!(repo.error.is_some());
    assert!(repo.rows.is_empty());
}

#[test]
fn open_gates_are_grouped_by_the_task_they_name_and_the_ones_that_only_record_are_left_out() {
    let mut c = carried(REPO);
    let mut waiting = card("1", "Waiting on a PR");
    waiting.waits_on_person = true;
    waiting.task.pr_turn_at = Some("20261001T000200Z".to_string());
    waiting.task.gate_answered_at = Some("20261001T000300Z".to_string());
    c.tasks = vec![waiting, card("2", "Quiet task"), card("3", "Gate only")];
    c.gates = vec![
        gate_card("g1", Some("1"), true),
        gate_card("g2", Some("1"), true),
        gate_card("g3", Some("3"), true),
        gate_card("g4", None, true),
        gate_card("g5", None, true),
        gate_card("g6", Some("2"), false),
    ];
    let repo = repo_of("acme/widget".to_string(), vec![c]);
    let shape: Vec<(Option<&str>, Vec<&str>)> = repo
        .turns
        .iter()
        .map(|t| {
            (
                t.task.as_ref().map(|t| t.id.as_str()),
                t.gates.iter().map(|g| g.id.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            (Some("1"), vec!["g1", "g2"]),
            (Some("3"), vec!["g3"]),
            (None, vec!["g4", "g5"]),
        ]
    );
    assert!(repo.turns.iter().all(|t| t.board == REPO));
    let first = &repo.turns[0];
    let task = first.task.as_ref().unwrap();
    assert_eq!(task.pr_turn_at.as_deref(), Some("20261001T000200Z"));
    assert_eq!(task.gate_answered_at.as_deref(), Some("20261001T000300Z"));
    assert_eq!(first.gates[0].slug, REPO);
    assert_eq!(first.gates[0].opened_at, "20261001T000100Z");
    assert_eq!(repo.turns[1].task.as_ref().unwrap().title, "Gate only");
    assert_eq!(repo.turns[1].gates[0].task.as_deref(), Some("3"));
}

#[test]
fn a_task_whose_pr_waits_on_the_person_is_a_turn_even_with_no_session_or_gate() {
    let mut c = carried(REPO);
    let mut own = card("1", "Own");
    own.waits_on_person = true;
    let mut feature = card("1", "Feature");
    feature.waits_on_person = true;
    feature.owner_hub = Some(OwnerHub {
        slug: FEATURE.to_string(),
        key: None,
        human_col: None,
    });
    c.tasks = vec![own, card("2", "Not waiting")];
    c.hub_tasks = vec![feature];
    let repo = repo_of("acme/widget".to_string(), vec![c.clone(), c]);
    let seen: Vec<(&str, &str)> = repo
        .turns
        .iter()
        .map(|t| (t.board.as_str(), t.task.as_ref().unwrap().title.as_str()))
        .collect();
    // Two carriers that both list it do not make it twice.
    assert_eq!(seen, [(REPO, "Own"), (FEATURE, "Feature")]);
    assert!(repo.turns.iter().all(|t| t.gates.is_empty()));
    let json = serde_json::to_value(&repo).unwrap();
    assert_eq!(json["turns"][0]["task"]["waitsOnPerson"], true);
    assert!(json["turns"][0]["task"].get("prTurnAt").is_none());
}

fn parked(card: &mut TaskCard, reason: &str) {
    card.task.parked = Some(task::Park {
        reason: reason.to_string(),
        text: Some("資料待ち".to_string()),
        since: "20261002T000000Z".to_string(),
        extra: serde_json::Map::new(),
    });
}

#[test]
fn a_parked_task_with_nothing_waiting_is_a_turn_and_carries_its_park_and_a_done_one_does_not() {
    let mut c = carried(REPO);
    let mut waiting = card("1", "Parked");
    parked(&mut waiting, "pdm");
    let mut finished = card("2", "Finished");
    parked(&mut finished, "pdm");
    finished.task.status = task::Status::Done;
    let mut blank = card("3", "Blank");
    parked(&mut blank, " ");
    c.tasks = vec![waiting, finished, blank, card("4", "Plain")];
    let repo = repo_of("acme/widget".to_string(), vec![c]);
    let titles: Vec<&str> = repo
        .turns
        .iter()
        .map(|t| t.task.as_ref().unwrap().title.as_str())
        .collect();
    assert_eq!(titles, ["Parked"]);
    assert!(repo.turns[0].gates.is_empty());
    let json = serde_json::to_value(&repo).unwrap();
    assert_eq!(json["turns"][0]["task"]["parked"]["reason"], "pdm");
    assert_eq!(json["turns"][0]["task"]["parked"]["text"], "資料待ち");
    assert_eq!(
        json["turns"][0]["task"]["parked"]["since"],
        "20261002T000000Z"
    );
}

#[test]
fn a_gate_of_a_parked_task_stays_in_that_tasks_turn() {
    let mut c = carried(REPO);
    let mut task_card = card("1", "Parked");
    parked(&mut task_card, "review");
    c.tasks = vec![task_card];
    c.gates = vec![gate_card("g1", Some("1"), true)];
    let repo = repo_of("acme/widget".to_string(), vec![c]);
    assert_eq!(repo.turns.len(), 1);
    assert_eq!(repo.turns[0].gates.len(), 1);
    assert!(repo.turns[0].task.as_ref().unwrap().parked.is_some());
}

#[test]
fn a_gate_of_a_parent_task_hub_is_a_turn_of_that_hubs_board_and_task() {
    let mut c = carried(REPO);
    let mut feature = card("1", "Feature task");
    feature.owner_hub = Some(OwnerHub {
        slug: FEATURE.to_string(),
        key: None,
        human_col: None,
    });
    c.tasks = vec![card("1", "Own task")];
    c.hub_tasks = vec![feature];
    c.gates = vec![gate_card("own", Some("1"), true)];
    c.hub_gates = vec![
        (FEATURE.to_string(), gate_card("theirs", Some("1"), true)),
        (FEATURE.to_string(), gate_card("recorded", Some("1"), false)),
    ];
    // The same id on another board is another gate.
    c.hub_gates
        .push((FEATURE.to_string(), gate_card("own", Some("1"), true)));
    let repo = repo_of("acme/widget".to_string(), vec![c]);
    let shape: Vec<(&str, &str, Vec<&str>)> = repo
        .turns
        .iter()
        .map(|t| {
            (
                t.board.as_str(),
                t.task.as_ref().unwrap().title.as_str(),
                t.gates.iter().map(|g| g.id.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            (REPO, "Own task", vec!["own"]),
            (FEATURE, "Feature task", vec!["theirs", "own"]),
        ]
    );
    assert_eq!(repo.turns[1].gates[0].slug, FEATURE);
}
