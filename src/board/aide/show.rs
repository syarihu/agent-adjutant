use std::path::Path;

use serde_json::{Map, Value, json};

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

/// What the aide asks to have shown: a kind and a reference, never content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShowRequest {
    /// One gate, by the slug of its board and its id.
    Gate { board: String, id: String },
    /// Everything that waits on the person.
    Waits,
    /// The read-through of a PR, all of what it says to look at or some of it.
    Read { pr: String, pick: Pick },
    /// A task, or a parent with the progress of its sub-issues.
    Task { board: String, of: TaskOf },
    /// A PR: where it stands.
    Pr { of: PrOf },
    /// A worker session of a board.
    Session { board: String, id: String },
    /// Some lines of a diff.
    Diff {
        source: DiffSource,
        path: String,
        from: u32,
        to: u32,
        side: Option<String>,
    },
}

/// Which points of a read-through to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// The ones that need attention, else the first.
    All,
    /// This one, counted from 1.
    Point(u32),
    /// Every one of this level: `attention`, `check` or `info`.
    Level(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOf {
    /// A task, by its id.
    Id(String),
    /// A parent, by `repos[].parents[].key` of `adjutant_work`.
    Parent(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrOf {
    /// The current PR of a task.
    Task { board: String, id: String },
    /// A PR that has a record (a review request, or a task's PR that was read), `owner/repo#N`.
    Pr(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    /// The diff stored with the PR's read-through.
    Pr(String),
    /// The diff an open gate carries.
    Gate { board: String, id: String },
}

/// The most lines one `diff` part shows.
pub const DIFF_LINES_MAX: u32 = 60;

/// What `show` did.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub display: AideDisplay,
    /// Another topic replaced the one on show, and its parts went with it.
    pub cleared: bool,
    /// The file could not be read and was written anew.
    pub replaced_unreadable: bool,
    /// For `read`: `{mode, points}` of the read-through, so the aide can follow up by number.
    pub read_through: Option<Value>,
}

/// What a request comes to before anything is written.
struct Resolved {
    topic: Topic,
    kind: &'static str,
    reference: Option<PartRef>,
    read_through: Option<Value>,
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
    show_with(root, request, || {
        crate::board::ask_resident(root, "/api/work")
    })
}

/// `show`, with the document `adjutant_work` returns given by `work`, which is asked only for a
/// request that needs the sessions or the parents of the repositories (they are the resident's).
pub fn show_with(
    root: &Path,
    request: ShowRequest,
    work: impl FnOnce() -> Result<Value, String>,
) -> Result<Shown, String> {
    let Resolved {
        topic,
        kind,
        reference,
        read_through,
    } = resolve(root, request, work)?;
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
        read_through,
    })
}

fn resolve(
    root: &Path,
    request: ShowRequest,
    work: impl FnOnce() -> Result<Value, String>,
) -> Result<Resolved, String> {
    let mut out = Resolved {
        topic: Topic::new(WAITS_KEY, WAITS_LABEL),
        kind: "waits",
        reference: None,
        read_through: None,
    };
    match request {
        ShowRequest::Waits => {}
        ShowRequest::Gate { board, id } => {
            let (topic, reference) = gate_topic(root, &board, &id)?;
            (out.topic, out.kind, out.reference) = (topic, "gate", Some(reference));
        }
        ShowRequest::Read { pr, pick } => {
            let pr = pr_ref_of(&pr)?;
            let read = crate::others::read_through(root, &pr)?
                .ok_or_else(|| format!("no read-through for {}", crate::others::pr_id(&pr)))?;
            let points = points_of(&read);
            let mut reference = PartRef {
                pr: Some(crate::others::pr_id(&pr)),
                ..Default::default()
            };
            check_pick(&crate::others::pr_id(&pr), &read, &points, &pick)?;
            match pick {
                Pick::All => {}
                Pick::Point(n) => reference.point = Some(n),
                Pick::Level(level) => reference.level = Some(level),
            }
            out.topic = pr_topic(root, &pr)?;
            out.kind = "read";
            out.reference = Some(reference);
            let mode = match read["mode"] == "split" {
                true => "split",
                false => "read",
            };
            out.read_through = Some(json!({ "mode": mode, "points": points }));
        }
        ShowRequest::Task { board, of } => {
            let nwo = board_nwo(root, &board)?;
            out.kind = "task";
            match of {
                TaskOf::Id(id) => {
                    let task = load_task(root, &board, &id)?
                        .ok_or_else(|| format!("no such task: {id}"))?;
                    out.topic = task_topic(&nwo, &board, &task)?;
                    out.reference = Some(PartRef::of(&board, &task.id));
                }
                TaskOf::Parent(parent) => {
                    let key = parent_in_work(&work()?, &nwo, &parent)?;
                    out.topic = parent_topic(&nwo, &key);
                    out.reference = Some(PartRef {
                        board,
                        parent: Some(key),
                        ..Default::default()
                    });
                }
            }
        }
        ShowRequest::Pr { of } => {
            out.kind = "pr";
            match of {
                PrOf::Task { board, id } => {
                    let nwo = board_nwo(root, &board)?;
                    let task = load_task(root, &board, &id)?
                        .ok_or_else(|| format!("no such task: {id}"))?;
                    if task.pr.as_deref().is_none_or(|p| p.trim().is_empty()) {
                        return Err(format!("task {id} has no PR"));
                    }
                    out.topic = task_topic(&nwo, &board, &task)?;
                    out.reference = Some(PartRef::of(&board, &task.id));
                }
                PrOf::Pr(text) => {
                    let pr = pr_ref_of(&text)?;
                    if crate::others::record(root, &pr)?.is_none() {
                        return Err(format!(
                            "no record of {}: ask for a read-through or show the task's PR by \
                             board and id",
                            crate::others::pr_id(&pr)
                        ));
                    }
                    out.topic = pr_topic(root, &pr)?;
                    out.reference = Some(PartRef {
                        pr: Some(crate::others::pr_id(&pr)),
                        ..Default::default()
                    });
                }
            }
        }
        ShowRequest::Session { board, id } => {
            let nwo = board_nwo(root, &board)?;
            let task = session_task(&work()?, &board, &id)?;
            let task = match task {
                Some(task_id) => load_task(root, &board, &task_id)?,
                None => None,
            };
            out.topic = match &task {
                Some(task) => task_topic(&nwo, &board, task)?,
                None => Topic::new(format!("{nwo}#s:{board}:{id}"), format!("{nwo} · {id}")),
            };
            out.kind = "session";
            out.reference = Some(PartRef::of(&board, &id));
        }
        ShowRequest::Diff {
            source,
            path,
            from,
            to,
            side,
        } => {
            if from < 1 || to < from {
                return Err("a diff range needs 1 <= from <= to".to_string());
            }
            if to - from >= DIFF_LINES_MAX {
                return Err(format!("a diff part shows at most {DIFF_LINES_MAX} lines"));
            }
            if path.trim().is_empty() || path.starts_with('/') || path.chars().any(char::is_control)
            {
                return Err(format!("not a path of the diff: {path:?}"));
            }
            if side
                .as_deref()
                .is_some_and(|s| !matches!(s, "right" | "left"))
            {
                return Err("side must be right or left".to_string());
            }
            let mut reference = PartRef {
                path: Some(path.clone()),
                from: Some(from),
                to: Some(to),
                side,
                ..Default::default()
            };
            let diff = match source {
                DiffSource::Pr(text) => {
                    let pr = pr_ref_of(&text)?;
                    let stored = crate::others::stored_diff(root, &pr)?;
                    let diff = stored
                        .as_ref()
                        .and_then(|d| d["diff"].as_str())
                        .ok_or_else(|| format!("no diff stored for {}", crate::others::pr_id(&pr)))?
                        .to_string();
                    out.topic = pr_topic(root, &pr)?;
                    reference.pr = Some(crate::others::pr_id(&pr));
                    diff
                }
                DiffSource::Gate { board, id } => {
                    let (topic, gate_ref) = gate_topic(root, &board, &id)?;
                    let gate = crate::gate::get(root, &board, &id)?;
                    let diff = gate
                        .diff
                        .ok_or_else(|| format!("gate {id} carries no diff"))?;
                    out.topic = topic;
                    reference.board = gate_ref.board;
                    reference.id = gate_ref.id;
                    diff
                }
            };
            if !diff_names_file(&diff, &path) {
                return Err(format!("{path} is not in that diff"));
            }
            out.kind = "diff";
            out.reference = Some(reference);
        }
    }
    Ok(out)
}

/// The slug of `board` and the name of its repository. `address_of` builds a path from the slug.
fn board_nwo(root: &Path, board: &str) -> Result<String, String> {
    if !task::is_plain_id(board) {
        return Err(format!("no such board: {board}"));
    }
    crate::registry::address_of(root, board)
        .map(|a| a.nwo)
        .ok_or_else(|| format!("no such board: {board}"))
}

/// The PR `text` names.
fn pr_ref_of(text: &str) -> Result<crate::task::PrRef, String> {
    crate::others::pr_of(text).ok_or_else(|| format!("not a pull request: {text}"))
}

/// A task that is there, `None` for one that has no record, and an error for a record that
/// cannot be read, since another topic would clear what is on show.
fn load_task(root: &Path, board: &str, id: &str) -> Result<Option<Task>, String> {
    let id = id.trim();
    match task::exists(root, board, id)? {
        true => task::get(root, board, id).map(Some),
        false => Ok(None),
    }
}

/// The points of a stored read-through the aide can name by number.
fn points_of(read: &Value) -> Vec<Value> {
    let list = read["points"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    list.iter()
        .enumerate()
        .map(|(i, p)| {
            json!({
                "n": i + 1,
                "level": p["level"],
                "title": p["title"],
                "path": p["path"],
                "line": p["line"],
            })
        })
        .collect()
}

fn check_pick(id: &str, read: &Value, points: &[Value], pick: &Pick) -> Result<(), String> {
    if read["mode"] == "split" {
        return match pick {
            Pick::All => Ok(()),
            _ => Err(format!(
                "{id} has a split proposal, which has no points to pick"
            )),
        };
    }
    match pick {
        Pick::All => Ok(()),
        Pick::Point(n) => match (*n as usize).checked_sub(1) {
            Some(i) if i < points.len() => Ok(()),
            _ => Err(format!("{id} has {} points", points.len())),
        },
        Pick::Level(level) => {
            if !matches!(level.as_str(), "attention" | "check" | "info") {
                return Err("level must be attention, check or info".to_string());
            }
            match points.iter().any(|p| p["level"] == level.as_str()) {
                true => Ok(()),
                false => Err(format!("{id} has no point of level {level}")),
            }
        }
    }
}

/// The topic of a PR: its task's when the record says the PR is read for a task that is still
/// there, so a task and its PR are one topic; else the PR alone.
fn pr_topic(root: &Path, pr: &crate::task::PrRef) -> Result<Topic, String> {
    let record = crate::others::record(root, pr)?;
    if let Some(link) = record
        .as_ref()
        .and_then(|r| r.task.as_ref())
        .filter(|l| task::is_plain_id(&l.board))
        && let Some(address) = crate::registry::address_of(root, &link.board)
        && let Some(task) = load_task(root, &link.board, &link.id)?
    {
        return task_topic(&address.nwo, &link.board, &task);
    }
    let name = record
        .as_ref()
        .map(|r| r.repo.clone())
        .unwrap_or_else(|| pr.nwo());
    let title = record
        .as_ref()
        .map(|r| format!(" {}", r.title))
        .unwrap_or_default();
    Ok(Topic::new(
        format!("{}#pr:{}", pr.nwo(), pr.number),
        format!("{name} · PR #{}{title}", pr.number),
    ))
}

/// The parent of `wanted` among those the repositories of the document list, by the key the
/// board joins children by (matched however the case or the form is written).
fn parent_in_work(work: &Value, nwo: &str, wanted: &str) -> Result<String, String> {
    let want = task::parent_key(wanted).to_ascii_lowercase();
    let found = work["repos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| {
            r["nwo"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(nwo))
        })
        .flat_map(|r| r["parents"].as_array().into_iter().flatten())
        .filter_map(|p| p["key"].as_str())
        .find(|key| key.to_ascii_lowercase() == want);
    found
        .map(str::to_string)
        .ok_or_else(|| format!("no parent {wanted} in {nwo} on the board"))
}

/// The task a session is on (`None` when it is on none), or an error when no session of the
/// board has this id.
fn session_task(work: &Value, board: &str, id: &str) -> Result<Option<String>, String> {
    let repos = work["repos"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    for repo in repos {
        for row in repo["rows"].as_array().into_iter().flatten() {
            if row["board"] == board && row["session"]["id"] == id {
                return Ok(row["task"]["id"].as_str().map(str::to_string));
            }
        }
        for hub in repo["hubSessions"].as_array().into_iter().flatten() {
            if hub["board"] == board && hub["session"]["id"] == id {
                return Ok(None);
            }
        }
    }
    Err(format!("no session {id} on board {board}"))
}

/// A path as git writes it: plain, or in double quotes with C escapes when it holds a quote, a
/// backslash, a control character or (by default) a byte over 127: `"a/\346\227\245.txt"`. The same
/// reading as `prdUnquote` in `src/ui/pr-diff.js`.
fn git_unquote(text: &str) -> String {
    let Some(inner) = text.strip_prefix('"').and_then(|t| t.strip_suffix('"')) else {
        return text.to_string();
    };
    let bytes = inner.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        let c = bytes[i];
        if (b'0'..=b'7').contains(&c) {
            let mut n = 0u32;
            let mut len = 0;
            while len < 3 && i < bytes.len() && (b'0'..=b'7').contains(&bytes[i]) {
                n = n * 8 + u32::from(bytes[i] - b'0');
                i += 1;
                len += 1;
            }
            out.push((n & 255) as u8);
            continue;
        }
        out.push(match c {
            b'n' => 10,
            b't' => 9,
            b'a' => 7,
            b'b' => 8,
            b'f' => 12,
            b'r' => 13,
            b'v' => 11,
            other => other,
        });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The path of a `---` or `+++` line: unquoted, without the tab git adds and its `a/` or `b/`.
fn diff_line_path(raw: &str) -> Option<String> {
    let raw = raw.split('\t').next().unwrap_or("").trim();
    let path = git_unquote(raw);
    (path != "/dev/null").then(|| {
        path.strip_prefix("a/")
            .or_else(|| path.strip_prefix("b/"))
            .map(str::to_string)
            .unwrap_or(path)
    })
}

/// The two paths of a `diff --git` line, each word quoted or not.
fn header_paths(rest: &str) -> Option<(String, String)> {
    let rest = rest.trim_end();
    let word = |text: &str| -> Option<(usize, String)> {
        if text.starts_with('"') {
            let mut escaped = false;
            for (i, c) in text.char_indices().skip(1) {
                match (escaped, c) {
                    (true, _) => escaped = false,
                    (false, '\\') => escaped = true,
                    (false, '"') => return Some((i + 1, git_unquote(&text[..=i]))),
                    _ => {}
                }
            }
            None
        } else {
            let end = text.find(' ').unwrap_or(text.len());
            Some((end, text[..end].to_string()))
        }
    };
    let (n, a) = word(rest)?;
    if let Some(b) = rest[n..].strip_prefix(' ')
        && let Some((m, b)) = word(b)
        && n + 1 + m == rest.len()
    {
        let strip = |p: String| {
            p.strip_prefix("a/")
                .or_else(|| p.strip_prefix("b/"))
                .map(str::to_string)
                .unwrap_or(p)
        };
        return Some((strip(a), strip(b)));
    }
    // A path with a space in it: the same path on both sides, else split at the last ` b/`.
    // A byte index into text that may hold any character: `get` says no for one that is not on a boundary.
    let same = rest.strip_prefix("a/").and_then(|r| {
        let half = r.len().checked_sub(2)? / 2;
        let (left, right) = (r.get(..half)?, r.get(half..)?);
        (r.len() % 2 == 1 && right.strip_prefix(" b/") == Some(left)).then(|| left.to_string())
    });
    if let Some(p) = same {
        return Some((p.clone(), p));
    }
    let at = rest.rfind(" b/")?;
    Some((
        git_unquote(&rest[..at])
            .trim_start_matches("a/")
            .to_string(),
        rest[at + 3..].to_string(),
    ))
}

/// Whether the unified diff `diff` has a file `path`: by its `diff --git` line, or its old or new
/// name, read as the page reads them (so a name git quoted is found by what it says).
fn diff_names_file(diff: &str, path: &str) -> bool {
    diff.lines().any(|line| {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("diff --git ") {
            return header_paths(rest).is_some_and(|(a, b)| b == path || a == path);
        }
        match (line.strip_prefix("--- "), line.strip_prefix("+++ ")) {
            (Some(r), _) | (_, Some(r)) => diff_line_path(r).is_some_and(|p| p == path),
            _ => false,
        }
    })
}

/// The topic and the reference of a gate. Refuses a gate that cannot be read.
fn gate_topic(root: &Path, board: &str, id: &str) -> Result<(Topic, PartRef), String> {
    let nwo = board_nwo(root, board)?;
    if !task::is_plain_id(id) {
        return Err(format!("not a gate id: {id}"));
    }
    let gate = crate::gate::get(root, board, id)?;
    // A gate whose task has no record is the gate's own topic; a record that is there and cannot
    // be read is an error, since another topic would clear what is on show.
    let task = match gate.task.as_deref() {
        Some(task_id) => load_task(root, board, task_id)?,
        None => None,
    };
    let topic = match &task {
        Some(task) => task_topic(&nwo, board, task)?,
        None => gate_own_topic(&nwo, board, &gate),
    };
    Ok((topic, PartRef::of(board, &gate.id)))
}

fn gate_own_topic(nwo: &str, board: &str, gate: &Gate) -> Topic {
    Topic::new(
        format!("{nwo}#g:{board}:{}", gate.id),
        format!("{nwo} · {}", gate.title),
    )
}

/// The topic of a task: its parent when it has one, so the children of one parent are one topic;
/// else the task.
fn task_topic(nwo: &str, board: &str, task: &Task) -> Result<Topic, String> {
    topic_of_task(nwo, board, task, || {
        // Only read when the parent is a key that needs it, so a broken config blocks nothing else.
        crate::kernel::config::resolve_config(nwo).map(|r| r.settings.issue_keys)
    })
}

fn topic_of_task(
    nwo: &str,
    board: &str,
    task: &Task,
    issue_keys: impl FnOnce() -> Result<Map<String, Value>, String>,
) -> Result<Topic, String> {
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
    Ok(parent_topic(
        nwo,
        &task::parent_key(url.as_deref().unwrap_or(raw)),
    ))
}

/// The topic of a parent by the key the board joins its children by.
fn parent_topic(nwo: &str, key: &str) -> Topic {
    let short = match key.rsplit_once('#') {
        Some((repo, number)) if repo == nwo.to_ascii_lowercase() => format!("#{number}"),
        _ => key.to_string(),
    };
    Topic::new(format!("{nwo}#p:{key}"), format!("{nwo} · 親 {short}"))
}

/// Whether two references name the same thing; a key a newer binary added to one is no
/// difference. A side left out is the right one.
fn same_ref(a: &Option<PartRef>, b: &Option<PartRef>) -> bool {
    let key = |r: &PartRef| PartRef {
        side: r.side.clone().filter(|s| s != "right"),
        extra: Map::new(),
        ..r.clone()
    };
    match (a, b) {
        (Some(a), Some(b)) => key(a) == key(b),
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
        let t = match task {
            Some(task) => topic_of_task(NWO, "acme-widget", task, || Ok(issue_keys.clone())),
            None => Ok(gate_own_topic(NWO, "acme-widget", &gate(gate_id, None))),
        }
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
        let reference = PartRef::of("b", id);
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
            Some(PartRef::of("b", "g1")),
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
        assert_eq!(part.reference, Some(PartRef::of(&slug, "g1")));
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
        let gate_ref = |id: &str| PartRef::of("b", id);
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
        let broken = topic_of_task(NWO, "b", &by_key, || Err("config".to_string()));
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

    const PR: &str = "acme/widget#7";
    const SHA: &str = "0123456789012345678901234567890123456789";
    const FILE_DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n keep\n";

    /// A record of PR 7 of `acme/widget` (with a link to a task when `task` is given), its
    /// read-through with three points, and the diff it was made from.
    fn pr_files(root: &Path, task: Option<(&str, &str)>) {
        let others = root.join("others");
        std::fs::create_dir_all(others.join("reads")).unwrap();
        std::fs::create_dir_all(others.join("diffs")).unwrap();
        let mut record = json!({
            "id": PR, "repo": "Acme/Widget", "number": 7,
            "url": "https://github.com/acme/widget/pull/7",
            "registered": true, "title": "Add a timer", "base": "main", "head": "f", "headSha": SHA,
            "prState": "open", "additions": 1, "deletions": 1, "changedFiles": 1,
            "firstSeenAt": "20261001T000000Z", "readAt": "20261001T000000Z",
        });
        if let Some((board, id)) = task {
            record["task"] = json!({"board": board, "id": id});
        }
        std::fs::write(others.join("acme~widget~7.json"), record.to_string()).unwrap();
        let read = json!({
            "id": PR, "madeAt": "20261001T000000Z", "sha": SHA,
            "points": [
                {"level": "attention", "path": "src/a.rs", "line": 2, "title": "One", "why": "w"},
                {"level": "check", "path": "src/a.rs", "title": "Two", "why": "w"},
                {"level": "info", "path": "src/a.rs", "line": 1, "title": "Three", "why": "w"},
            ],
        });
        std::fs::write(others.join("reads/acme~widget~7.json"), read.to_string()).unwrap();
        let diff =
            json!({"sha": SHA, "savedAt": "20261001T000000Z", "bytes": 1, "diff": FILE_DIFF});
        std::fs::write(others.join("diffs/acme~widget~7.json"), diff.to_string()).unwrap();
    }

    fn no_work() -> Result<Value, String> {
        Err("no resident".to_string())
    }

    fn nothing_written(root: &Path) {
        assert!(!root.join("aide-display.json").exists());
        assert!(!root.join("aide-display.lock").exists());
    }

    fn read_req(pick: Pick) -> ShowRequest {
        ShowRequest::Read {
            pr: PR.to_string(),
            pick,
        }
    }

    #[test]
    fn a_pr_is_shown_under_its_task_when_the_record_names_a_task_and_else_by_itself() {
        let sandbox = crate::testing::Sandbox::empty();
        let (_main, slug) = board(&sandbox);
        let root = sandbox.state();
        pr_files(&root, None);
        let shown = show_with(&root, read_req(Pick::All), no_work).unwrap();
        let topic = shown.display.topic.unwrap();
        assert_eq!(topic.key, "acme/widget#pr:7");
        assert_eq!(topic.label, "Acme/Widget · PR #7 Add a timer");
        let read = shown.read_through.unwrap();
        assert_eq!(read["mode"], "read");
        assert_eq!(read["points"][1]["n"], 2);
        assert_eq!(read["points"][1]["level"], "check");

        // A record read for a task that is there is that task's topic, which is its parent's.
        pr_files(&root, Some((&slug, "t1")));
        let parent_topic = "acme/widget#p:acme/widget#40".to_string();
        for req in [
            read_req(Pick::Point(2)),
            ShowRequest::Pr {
                of: PrOf::Pr(PR.to_string()),
            },
            ShowRequest::Diff {
                source: DiffSource::Pr(PR.to_string()),
                path: "src/a.rs".to_string(),
                from: 1,
                to: 2,
                side: None,
            },
        ] {
            let shown = show_with(&root, req, no_work).unwrap();
            assert_eq!(shown.display.topic.unwrap().key, parent_topic);
        }
        // A task that has gone: the PR's own topic again.
        std::fs::remove_file(task::dir(&root, &slug).join("t1.json")).unwrap();
        let shown = show_with(&root, read_req(Pick::All), no_work).unwrap();
        assert_eq!(shown.display.topic.unwrap().key, "acme/widget#pr:7");
    }

    #[test]
    fn a_task_is_shown_by_id_or_by_parent_and_a_parent_the_board_does_not_have_is_refused() {
        let sandbox = crate::testing::Sandbox::empty();
        let (_main, slug) = board(&sandbox);
        let root = sandbox.state();
        let by_id = ShowRequest::Task {
            board: slug.clone(),
            of: TaskOf::Id("t1".to_string()),
        };
        let shown = show_with(&root, by_id, no_work).unwrap();
        // Its parent's topic, the one its gate is under.
        assert_eq!(
            shown.display.topic.unwrap().key,
            "acme/widget#p:acme/widget#40"
        );
        assert_eq!(
            shown.display.parts[0].reference,
            Some(PartRef::of(&slug, "t1"))
        );

        let work = || {
            Ok(json!({"repos": [
                {"nwo": "acme/other", "parents": [{"key": "acme/widget#41"}]},
                {"nwo": "Acme/Widget", "parents": [{"key": "acme/widget#41"}, {"key": "acme/widget#40"}]},
            ]}))
        };
        let parent = |text: &str| ShowRequest::Task {
            board: slug.clone(),
            of: TaskOf::Parent(text.to_string()),
        };
        for text in [
            "acme/widget#40",
            "https://github.com/Acme/Widget/issues/40",
            " ACME/WIDGET#40 ",
        ] {
            let shown = show_with(&root, parent(text), work).unwrap();
            let part = &shown.display.parts[0];
            assert_eq!(
                part.reference.as_ref().unwrap().parent.as_deref(),
                Some("acme/widget#40")
            );
            assert_eq!(part.topic, "acme/widget#p:acme/widget#40");
            assert_eq!(shown.display.topic.unwrap().label, "acme/widget · 親 #40");
        }
        let before = display_of(&root);
        let err = show_with(&root, parent("acme/widget#99"), work).unwrap_err();
        assert!(err.contains("no parent acme/widget#99"), "{err}");
        assert_eq!(
            show_with(&root, parent("acme/widget#40"), no_work).unwrap_err(),
            "no resident"
        );
        assert_eq!(display_of(&root), before);
    }

    #[test]
    fn a_session_is_shown_under_its_task_or_alone_and_one_that_is_not_listed_is_refused() {
        let sandbox = crate::testing::Sandbox::empty();
        let (_main, slug) = board(&sandbox);
        let root = sandbox.state();
        let work = || {
            Ok(json!({"repos": [{"nwo": "acme/widget",
                "rows": [
                    {"board": slug, "session": {"id": "s1"}, "task": {"id": "t1"}},
                    {"board": slug, "session": {"id": "s2"}},
                ],
                "hubSessions": [{"board": slug, "session": {"id": "s3"}}]}]}))
        };
        let session = |id: &str| ShowRequest::Session {
            board: slug.clone(),
            id: id.to_string(),
        };
        let shown = show_with(&root, session("s1"), work).unwrap();
        assert_eq!(
            shown.display.topic.unwrap().key,
            "acme/widget#p:acme/widget#40"
        );
        let shown = show_with(&root, session("s2"), work).unwrap();
        assert_eq!(
            shown.display.topic.unwrap().key,
            format!("acme/widget#s:{slug}:s2")
        );
        assert!(show_with(&root, session("s3"), work).is_ok());
        let before = display_of(&root);
        assert!(
            show_with(&root, session("s9"), work)
                .unwrap_err()
                .contains("no session s9")
        );
        assert_eq!(display_of(&root), before);
    }

    #[test]
    fn a_request_that_names_nothing_it_can_show_is_refused_and_writes_nothing() {
        let sandbox = crate::testing::Sandbox::empty();
        let (_main, slug) = board(&sandbox);
        let root = sandbox.state();
        let diff = |source: DiffSource, path: &str, from: u32, to: u32, side: Option<&str>| {
            ShowRequest::Diff {
                source,
                path: path.to_string(),
                from,
                to,
                side: side.map(str::to_string),
            }
        };
        let pr = || DiffSource::Pr(PR.to_string());
        // No read-through, no record, no diff yet.
        for req in [
            read_req(Pick::All),
            ShowRequest::Pr {
                of: PrOf::Pr(PR.to_string()),
            },
            diff(pr(), "src/a.rs", 1, 2, None),
        ] {
            assert!(show_with(&root, req, no_work).is_err());
        }
        // A task with no PR; a task that is not there; a bad board.
        let no_pr = ShowRequest::Pr {
            of: PrOf::Task {
                board: slug.clone(),
                id: "t1".to_string(),
            },
        };
        assert_eq!(
            show_with(&root, no_pr, no_work).unwrap_err(),
            "task t1 has no PR"
        );
        let gone = ShowRequest::Task {
            board: slug.clone(),
            of: TaskOf::Id("nope".to_string()),
        };
        assert!(show_with(&root, gone, no_work).is_err());
        let bad = ShowRequest::Task {
            board: "../x".to_string(),
            of: TaskOf::Id("t1".to_string()),
        };
        assert!(show_with(&root, bad, no_work).is_err());
        let text = ShowRequest::Pr {
            of: PrOf::Pr("not a pr".to_string()),
        };
        assert!(
            show_with(&root, text, no_work)
                .unwrap_err()
                .contains("not a pull request")
        );
        nothing_written(&root);

        pr_files(&root, None);
        let asks: Vec<(ShowRequest, &str)> = vec![
            (read_req(Pick::Point(0)), "has 3 points"),
            (read_req(Pick::Point(4)), "has 3 points"),
            (read_req(Pick::Level("urgent".to_string())), "level must be"),
            (diff(pr(), "src/b.rs", 1, 2, None), "not in that diff"),
            (diff(pr(), "/src/a.rs", 1, 2, None), "not a path"),
            (diff(pr(), "src/a.rs", 0, 2, None), "1 <= from <= to"),
            (diff(pr(), "src/a.rs", 3, 2, None), "1 <= from <= to"),
            (diff(pr(), "src/a.rs", 1, 61, None), "at most 60"),
            (diff(pr(), "src/a.rs", 1, 2, Some("up")), "side must be"),
        ];
        for (req, why) in asks {
            let err = show_with(&root, req.clone(), no_work).unwrap_err();
            assert!(err.contains(why), "{req:?}: {err}");
        }
        // 60 lines is the most.
        assert!(show_with(&root, diff(pr(), "src/a.rs", 1, 60, Some("left")), no_work).is_ok());
        assert!(show_with(&root, read_req(Pick::Level("info".to_string())), no_work).is_ok());
        // A level with no point, and a split proposal, which has none to pick.
        let path = root.join("others/reads/acme~widget~7.json");
        let mut read: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        read["points"] = json!([{"level": "check", "path": "p", "title": "t", "why": "w"}]);
        std::fs::write(&path, read.to_string()).unwrap();
        let err = show_with(
            &root,
            read_req(Pick::Level("attention".to_string())),
            no_work,
        )
        .unwrap_err();
        assert!(err.contains("no point of level attention"), "{err}");
        read["mode"] = json!("split");
        std::fs::write(&path, read.to_string()).unwrap();
        let err = show_with(&root, read_req(Pick::Point(1)), no_work).unwrap_err();
        assert!(err.contains("split proposal"), "{err}");
        let shown = show_with(&root, read_req(Pick::All), no_work).unwrap();
        assert_eq!(shown.read_through.unwrap()["mode"], "split");
    }

    #[test]
    fn a_diff_of_a_stored_read_of_another_head_and_a_gates_diff_are_checked_at_the_source() {
        let sandbox = crate::testing::Sandbox::empty();
        let (_main, slug) = board(&sandbox);
        let root = sandbox.state();
        pr_files(&root, None);
        let req = |source: DiffSource| ShowRequest::Diff {
            source,
            path: "src/a.rs".to_string(),
            from: 1,
            to: 2,
            side: None,
        };
        // The stored diff is of another head than the read-through.
        let path = root.join("others/diffs/acme~widget~7.json");
        let mut diff: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        diff["sha"] = json!("f".repeat(40));
        std::fs::write(&path, diff.to_string()).unwrap();
        let err = show_with(&root, req(DiffSource::Pr(PR.to_string())), no_work).unwrap_err();
        assert!(err.contains("no diff stored"), "{err}");

        // A gate with a diff, and one with none.
        let gates = root.join("gates").join(&slug);
        let mut with_diff = serde_json::to_value(gate("g2", Some("t1"))).unwrap();
        with_diff["diff"] = json!(FILE_DIFF);
        std::fs::write(gates.join("g2.json"), with_diff.to_string()).unwrap();
        let by = |id: &str| {
            req(DiffSource::Gate {
                board: slug.clone(),
                id: id.to_string(),
            })
        };
        let shown = show_with(&root, by("g2"), no_work).unwrap();
        let reference = shown.display.parts[0].reference.clone().unwrap();
        assert_eq!(
            (reference.board.as_str(), reference.id.as_str()),
            (slug.as_str(), "g2")
        );
        assert_eq!(reference.pr, None);
        assert_eq!(
            shown.display.topic.unwrap().key,
            "acme/widget#p:acme/widget#40"
        );
        assert!(
            show_with(&root, by("g1"), no_work)
                .unwrap_err()
                .contains("carries no diff")
        );
    }

    #[test]
    fn two_references_are_one_thing_by_every_field_but_the_ones_a_newer_binary_added() {
        let diff = |from: u32, side: Option<&str>| PartRef {
            pr: Some(PR.to_string()),
            path: Some("a".to_string()),
            from: Some(from),
            to: Some(9),
            side: side.map(str::to_string),
            ..Default::default()
        };
        let some = |r: PartRef| Some(r);
        assert!(same_ref(
            &some(diff(1, None)),
            &some(diff(1, Some("right")))
        ));
        assert!(!same_ref(
            &some(diff(1, None)),
            &some(diff(1, Some("left")))
        ));
        assert!(!same_ref(&some(diff(1, None)), &some(diff(2, None))));
        let mut newer = diff(1, None);
        newer.extra.insert("more".to_string(), json!(1));
        assert!(same_ref(&some(newer), &some(diff(1, None))));
        let read = |point: Option<u32>, level: Option<&str>| PartRef {
            pr: Some(PR.to_string()),
            point,
            level: level.map(str::to_string),
            ..Default::default()
        };
        assert!(!same_ref(
            &some(read(Some(1), None)),
            &some(read(None, None))
        ));
        assert!(!same_ref(
            &some(read(None, Some("info"))),
            &some(read(None, Some("check")))
        ));
        assert!(!same_ref(
            &some(PartRef::of("b", "1")),
            &some(PartRef::of("b", "2"))
        ));
        let parent = |p: &str| PartRef {
            board: "b".to_string(),
            parent: Some(p.to_string()),
            ..Default::default()
        };
        assert!(!same_ref(&some(parent("x")), &some(parent("y"))));
        assert!(!same_ref(&some(parent("x")), &None));
    }

    #[test]
    fn a_ref_always_writes_board_and_id_and_an_old_file_still_loads() {
        let value = serde_json::to_value(PartRef {
            pr: Some(PR.to_string()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(value, json!({"board": "", "id": "", "pr": PR}));
        let old: PartRef = serde_json::from_value(json!({"board": "b", "id": "g"})).unwrap();
        assert_eq!(old, PartRef::of("b", "g"));
        let new: PartRef = serde_json::from_value(json!({"pr": PR, "point": 2})).unwrap();
        assert_eq!((new.board.as_str(), new.point), ("", Some(2)));
    }

    #[test]
    fn a_path_git_quotes_is_found_by_what_it_says_and_a_plain_one_by_its_lines() {
        let quoted = "diff --git \"a/\\346\\227\\245.txt\" \"b/\\346\\227\\245.txt\"\n--- \"a/\\346\\227\\245.txt\"\n+++ \"b/\\346\\227\\245.txt\"\n@@ -1 +1 @@\n-a\n+b\n\
diff --git \"a/q\\\"x\\\\y\\tz\" \"b/q\\\"x\\\\y\\tz\"\nBinary files a and b differ\n\
diff --git a/with space.rs b/with space.rs\nBinary files a and b differ\n\
diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\n";
        assert!(diff_names_file(quoted, "日.txt"));
        assert!(diff_names_file(quoted, "q\"x\\y\tz"));
        assert!(diff_names_file(quoted, "with space.rs"));
        assert!(diff_names_file(quoted, "new.rs"));
        assert!(!diff_names_file(quoted, "\\346\\227\\245.txt"));
        assert!(!diff_names_file(quoted, "other.rs"));
        assert!(!diff_names_file(FILE_DIFF, "src/a.rs.bak"));
        assert!(diff_names_file(FILE_DIFF, "src/a.rs"));
    }

    #[test]
    fn a_header_with_a_multibyte_path_is_read_without_panicking() {
        for header in [
            "diff --git a/x b/日本語\n",
            "diff --git a/日本語 b/x\n",
            "diff --git a/日 b/日\n",
            "diff --git a/あ い b/あ い\n",
            "diff --git a/😀 b/😀😀\n",
        ] {
            let _ = diff_names_file(header, "x");
        }
        assert!(diff_names_file("diff --git a/日 b/日\nx\n", "日"));
        assert!(diff_names_file("diff --git a/あ い b/あ い\nx\n", "あ い"));
        assert!(diff_names_file("diff --git a/x b/日本語\n", "日本語"));
        assert!(!diff_names_file("diff --git a/x b/日本語\n", "y"));
    }
}
