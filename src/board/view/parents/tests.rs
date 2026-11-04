use super::*;
use crate::board::view::{OwnerHub, with_records};

const SLUG: &str = "acme-widget";

fn task_of(id: &str, order: u32) -> task::Task {
    let mut t = task::Task::new(
        id.to_string(),
        task::Kind::Start,
        format!("Task {id}"),
        task::DoneWhen::Pr,
        "20261001T000000Z",
    );
    t.order = order;
    t
}

fn with_parent(id: &str, order: u32, parent: &str) -> task::Task {
    let mut t = task_of(id, order);
    t.parent = Some(parent.to_string());
    t
}

fn cards(tasks: Vec<task::Task>) -> Vec<TaskCard> {
    with_records(tasks, Vec::new(), Vec::new())
}

fn on_branch(mut t: task::Task, worktree: &str, base: Option<&str>) -> task::Task {
    t.worktree = Some(worktree.to_string());
    t.base = base.map(str::to_string);
    t
}

fn pr(state: &str, head: Option<&str>) -> task::PrStatus {
    task::PrStatus {
        state: state.to_string(),
        title: String::new(),
        review: "none".to_string(),
        ci: task::CheckCounts::default(),
        head: head.map(str::to_string),
    }
}

fn worktree(path: &str, branch: &str) -> Worktree {
    Worktree {
        path: path.to_string(),
        branch: Some(branch.to_string()),
    }
}

fn keys() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({"acme/widget": "ALPHA"})
        .as_object()
        .unwrap()
        .clone()
}

fn run(
    tasks: &mut [TaskCard],
    hub_tasks: &mut [TaskCard],
    linked: &[Worktree],
    tracker: &dyn Fn(&str) -> Option<TrackerParent>,
) -> Vec<ParentGroup> {
    run_with_hubs(tasks, hub_tasks, linked, tracker, &[])
}

fn run_with_hubs(
    tasks: &mut [TaskCard],
    hub_tasks: &mut [TaskCard],
    linked: &[Worktree],
    tracker: &dyn Fn(&str) -> Option<TrackerParent>,
    hub_parents: &[HubParent],
) -> Vec<ParentGroup> {
    attach(
        tasks,
        hub_tasks,
        &Inputs {
            slug: SLUG,
            issue_keys: &keys(),
            linked,
            parent_hubs: &["acme-widget-9".to_string()],
            hub_parents,
            tracker,
        },
    )
}

fn never(_: &str) -> Option<TrackerParent> {
    None
}

fn tracker_parent(number: u64, total: u32) -> TrackerParent {
    TrackerParent {
        url: format!("https://github.com/acme/widget/issues/{number}"),
        number,
        title: "From the tracker".to_string(),
        open: true,
        total,
        completed: 0,
    }
}

fn ids(group: &ParentGroup) -> Vec<&str> {
    group.children.iter().map(|c| c.id.as_str()).collect()
}

#[test]
fn the_tracker_beats_the_record() {
    let mut a = task_of("a", 1);
    a.issue_url = Some("https://github.com/acme/widget/issues/20".to_string());
    a.parent = Some("https://github.com/acme/widget/issues/99".to_string());
    let mut tasks = cards(vec![a]);
    let groups = run(&mut tasks, &mut [], &[], &|url| {
        url.ends_with("/issues/20").then(|| tracker_parent(549, 4))
    });
    let shown = tasks[0].parent_issue.as_ref().unwrap();
    assert_eq!(shown.source, ParentSource::Tracker);
    assert_eq!(shown.key, "acme/widget#549");
    assert_eq!(shown.number, Some(549));
    assert_eq!(shown.title.as_deref(), Some("From the tracker"));
    // The record's own value is not touched, only outranked.
    assert_eq!(
        tasks[0].task.parent.as_deref(),
        Some("https://github.com/acme/widget/issues/99")
    );
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].key, "acme/widget#549");
}

#[test]
fn the_record_is_used_when_the_tracker_has_none_or_was_never_asked() {
    let mut a = task_of("a", 1);
    a.issue_url = Some("https://github.com/acme/widget/issues/20".to_string());
    a.parent = Some("https://github.com/Acme/Widget/issues/549#note".to_string());
    let mut tasks = cards(vec![a]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    let shown = tasks[0].parent_issue.as_ref().unwrap();
    assert_eq!(shown.source, ParentSource::Record);
    assert_eq!(shown.key, "acme/widget#549");
    assert_eq!(shown.number, Some(549));
    assert_eq!(shown.title, None);
    assert_eq!(groups[0].total, 1);
    // A task with neither has no parent at all.
    let mut bare = cards(vec![task_of("b", 1)]);
    assert!(run(&mut bare, &mut [], &[], &never).is_empty());
    assert!(bare[0].parent_issue.is_none());
}

#[test]
fn a_key_becomes_the_issue_it_names_and_an_unknown_one_stays_as_written() {
    let mut tasks = cards(vec![
        with_parent("a", 1, "ALPHA-549"),
        with_parent("b", 2, "GAMMA-3"),
    ]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    let a = tasks[0].parent_issue.as_ref().unwrap();
    assert_eq!(a.url, "https://github.com/acme/widget/issues/549");
    assert_eq!((a.key.as_str(), a.number), ("acme/widget#549", Some(549)));
    let b = tasks[1].parent_issue.as_ref().unwrap();
    assert_eq!(
        (b.key.as_str(), b.url.as_str(), b.number),
        ("GAMMA-3", "GAMMA-3", None)
    );
    // And a key and a URL of one issue are one parent.
    let mut same = cards(vec![
        with_parent("a", 1, "ALPHA-549"),
        with_parent("b", 2, "https://github.com/acme/widget/issues/549"),
    ]);
    let groups_same = run(&mut same, &mut [], &[], &never);
    assert_eq!(groups_same.len(), 1);
    assert_eq!(groups_same[0].children.len(), 2);
    assert_eq!(groups.len(), 2);
}

#[test]
fn the_parents_are_sorted_by_key_and_count_the_merged_children() {
    let mut a = with_parent("a", 1, "https://github.com/acme/widget/issues/9");
    a.pr_status = Some(pr("merged", None));
    let mut b = with_parent("b", 2, "https://github.com/acme/widget/issues/9");
    b.pr_status = Some(pr("open", None));
    let c = with_parent("c", 3, "https://github.com/acme/widget/issues/2");
    let mut tasks = cards(vec![a, b, c]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    let keys: Vec<&str> = groups.iter().map(|g| g.key.as_str()).collect();
    assert_eq!(keys, ["acme/widget#2", "acme/widget#9"]);
    let nine = &groups[1];
    assert_eq!((nine.merged, nine.total, nine.stacked), (1, 2, false));
    assert_eq!(
        nine.children.iter().map(|c| c.merged).collect::<Vec<_>>(),
        [true, false]
    );
}

#[test]
fn the_total_is_the_trackers_when_a_child_brought_it() {
    let mut a = with_parent("a", 1, "https://github.com/acme/widget/issues/9");
    a.issue_url = Some("https://github.com/acme/widget/issues/21".to_string());
    let b = with_parent("b", 2, "https://github.com/acme/widget/issues/9");
    let mut tasks = cards(vec![a, b]);
    let groups = run(&mut tasks, &mut [], &[], &|url| {
        url.ends_with("/issues/21").then(|| tracker_parent(9, 7))
    });
    assert_eq!(groups.len(), 1);
    assert_eq!((groups[0].total, groups[0].children.len()), (7, 2));
    // The tracker's title and number are the group's, though b only had the record's word.
    assert_eq!(groups[0].title.as_deref(), Some("From the tracker"));
}

#[test]
fn children_stacked_on_each_others_branches_come_root_first() {
    let p = "https://github.com/acme/widget/issues/9";
    // Queue order is c, a, b; the stack is a <- b <- c.
    let a = on_branch(with_parent("a", 2, p), "/w/a", Some("main"));
    let b = on_branch(with_parent("b", 3, p), "/w/b", Some("origin/feature/a"));
    let c = on_branch(with_parent("c", 1, p), "/w/c", Some("feature/b"));
    let d = with_parent("d", 4, p);
    let linked = [
        worktree("/w/a", "feature/a"),
        worktree("/w/b", "feature/b"),
        worktree("/w/c", "feature/c"),
    ];
    let mut tasks = cards(vec![a, b, c, d]);
    let groups = run(&mut tasks, &mut [], &linked, &never);
    let g = &groups[0];
    assert!(g.stacked);
    assert_eq!(ids(g), ["a", "b", "c", "d"]);
    let on: Vec<Option<&str>> = g.children.iter().map(|c| c.on.as_deref()).collect();
    assert_eq!(on, [None, Some("a"), Some("b"), None]);
}

#[test]
fn a_base_has_to_be_a_siblings_branch_exactly() {
    let p = "https://github.com/acme/widget/issues/9";
    let a = on_branch(with_parent("a", 1, p), "/w/a", Some("main"));
    let b = on_branch(with_parent("b", 2, p), "/w/b", Some("feature/x"));
    let linked = [
        worktree("/w/a", "feature/x-2"),
        worktree("/w/b", "feature/b"),
    ];
    let mut tasks = cards(vec![a, b]);
    let g = &run(&mut tasks, &mut [], &linked, &never)[0];
    assert!(!g.stacked);
    assert!(g.children.iter().all(|c| c.on.is_none()));
    // Only one leading `origin/` is taken off.
    let a = on_branch(with_parent("a", 1, p), "/w/a", Some("feature/x"));
    let b = on_branch(
        with_parent("b", 2, p),
        "/w/b",
        Some("origin/origin/feature/x"),
    );
    let linked = [worktree("/w/a", "feature/x"), worktree("/w/b", "feature/b")];
    let mut tasks = cards(vec![a, b]);
    assert!(!run(&mut tasks, &mut [], &linked, &never)[0].stacked);
}

#[test]
fn a_broken_chain_goes_back_to_queue_order() {
    let p = "https://github.com/acme/widget/issues/9";
    // b is cut from a branch no sibling is on (a's worktree is gone and it has no PR head).
    let a = with_parent("a", 1, p);
    let b = on_branch(with_parent("b", 2, p), "/w/b", Some("feature/a"));
    let c = with_parent("c", 3, p);
    let mut tasks = cards(vec![c, b, a]);
    let g = &run(
        &mut tasks,
        &mut [],
        &[worktree("/w/b", "feature/b")],
        &never,
    )[0];
    assert!(!g.stacked);
    assert_eq!(ids(g), ["a", "b", "c"]);
}

#[test]
fn a_finished_child_keeps_its_place_in_the_stack_by_its_pull_requests_head() {
    let p = "https://github.com/acme/widget/issues/9";
    let mut a = with_parent("a", 1, p);
    a.status = task::Status::Done;
    a.pr_status = Some(pr("merged", Some("feature/a")));
    let b = on_branch(with_parent("b", 2, p), "/w/b", Some("feature/a"));
    let mut tasks = cards(vec![b, a]);
    let g = &run(
        &mut tasks,
        &mut [],
        &[worktree("/w/b", "feature/b")],
        &never,
    )[0];
    assert!(g.stacked);
    assert_eq!(ids(g), ["a", "b"]);
    assert_eq!(g.children[1].on.as_deref(), Some("a"));
    assert_eq!(g.merged, 1);
}

#[test]
fn a_cycle_of_bases_does_not_loop() {
    let p = "https://github.com/acme/widget/issues/9";
    let a = on_branch(with_parent("a", 1, p), "/w/a", Some("feature/b"));
    let b = on_branch(with_parent("b", 2, p), "/w/b", Some("feature/a"));
    let linked = [worktree("/w/a", "feature/a"), worktree("/w/b", "feature/b")];
    let mut tasks = cards(vec![a, b]);
    let g = &run(&mut tasks, &mut [], &linked, &never)[0];
    assert_eq!(g.children.len(), 2);
}

#[test]
fn one_id_in_two_hubs_is_two_children() {
    let p = "https://github.com/acme/widget/issues/9";
    let mut tasks = cards(vec![with_parent("1", 1, p)]);
    let mut hub_tasks = cards(vec![with_parent("1", 1, p)]);
    hub_tasks[0].owner_hub = Some(OwnerHub {
        slug: "acme-widget-9".to_string(),
        key: None,
        human_col: None,
    });
    let g = &run(&mut tasks, &mut hub_tasks, &[], &never)[0];
    let hubs: Vec<(&str, &str)> = g
        .children
        .iter()
        .map(|c| (c.hub.as_str(), c.id.as_str()))
        .collect();
    assert_eq!(hubs, [(SLUG, "1"), ("acme-widget-9", "1")]);
    assert_eq!(g.total, 2);
    assert!(hub_tasks[0].parent_issue.is_some());
}

#[test]
fn a_child_names_the_hub_of_the_sibling_it_is_on_when_two_hubs_share_an_id() {
    let p = "https://github.com/acme/widget/issues/9";
    // Both hubs have a child "1"; the one in the other hub is cut from this board's "1".
    let own = on_branch(with_parent("1", 1, p), "/w/own", Some("main"));
    let other = on_branch(with_parent("1", 2, p), "/w/other", Some("feature/own"));
    let mut tasks = cards(vec![own]);
    let mut hub_tasks = cards(vec![other]);
    hub_tasks[0].owner_hub = Some(OwnerHub {
        slug: "acme-widget-9".to_string(),
        key: None,
        human_col: None,
    });
    let linked = [
        worktree("/w/own", "feature/own"),
        worktree("/w/other", "feature/other"),
    ];
    let g = &run(&mut tasks, &mut hub_tasks, &linked, &never)[0];
    let kids: Vec<_> = g
        .children
        .iter()
        .map(|c| {
            (
                c.hub.as_str(),
                c.id.as_str(),
                c.on_hub.as_deref(),
                c.on.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        kids,
        [
            (SLUG, "1", None, None),
            ("acme-widget-9", "1", Some(SLUG), Some("1"))
        ]
    );
}

#[test]
fn the_shape_the_page_reads() {
    let mut tasks = cards(vec![with_parent("a", 1, "ALPHA-5")]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    let card = serde_json::to_value(&tasks[0]).unwrap();
    assert_eq!(
        card["parentIssue"],
        serde_json::json!({
            "key": "acme/widget#5",
            "url": "https://github.com/acme/widget/issues/5",
            "number": 5,
            "source": "record",
            "hub": SLUG,
            "recordKey": "acme/widget#5"
        })
    );
    assert_eq!(
        serde_json::to_value(&groups[0]).unwrap(),
        serde_json::json!({
            "key": "acme/widget#5",
            "url": "https://github.com/acme/widget/issues/5",
            "number": 5,
            "children": [{"hub": SLUG, "id": "a", "title": "Task a", "merged": false, "progress": "not-started"}],
            "merged": 0,
            "total": 1,
            "stacked": false,
            "hub": SLUG
        })
    );
}

#[test]
fn the_total_is_never_below_the_children_listed() {
    let mut a = with_parent("a", 1, "https://github.com/acme/widget/issues/9");
    a.issue_url = Some("https://github.com/acme/widget/issues/21".to_string());
    let b = with_parent("b", 2, "https://github.com/acme/widget/issues/9");
    let c = with_parent("c", 3, "https://github.com/acme/widget/issues/9");
    let mut tasks = cards(vec![a, b, c]);
    // The tracker counts 2 (and 0 when its summary was missing); the board holds 3.
    for counted in [2, 0] {
        let g = &run(&mut tasks, &mut [], &[], &|url| {
            url.ends_with("/issues/21")
                .then(|| tracker_parent(9, counted))
        })[0];
        assert_eq!(g.total, 3);
    }
}

fn stacked(id: &str, order: u32, base: &str, branch: &str) -> (task::Task, Worktree) {
    let path = format!("/w/{id}");
    let t = on_branch(
        with_parent(id, order, "https://github.com/acme/widget/issues/9"),
        &path,
        Some(base),
    );
    (t, worktree(&path, branch))
}

fn order_of(parts: Vec<(task::Task, Worktree)>) -> (Vec<String>, Vec<Option<String>>) {
    let (tasks, linked): (Vec<_>, Vec<_>) = parts.into_iter().unzip();
    let mut cards = cards(tasks);
    let g = run(&mut cards, &mut [], &linked, &never).remove(0);
    (
        g.children.iter().map(|c| c.id.clone()).collect(),
        g.children.iter().map(|c| c.on.clone()).collect(),
    )
}

#[test]
fn two_chains_come_one_after_the_other() {
    // Queue order interleaves them: a1, b1, a2, b2.
    let (ids, on) = order_of(vec![
        stacked("a1", 1, "main", "fa1"),
        stacked("b1", 2, "main", "fb1"),
        stacked("a2", 3, "fa1", "fa2"),
        stacked("b2", 4, "fb1", "fb2"),
    ]);
    assert_eq!(ids, ["a1", "a2", "b1", "b2"]);
    assert_eq!(
        on,
        [None, Some("a1".to_string()), None, Some("b1".to_string())]
    );
}

#[test]
fn children_on_one_base_come_in_queue_order_after_it() {
    let (ids, _) = order_of(vec![
        stacked("r", 1, "main", "fr"),
        stacked("y", 3, "fr", "fy"),
        stacked("x", 2, "fr", "fx"),
        stacked("z", 4, "fx", "fz"),
    ]);
    // Depth first: r, then its first child x with what is cut from x, then y.
    assert_eq!(ids, ["r", "x", "z", "y"]);
}

#[test]
fn the_record_key_is_sent_beside_a_trackers_parent_and_names_the_hub_of_the_card() {
    let mut a = with_parent("a", 1, "https://github.com/Acme/Widget/issues/99#x");
    a.issue_url = Some("https://github.com/acme/widget/issues/20".to_string());
    let mut b = with_parent("b", 2, "GAMMA-3");
    b.issue_url = Some("https://github.com/acme/widget/issues/21".to_string());
    let mut c = task_of("c", 3);
    c.issue_url = Some("https://github.com/acme/widget/issues/22".to_string());
    let mut tasks = cards(vec![a, b, c]);
    run(&mut tasks, &mut [], &[], &|_| Some(tracker_parent(549, 3)));
    let shown = |i: usize| tasks[i].parent_issue.clone().unwrap();
    // The record named another issue; its key is the server's own spelling of it.
    assert_eq!(shown(0).source, ParentSource::Tracker);
    assert_eq!(shown(0).record_key.as_deref(), Some("acme/widget#99"));
    // A key no repository is known for names no issue, and no record names none.
    assert_eq!(shown(1).record_key, None);
    assert_eq!(shown(2).record_key, None);
    assert!(
        tasks
            .iter()
            .all(|t| t.parent_issue.as_ref().unwrap().hub == SLUG)
    );
    let json = serde_json::to_value(shown(1)).unwrap();
    assert!(json.get("recordKey").is_none(), "{json}");
}

#[test]
fn a_child_is_as_far_as_its_status_and_pull_request_say() {
    let p = "https://github.com/acme/widget/issues/9";
    let mut merged = with_parent("merged", 1, p);
    merged.pr_status = Some(pr("merged", None));
    let mut done = with_parent("done", 2, p);
    done.status = task::Status::Done;
    let mut with_pr = with_parent("pr", 3, p);
    with_pr.pr = Some("https://github.com/acme/widget/pull/3".to_string());
    with_pr.status = task::Status::Dispatched;
    let mut status_pr = with_parent("status-pr", 4, p);
    status_pr.status = task::Status::Pr;
    let mut working = with_parent("working", 5, p);
    working.status = task::Status::Dispatched;
    let mut queued = with_parent("queued", 6, p);
    queued.status = task::Status::Queued;
    let mut cancelled = with_parent("cancelled", 7, p);
    cancelled.status = task::Status::Cancelled;
    let mut done_merged = with_parent("done-merged", 8, p);
    done_merged.status = task::Status::Done;
    done_merged.pr_status = Some(pr("merged", None));
    let mut cancelled_pr = with_parent("cancelled-pr", 9, p);
    cancelled_pr.status = task::Status::Cancelled;
    cancelled_pr.pr = Some("https://github.com/acme/widget/pull/4".to_string());
    let mut tasks = cards(vec![
        merged,
        done,
        with_pr,
        status_pr,
        working,
        queued,
        cancelled,
        done_merged,
        cancelled_pr,
    ]);
    let g = &run(&mut tasks, &mut [], &[], &never)[0];
    let seen: Vec<(&str, Progress)> = g
        .children
        .iter()
        .map(|c| (c.id.as_str(), c.progress))
        .collect();
    assert_eq!(
        seen,
        [
            ("merged", Progress::Merged),
            ("done", Progress::Done),
            ("pr", Progress::Pr),
            ("status-pr", Progress::Pr),
            ("working", Progress::Working),
            ("queued", Progress::NotStarted),
            ("cancelled", Progress::NotStarted),
            ("done-merged", Progress::Merged),
            ("cancelled-pr", Progress::NotStarted),
        ]
    );
    let shown = serde_json::to_value(g).unwrap();
    assert_eq!(shown["children"][2]["progress"], "pr");
    assert_eq!(shown["children"][5]["progress"], "not-started");
}

#[test]
fn a_child_carries_its_title_and_when_its_pull_requests_turn_changed() {
    let p = "https://github.com/acme/widget/issues/9";
    let mut merged = with_parent("merged", 1, p);
    merged.pr_status = Some(pr("merged", None));
    merged.pr_turn_at = Some("20261002T030405Z".to_string());
    let quiet = with_parent("quiet", 2, p);
    let mut tasks = cards(vec![merged, quiet]);
    let g = &run(&mut tasks, &mut [], &[], &never)[0];
    let shown = serde_json::to_value(g).unwrap();
    assert_eq!(shown["children"][0]["title"], "Task merged");
    assert_eq!(shown["children"][0]["progress"], "merged");
    assert_eq!(shown["children"][0]["prTurnAt"], "20261002T030405Z");
    assert_eq!(shown["children"][1]["title"], "Task quiet");
    assert!(shown["children"][1].get("prTurnAt").is_none());
}

#[test]
fn a_stacked_chain_names_its_branches_and_what_the_root_is_cut_from() {
    let (a, wa) = stacked("a", 1, "origin/main", "feature/a");
    let (b, wb) = stacked("b", 2, "feature/a", "feature/b");
    let mut tasks = cards(vec![b, a]);
    let g = &run(&mut tasks, &mut [], &[wa, wb], &never)[0];
    let seen: Vec<(&str, Option<&str>, Option<&str>)> = g
        .children
        .iter()
        .map(|c| (c.id.as_str(), c.branch.as_deref(), c.base.as_deref()))
        .collect();
    assert_eq!(
        seen,
        [
            ("a", Some("feature/a"), Some("main")),
            ("b", Some("feature/b"), Some("feature/a"))
        ]
    );
}

#[test]
fn the_parent_is_run_by_a_parent_task_hub_among_its_children_else_the_repository() {
    let p = "https://github.com/acme/widget/issues/9";
    let mut own = cards(vec![with_parent("1", 1, p)]);
    assert_eq!(run(&mut own, &mut [], &[], &never)[0].hub, SLUG);
    // Mixed: one child on the board, one in a parent-task hub.
    let mut tasks = cards(vec![with_parent("1", 1, p)]);
    let mut hub_tasks = cards(vec![with_parent("2", 2, p)]);
    hub_tasks[0].owner_hub = Some(OwnerHub {
        slug: "acme-widget-9".to_string(),
        key: None,
        human_col: None,
    });
    assert_eq!(
        run(&mut tasks, &mut hub_tasks, &[], &never)[0].hub,
        "acme-widget-9"
    );
    // A hub that is not a parent-task hub of the repository does not own the parent.
    let mut tasks = cards(vec![with_parent("1", 1, p)]);
    let mut hub_tasks = cards(vec![with_parent("2", 2, p)]);
    hub_tasks[0].owner_hub = Some(OwnerHub {
        slug: "acme-widget-other".to_string(),
        key: None,
        human_col: None,
    });
    assert_eq!(run(&mut tasks, &mut hub_tasks, &[], &never)[0].hub, SLUG);
}

fn hub_parent(key: &str, title: Option<&str>, total: Option<u32>) -> HubParent {
    HubParent {
        key: key.to_string(),
        slug: "acme-widget-ALPHA-549".to_string(),
        title: title.map(str::to_string),
        total,
    }
}

#[test]
fn a_group_the_tracker_never_described_takes_the_title_and_count_of_the_hub_for_it() {
    let mut a = task_of("a", 1);
    a.parent = Some("https://github.com/acme/widget/issues/549".to_string());
    let mut tasks = cards(vec![a]);
    let hubs = [hub_parent("acme/widget#549", Some("From the hub"), Some(7))];
    let groups = run_with_hubs(&mut tasks, &mut [], &[], &never, &hubs);
    assert_eq!(groups[0].title.as_deref(), Some("From the hub"));
    assert_eq!(groups[0].total, 7);
    assert_eq!(
        groups[0].title_hub.as_deref(),
        Some("acme-widget-ALPHA-549")
    );
    // Never fewer than the children listed, as for the tracker's count.
    let mut kids: Vec<_> = (0..3)
        .map(|i| {
            with_parent(
                &format!("k{i}"),
                i,
                "https://github.com/acme/widget/issues/549",
            )
        })
        .collect();
    let mut tasks = cards(std::mem::take(&mut kids));
    let hubs = [hub_parent("acme/widget#549", None, Some(2))];
    let groups = run_with_hubs(&mut tasks, &mut [], &[], &never, &hubs);
    assert_eq!(groups[0].total, 3);
    assert_eq!(groups[0].title, None);
}

#[test]
fn the_tracker_s_title_and_count_win_over_the_hub_s() {
    let mut a = task_of("a", 1);
    a.issue_url = Some("https://github.com/acme/widget/issues/20".to_string());
    let mut tasks = cards(vec![a]);
    let hubs = [hub_parent("acme/widget#549", Some("From the hub"), Some(7))];
    let groups = run_with_hubs(
        &mut tasks,
        &mut [],
        &[],
        &|url| url.ends_with("/issues/20").then(|| tracker_parent(549, 4)),
        &hubs,
    );
    assert_eq!(groups[0].title.as_deref(), Some("From the tracker"));
    assert_eq!(groups[0].total, 4);
    // The hub is still the one the page puts the title's state beside.
    assert_eq!(
        groups[0].title_hub.as_deref(),
        Some("acme-widget-ALPHA-549")
    );
}

#[test]
fn a_group_names_a_title_hub_only_when_the_issue_keys_match() {
    let mut a = task_of("a", 1);
    a.parent = Some("https://github.com/acme/widget/issues/549".to_string());
    let mut tasks = cards(vec![a]);
    let hubs = [hub_parent("acme/widget#550", Some("Another"), Some(7))];
    let groups = run_with_hubs(&mut tasks, &mut [], &[], &never, &hubs);
    assert_eq!(groups[0].title_hub, None);
    assert_eq!(groups[0].title, None);
    assert_eq!(groups[0].total, 1);
}

const PARENT: &str = "https://github.com/acme/widget/issues/9";

/// A child of `PARENT` that works on the issue numbered `issue`, coming after `after`.
fn sibling(id: &str, order: u32, issue: u32, after: &[&str]) -> task::Task {
    let mut t = with_parent(id, order, PARENT);
    t.issue_url = Some(format!("https://github.com/acme/widget/issues/{issue}"));
    t.after = after.iter().map(|a| a.to_string()).collect();
    t
}

fn after_of(group: &ParentGroup, id: &str) -> Vec<String> {
    group
        .children
        .iter()
        .find(|c| c.id == id)
        .unwrap()
        .after
        .clone()
}

#[test]
fn a_child_comes_after_the_sibling_whose_issue_url_it_names() {
    let api = sibling("api", 1, 21, &[]);
    let screen = sibling(
        "screen",
        2,
        22,
        &["https://github.com/acme/widget/issues/21"],
    );
    let mut tasks = cards(vec![api, screen]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    assert_eq!(after_of(&groups[0], "screen"), vec!["api"]);
    assert_eq!(after_of(&groups[0], "api"), Vec::<String>::new());
    let screen = groups[0]
        .children
        .iter()
        .find(|c| c.id == "screen")
        .unwrap();
    assert_eq!(screen.after_hub, vec![SLUG]);
    assert!(!groups[0].stacked, "an `after` is not a stack");
}

#[test]
fn a_key_names_the_sibling_it_resolves_to_and_a_url_in_another_case_matches() {
    let api = sibling("api", 1, 21, &[]);
    let screen = sibling("screen", 2, 22, &["ALPHA-21"]);
    let other = sibling(
        "other",
        3,
        23,
        &["https://GitHub.com/Acme/Widget/issues/21/"],
    );
    let mut tasks = cards(vec![api, screen, other]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    assert_eq!(after_of(&groups[0], "screen"), vec!["api"]);
    assert_eq!(after_of(&groups[0], "other"), vec!["api"]);
}

#[test]
fn a_key_the_config_does_not_resolve_matches_the_last_segment_of_a_trackers_url() {
    let mut api = with_parent("api", 1, "WEB-1");
    api.issue_url = Some("https://example.test/browse/WEB-7".to_string());
    let mut screen = with_parent("screen", 2, "WEB-1");
    screen.after = vec!["web-7".to_string()];
    let mut tasks = cards(vec![api, screen]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    assert_eq!(after_of(&groups[0], "screen"), vec!["api"]);

    // A query or a fragment on the sibling's URL is not part of the key.
    for tail in ["?view=all", "#comments", "/?a=b#c"] {
        let mut api = with_parent("api", 1, "WEB-1");
        api.issue_url = Some(format!("https://example.test/browse/WEB-7{tail}"));
        let mut screen = with_parent("screen", 2, "WEB-1");
        screen.after = vec!["WEB-7".to_string()];
        let mut tasks = cards(vec![api, screen]);
        let groups = run(&mut tasks, &mut [], &[], &never);
        assert_eq!(after_of(&groups[0], "screen"), vec!["api"], "{tail}");
    }
}

#[test]
fn an_entry_no_sibling_answers_to_is_dropped() {
    let api = sibling("api", 1, 21, &[]);
    let screen = sibling(
        "screen",
        2,
        22,
        &[
            "https://github.com/acme/widget/issues/404",
            "ALPHA-405",
            "not a task",
        ],
    );
    let mut tasks = cards(vec![api, screen]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    assert!(after_of(&groups[0], "screen").is_empty());
    assert_eq!(groups[0].children.len(), 2);
}

#[test]
fn a_task_of_another_parent_is_not_a_sibling() {
    let mut elsewhere = sibling("api", 1, 21, &[]);
    elsewhere.parent = Some("https://github.com/acme/widget/issues/10".to_string());
    let screen = sibling(
        "screen",
        2,
        22,
        &["https://github.com/acme/widget/issues/21"],
    );
    let mut tasks = cards(vec![elsewhere, screen]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    let nine = groups.iter().find(|g| g.url == PARENT).unwrap();
    assert!(after_of(nine, "screen").is_empty());
}

#[test]
fn a_child_does_not_come_after_itself() {
    let screen = sibling(
        "screen",
        1,
        22,
        &["https://github.com/acme/widget/issues/22"],
    );
    let mut tasks = cards(vec![screen]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    assert!(after_of(&groups[0], "screen").is_empty());
}

#[test]
fn a_cycle_of_afters_loses_one_edge_and_no_child() {
    let a = sibling("a", 1, 21, &["ALPHA-22"]);
    let b = sibling("b", 2, 22, &["ALPHA-23"]);
    let c = sibling("c", 3, 23, &["ALPHA-21"]);
    let mut tasks = cards(vec![a, b, c]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    assert_eq!(ids(&groups[0]), vec!["a", "b", "c"]);
    assert_eq!(after_of(&groups[0], "a"), vec!["b"]);
    assert_eq!(after_of(&groups[0], "b"), vec!["c"]);
    assert!(after_of(&groups[0], "c").is_empty());
}

#[test]
fn an_after_leaves_the_order_and_the_stack_as_they_were() {
    let (mut a, wa) = stacked("a", 1, "main", "feat-a");
    let (mut b, wb) = stacked("b", 2, "feat-a", "feat-b");
    a.issue_url = Some("https://github.com/acme/widget/issues/20".to_string());
    // Stacked on `a` and also after `api`, which comes last in the queue.
    b.after = vec!["ALPHA-21".to_string()];
    let api = sibling("api", 3, 21, &[]);
    let mut tasks = cards(vec![a, b, api]);
    let groups = run(&mut tasks, &mut [], &[wa, wb], &never);
    assert_eq!(ids(&groups[0]), vec!["a", "b", "api"]);
    assert!(groups[0].stacked);
    assert_eq!(groups[0].children[1].on.as_deref(), Some("a"));
    assert_eq!(groups[0].children[1].after, vec!["api"]);
}

#[test]
fn an_after_is_sent_only_when_it_is_not_empty() {
    let api = sibling("api", 1, 21, &[]);
    let screen = sibling("screen", 2, 22, &["ALPHA-21"]);
    let mut tasks = cards(vec![api, screen]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    let json = serde_json::to_value(&groups[0]).unwrap();
    assert!(json["children"][0].get("after").is_none());
    assert!(json["children"][0].get("afterHub").is_none());
    assert_eq!(json["children"][1]["after"], serde_json::json!(["api"]));
    assert_eq!(json["children"][1]["afterHub"], serde_json::json!([SLUG]));
}

#[test]
fn a_key_the_config_resolves_names_only_that_github_issue_and_not_a_trackers_url_with_the_same_key()
{
    // Queued first, so a looser match would pick it.
    let mut jira = with_parent("jira", 1, PARENT);
    jira.issue_url = Some("https://example.test/browse/ALPHA-21".to_string());
    let github = sibling("github", 2, 21, &[]);
    let screen = sibling("screen", 3, 22, &["ALPHA-21"]);
    let mut tasks = cards(vec![jira, github, screen]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    assert_eq!(after_of(&groups[0], "screen"), vec!["github"]);
}

#[test]
fn a_key_the_config_does_not_resolve_matches_a_linear_url_with_a_title() {
    let mut api = with_parent("api", 1, "XYZ-1");
    api.issue_url = Some("https://linear.app/acme/issue/XYZ-9/fix-the-thing".to_string());
    let mut screen = with_parent("screen", 2, "XYZ-1");
    screen.after = vec!["xyz-9".to_string()];
    let mut tasks = cards(vec![api, screen]);
    let groups = run(&mut tasks, &mut [], &[], &never);
    assert_eq!(after_of(&groups[0], "screen"), vec!["api"]);
}
