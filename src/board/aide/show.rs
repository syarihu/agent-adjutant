use std::path::Path;

use serde_json::{Map, Value};

use super::display::{AideDisplay, AidePart, PartRef, Topic, display_path, lock_path, read};
use crate::gate::Gate;
use crate::task::{self, Task};

/// How many earlier topics are kept, newest first.
pub const EARLIER_KEPT: usize = 5;
/// How many parts one topic keeps, newest first.
pub const PARTS_KEPT: usize = 20;

/// The key and label of the topic of the list of waits. The page names the same one for the
/// list it draws while the aide has shown nothing (`aideDisplayWithWaits` in
/// `src/ui/aide-display.js`), so the two stay one topic.
const WAITS_KEY: &str = "waits";
const WAITS_LABEL: &str = "判断待ちの一覧";

/// What the aide asks to have shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShowRequest {
    /// One gate, by the slug of its board and its id.
    Gate { board: String, id: String },
    /// Everything that waits on the person.
    Waits,
}

/// What `show` did.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub display: AideDisplay,
    /// Another topic replaced the one on show, and its parts went with it.
    pub cleared: bool,
    /// The file could not be read and was written anew.
    pub replaced_unreadable: bool,
}

/// Put one more part in the display area.
///
/// Showing a part of another topic clears the area and keeps the old topic by name; showing one
/// of the same topic adds it on top, and showing the same thing again moves it to the top. What
/// is shown is worked out before the lock is taken, so a request that is refused writes nothing,
/// not even the lock file.
///
/// A file that cannot be read is replaced: the area holds nothing that cannot be shown again, and
/// a wall of an error in front of the aide would be worse than an empty area.
pub fn show(root: &Path, request: ShowRequest) -> Result<Shown, String> {
    let (topic, kind, reference) = match request {
        ShowRequest::Waits => (Topic::new(WAITS_KEY, WAITS_LABEL), "waits", None),
        ShowRequest::Gate { board, id } => {
            let (topic, reference) = gate_topic(root, &board, &id)?;
            (topic, "gate", Some(reference))
        }
    };
    let path = display_path(root);
    let _lock = crate::infra::fs::lock(&lock_path(root))?;
    let (mut display, replaced_unreadable) = match read(&path) {
        Ok(display) => (display, false),
        Err(_) => (AideDisplay::default(), true),
    };
    let cleared = apply(
        &mut display,
        topic,
        kind,
        reference,
        crate::infra::clock::now_secs(),
    );
    crate::infra::fs::write_json(&path, &display)?;
    Ok(Shown {
        display,
        cleared,
        replaced_unreadable,
    })
}

/// The topic a gate belongs to, and the reference a part of it carries. Refuses a gate that
/// cannot be read.
fn gate_topic(root: &Path, board: &str, id: &str) -> Result<(Topic, PartRef), String> {
    // `address_of` builds a path from the slug.
    if !task::is_plain_id(board) {
        return Err(format!("no such board: {board}"));
    }
    let address = crate::registry::address_of(root, board)
        .ok_or_else(|| format!("no such board: {board}"))?;
    if !task::is_plain_id(id) {
        return Err(format!("not a gate id: {id}"));
    }
    let gate = crate::gate::get(root, board, id)?;
    // A gate whose task has no record is the gate's own topic; a record that is there and cannot
    // be read is an error, since another topic would clear what is on show.
    let task = match gate
        .task
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        Some(id) if task::exists(root, board, id)? => Some(task::get(root, board, id)?),
        _ => None,
    };
    let topic = topic_of(
        &address.nwo,
        board,
        &gate,
        task.as_ref(),
        // Only read when the parent is a key that needs it, so a broken config blocks nothing else.
        || crate::kernel::config::resolve_config(&address.nwo).map(|r| r.settings.issue_keys),
    )?;
    let reference = PartRef {
        board: board.to_string(),
        id: gate.id,
        extra: Map::new(),
    };
    Ok((topic, reference))
}

/// The topic of a gate: its task's parent when it has one, so the gates of the children of one
/// parent are one topic; else its task; else the gate alone.
fn topic_of(
    nwo: &str,
    board: &str,
    gate: &Gate,
    task: Option<&Task>,
    issue_keys: impl FnOnce() -> Result<Map<String, Value>, String>,
) -> Result<Topic, String> {
    let Some(task) = task else {
        return Ok(Topic::new(
            format!("{nwo}#g:{board}:{}", gate.id),
            format!("{nwo} · {}", gate.title),
        ));
    };
    let parent = task
        .parent
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty());
    let Some(raw) = parent else {
        let id = match task.id.chars().all(|c| c.is_ascii_digit()) {
            true => format!("#{}", task.id),
            false => task.id.clone(),
        };
        return Ok(Topic::new(
            format!("{nwo}#t:{board}:{}", task.id),
            format!("{nwo} · {id} {}", task.title),
        ));
    };
    // The same reading of the record's `parent` as the board's cards: a key is turned into the
    // issue it names where `issueKeys` says which repository that is, and one it cannot turn is
    // kept as written.
    let url = match crate::kernel::brief::is_key(raw) {
        true => task::parent_issue_url(raw, &issue_keys()?),
        false => Some(raw.to_string()),
    };
    let key = task::parent_key(url.as_deref().unwrap_or(raw));
    let short = match key.rsplit_once('#') {
        Some((repo, number)) if repo == nwo.to_ascii_lowercase() => format!("#{number}"),
        _ => key.clone(),
    };
    Ok(Topic::new(
        format!("{nwo}#p:{key}"),
        format!("{nwo} · 親 {short}"),
    ))
}

/// Whether two references name the same gate; a key a newer binary added to one is no difference.
fn same_ref(a: &Option<PartRef>, b: &Option<PartRef>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.board == b.board && a.id == b.id,
        (None, None) => true,
        _ => false,
    }
}

/// Put a part on top of `display`. `true` when it replaced a topic that had parts, which went
/// away with it.
fn apply(
    display: &mut AideDisplay,
    topic: Topic,
    kind: &str,
    reference: Option<PartRef>,
    now: i64,
) -> bool {
    let mut cleared = false;
    // A topic already stored keeps the keys a newer binary put in it; only the label is the
    // latest wording.
    let named = |mut stored: Topic| {
        stored.label = topic.label.clone();
        stored
    };
    if display.topic.as_ref().is_some_and(|t| t.key == topic.key) {
        display.topic = display.topic.take().map(named);
    } else {
        if let Some(previous) = display.topic.take() {
            display.earlier.retain(|t| t.key != previous.key);
            display.earlier.insert(0, previous);
        }
        let back = display.earlier.iter().position(|t| t.key == topic.key);
        let current = match back {
            Some(i) => named(display.earlier.remove(i)),
            None => topic.clone(),
        };
        display.earlier.truncate(EARLIER_KEPT);
        cleared = !display.parts.is_empty();
        display.parts.clear();
        display.topic = Some(current);
    }
    // The same thing shown again is the same part moved to the top, with what it carried.
    let mut part = match display
        .parts
        .iter()
        .position(|p| p.kind == kind && same_ref(&p.reference, &reference))
    {
        Some(i) => display.parts.remove(i),
        None => AidePart {
            id: String::new(),
            kind: kind.to_string(),
            reference,
            topic: String::new(),
            shown_at: now,
            by: String::new(),
            extra: Map::new(),
        },
    };
    part.id = format!("p{}", display.seq);
    part.topic = topic.key;
    part.shown_at = now;
    part.by = "aide".to_string();
    display.parts.insert(0, part);
    display.seq += 1;
    display.parts.truncate(PARTS_KEPT);
    cleared
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NWO: &str = "acme/widget";

    fn gate(id: &str, task: Option<&str>) -> Gate {
        serde_json::from_value(json!({
            "id": id, "kind": "plan", "worktree": "/tmp/wt", "task": task,
            "title": format!("Gate {id}"), "openedAt": "20260922T010000Z",
        }))
        .unwrap()
    }

    fn task_of(id: &str, parent: Option<&str>) -> Task {
        serde_json::from_value(json!({
            "id": id, "kind": "start", "title": "Add a timer", "doneWhen": "pr",
            "autoStart": true, "status": "queued", "parent": parent,
            "createdAt": "20260922T000000Z", "updatedAt": "20260922T000000Z",
        }))
        .unwrap()
    }

    fn keys(pairs: &[(&str, &str)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect()
    }

    fn topic(
        task: Option<&Task>,
        gate_id: &str,
        issue_keys: &Map<String, Value>,
    ) -> (String, String) {
        let t = topic_of(NWO, "acme-widget", &gate(gate_id, None), task, || {
            Ok(issue_keys.clone())
        })
        .unwrap();
        (t.key, t.label)
    }

    #[test]
    fn the_children_of_one_parent_are_one_topic_whatever_the_parent_is_written_as() {
        let by_url = task_of("7", Some("https://github.com/Acme/Widget/issues/40"));
        let by_key = task_of("8", Some("ALPHA-40"));
        let resolved = keys(&[("acme/widget", "ALPHA")]);
        let want = (
            "acme/widget#p:acme/widget#40".to_string(),
            "acme/widget · 親 #40".to_string(),
        );
        assert_eq!(topic(Some(&by_url), "g", &keys(&[])), want);
        assert_eq!(topic(Some(&by_key), "g", &resolved), want);
    }

    #[test]
    fn a_parent_key_that_names_no_repository_is_kept_as_written() {
        let task = task_of("8", Some("ALPHA-40"));
        let (key, label) = topic(Some(&task), "g", &keys(&[]));
        assert_eq!(key, "acme/widget#p:ALPHA-40");
        assert_eq!(label, "acme/widget · 親 ALPHA-40");
    }

    #[test]
    fn a_parent_in_another_repository_is_named_in_full() {
        let task = task_of("8", Some("https://github.com/acme/other/issues/3"));
        let (_, label) = topic(Some(&task), "g", &keys(&[]));
        assert_eq!(label, "acme/widget · 親 acme/other#3");
    }

    #[test]
    fn a_task_with_no_parent_is_its_own_topic_and_a_gate_with_no_task_is_too() {
        let numbered = task_of("41", None);
        assert_eq!(
            topic(Some(&numbered), "g", &keys(&[])),
            (
                "acme/widget#t:acme-widget:41".to_string(),
                "acme/widget · #41 Add a timer".to_string()
            )
        );
        let named = task_of("20260922T000000Z-timer", Some("  "));
        assert_eq!(
            topic(Some(&named), "g", &keys(&[])).1,
            "acme/widget · 20260922T000000Z-timer Add a timer"
        );
        assert_eq!(
            topic(None, "g1", &keys(&[])),
            (
                "acme/widget#g:acme-widget:g1".to_string(),
                "acme/widget · Gate g1".to_string()
            )
        );
    }

    fn on_show(display: &mut AideDisplay, key: &str, id: &str, now: i64) -> bool {
        let reference = PartRef {
            board: "b".to_string(),
            id: id.to_string(),
            extra: Map::new(),
        };
        apply(display, Topic::new(key, key), "gate", Some(reference), now)
    }

    fn ids(display: &AideDisplay) -> Vec<&str> {
        display.parts.iter().map(|p| p.id.as_str()).collect()
    }

    fn earlier(display: &AideDisplay) -> Vec<&str> {
        display.earlier.iter().map(|t| t.key.as_str()).collect()
    }

    #[test]
    fn another_topic_clears_the_parts_and_keeps_the_old_topic_by_name() {
        let mut d = AideDisplay::default();
        assert!(!on_show(&mut d, "a", "g1", 1));
        assert!(!on_show(&mut d, "a", "g2", 2));
        assert_eq!(ids(&d), ["p1", "p0"]);
        assert!(on_show(&mut d, "b", "g3", 3));
        assert_eq!(ids(&d), ["p2"]);
        assert_eq!(d.topic.as_ref().unwrap().key, "b");
        assert_eq!(earlier(&d), ["a"]);
        assert_eq!(d.parts[0].topic, "b");
        assert_eq!(d.parts[0].shown_at, 3);
    }

    #[test]
    fn showing_an_earlier_topic_again_takes_it_out_of_the_earlier_ones() {
        let mut d = AideDisplay::default();
        for key in ["a", "b", "a"] {
            on_show(&mut d, key, "g", 1);
        }
        assert_eq!(d.topic.as_ref().unwrap().key, "a");
        assert_eq!(earlier(&d), ["b"]);
    }

    #[test]
    fn only_the_last_five_earlier_topics_are_kept_and_none_twice() {
        let mut d = AideDisplay::default();
        for key in ["a", "b", "c", "d", "e", "f", "g", "h"] {
            on_show(&mut d, key, "g", 1);
        }
        assert_eq!(earlier(&d), ["g", "f", "e", "d", "c"]);
    }

    #[test]
    fn the_same_thing_shown_again_moves_to_the_top_and_the_topic_keeps_the_latest_label() {
        let mut d = AideDisplay::default();
        on_show(&mut d, "a", "g1", 1);
        on_show(&mut d, "a", "g2", 2);
        apply(
            &mut d,
            Topic::new("a", "A, renamed"),
            "gate",
            Some(PartRef {
                board: "b".to_string(),
                id: "g1".to_string(),
                extra: Map::new(),
            }),
            3,
        );
        assert_eq!(ids(&d), ["p2", "p1"]);
        assert_eq!(d.parts[0].reference.as_ref().unwrap().id, "g1");
        assert_eq!(d.topic.as_ref().unwrap().label, "A, renamed");
    }

    #[test]
    fn a_topic_keeps_twenty_parts_and_the_ids_never_repeat() {
        let mut d = AideDisplay::default();
        for n in 0..25 {
            on_show(&mut d, "a", &format!("g{n}"), n);
        }
        assert_eq!(d.parts.len(), PARTS_KEPT);
        assert_eq!(d.parts[0].id, "p24");
        assert_eq!(d.parts[19].id, "p5");
        assert_eq!(d.seq, 25);
    }

    /// A board of `acme/widget` in the sandbox, with a gate on a task that has a parent.
    fn board(sandbox: &crate::testing::Sandbox) -> (tempfile::TempDir, String) {
        let main = tempfile::tempdir().unwrap();
        let slug = crate::kernel::identity::slug_for(NWO, None);
        let repo = crate::kernel::identity::RepoInfo {
            main: main.path().to_string_lossy().to_string(),
            nwo: NWO.to_string(),
            repo: "widget".to_string(),
            hub: None,
            slug: slug.clone(),
            hub_name: format!("adjutant-{slug}"),
            nwo_source: "dirname",
        };
        crate::registry::note_board(&sandbox.state(), &repo);
        let tasks = task::dir(&sandbox.state(), &slug);
        std::fs::create_dir_all(&tasks).unwrap();
        let task = task_of("t1", Some("https://github.com/acme/widget/issues/40"));
        std::fs::write(tasks.join("t1.json"), serde_json::to_string(&task).unwrap()).unwrap();
        let gates = sandbox.state().join("gates").join(&slug);
        std::fs::create_dir_all(&gates).unwrap();
        let open = gate("g1", Some("t1"));
        std::fs::write(gates.join("g1.json"), serde_json::to_string(&open).unwrap()).unwrap();
        (main, slug)
    }

    fn files(root: &Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(root)
            .map(|dir| {
                dir.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    #[test]
    fn showing_the_waits_writes_the_display_and_reads_back() {
        let sandbox = crate::testing::Sandbox::empty();
        let root = sandbox.state();
        assert_eq!(display_of(&root), AideDisplay::default());
        let shown = show(&root, ShowRequest::Waits).unwrap();
        assert!(!shown.cleared && !shown.replaced_unreadable);
        assert_eq!(
            shown.display.topic,
            Some(Topic::new("waits", "判断待ちの一覧"))
        );
        assert_eq!(display_of(&root), shown.display);
        assert_eq!(files(&root), ["aide-display.json", "aide-display.lock"]);
        let raw: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("aide-display.json")).unwrap())
                .unwrap();
        assert_eq!(raw["parts"][0]["ref"], Value::Null);
        assert_eq!(raw["parts"][0]["kind"], "waits");
        assert_eq!(raw["parts"][0]["by"], "aide");
        assert!(raw["parts"][0]["shownAt"].is_i64());
    }

    fn display_of(root: &Path) -> AideDisplay {
        super::super::display(root).unwrap()
    }

    #[test]
    fn a_gate_is_shown_under_its_parent_and_a_gate_that_is_not_there_is_refused_with_nothing_written()
     {
        let sandbox = crate::testing::Sandbox::empty();
        let (_main, slug) = board(&sandbox);
        let root = sandbox.state();
        let gate_req = |board: &str, id: &str| ShowRequest::Gate {
            board: board.to_string(),
            id: id.to_string(),
        };
        for (board, id) in [
            ("nope-0000000000000000", "g1"),
            ("../boards/x", "g1"),
            (slug.as_str(), "missing"),
            (slug.as_str(), "../tasks/t1"),
            (slug.as_str(), ""),
        ] {
            assert!(show(&root, gate_req(board, id)).is_err(), "{board} {id}");
        }
        assert!(!root.join("aide-display.json").exists());
        assert!(!root.join("aide-display.lock").exists());

        let shown = show(&root, gate_req(&slug, "g1")).unwrap();
        let part = &shown.display.parts[0];
        assert_eq!(
            part.reference,
            Some(PartRef {
                board: slug.clone(),
                id: "g1".to_string(),
                extra: Map::new()
            })
        );
        assert_eq!(shown.display.topic.unwrap().label, "acme/widget · 親 #40");
    }

    #[test]
    fn an_unreadable_file_is_an_error_to_read_and_is_replaced_by_a_show() {
        let sandbox = crate::testing::Sandbox::empty();
        let root = sandbox.state();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("aide-display.json"), "{ not json").unwrap();
        assert!(super::super::display(&root).is_err());
        let shown = show(&root, ShowRequest::Waits).unwrap();
        assert!(shown.replaced_unreadable);
        assert_eq!(display_of(&root).parts.len(), 1);
    }

    #[test]
    fn a_key_this_binary_does_not_know_survives_a_show() {
        let sandbox = crate::testing::Sandbox::empty();
        let root = sandbox.state();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("aide-display.json"),
            json!({"parts": [], "earlier": [], "future": 1}).to_string(),
        )
        .unwrap();
        show(&root, ShowRequest::Waits).unwrap();
        let raw: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("aide-display.json")).unwrap())
                .unwrap();
        assert_eq!(raw["future"], 1);
        // And one inside a part's `ref`, which a newer binary may have added.
        std::fs::write(
            root.join("aide-display.json"),
            json!({"topic": {"key": "a", "label": "a"}, "parts": [{
                "id": "p0", "kind": "gate", "ref": {"board": "b", "id": "g", "more": 2},
                "topic": "a", "shownAt": 1, "by": "aide"}]})
            .to_string(),
        )
        .unwrap();
        let gate_ref = |id: &str| PartRef {
            board: "b".to_string(),
            id: id.to_string(),
            extra: Map::new(),
        };
        let mut d = display_of(&root);
        apply(
            &mut d,
            Topic::new("a", "a"),
            "gate",
            Some(gate_ref("g2")),
            2,
        );
        assert_eq!(d.parts[1].reference.as_ref().unwrap().extra["more"], 2);
    }

    #[test]
    fn a_task_that_cannot_be_read_refuses_and_one_with_no_record_is_the_gates_own_topic() {
        let sandbox = crate::testing::Sandbox::empty();
        let (_main, slug) = board(&sandbox);
        let root = sandbox.state();
        let req = || ShowRequest::Gate {
            board: slug.clone(),
            id: "g1".to_string(),
        };
        let record = task::dir(&root, &slug).join("t1.json");
        std::fs::write(&record, "{ malformed").unwrap();
        assert!(show(&root, req()).is_err());
        assert!(!root.join("aide-display.json").exists());
        assert!(!root.join("aide-display.lock").exists());

        std::fs::remove_file(&record).unwrap();
        let shown = show(&root, req()).unwrap();
        assert_eq!(
            shown.display.topic.unwrap().key,
            format!("acme/widget#g:{slug}:g1")
        );
    }

    #[test]
    fn a_config_that_cannot_be_read_blocks_only_a_gate_whose_parent_needs_it() {
        let sandbox = crate::testing::Sandbox::empty();
        let (_main, slug) = board(&sandbox);
        let root = sandbox.state();
        // The fixture's parent is a URL, which needs no config.
        std::fs::write(root.join("../config.json"), "{ broken").unwrap();
        let req = ShowRequest::Gate {
            board: slug,
            id: "g1".to_string(),
        };
        assert!(show(&root, req).is_ok());
        let by_key = task_of("k", Some("ALPHA-40"));
        let broken = topic_of(NWO, "b", &gate("g", None), Some(&by_key), || {
            Err("config".to_string())
        });
        assert_eq!(broken.unwrap_err(), "config");
    }

    #[test]
    fn showing_again_keeps_what_a_newer_binary_put_in_the_topic_and_the_part() {
        let mut d: AideDisplay = serde_json::from_value(json!({
            "topic": {"key": "a", "label": "old", "tx": 1},
            "parts": [{"id": "p0", "kind": "gate", "ref": {"board": "b", "id": "g", "rx": 2},
                       "topic": "a", "shownAt": 1, "by": "x", "px": 3}],
            "earlier": [{"key": "z", "label": "Z", "ex": 4}],
            "seq": 1,
        }))
        .unwrap();
        on_show(&mut d, "a", "g2", 2);
        on_show(&mut d, "a", "g", 3);
        assert_eq!(d.topic.as_ref().unwrap().extra["tx"], 1);
        assert_eq!(d.parts.len(), 2);
        assert_eq!(d.parts[0].id, "p2");
        assert_eq!(d.parts[0].shown_at, 3);
        assert_eq!(d.parts[0].by, "aide");
        assert_eq!(d.parts[0].extra["px"], 3);
        assert_eq!(d.parts[0].reference.as_ref().unwrap().extra["rx"], 2);
        // Away to the earlier topic and back, and into `earlier` and out again.
        on_show(&mut d, "z", "g", 4);
        assert_eq!(d.topic.as_ref().unwrap().extra["ex"], 4);
        assert_eq!(d.earlier[0].extra["tx"], 1);
        on_show(&mut d, "a", "g", 5);
        assert_eq!(d.topic.as_ref().unwrap().extra["tx"], 1);
        assert_eq!(d.earlier[0].extra["ex"], 4);
    }
}
