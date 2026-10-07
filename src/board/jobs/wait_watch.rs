//! Telling the person that a session is waiting on them, when nobody is looking at it.
//!
//! A permission prompt or a question stops a worker or a hub until somebody answers in its
//! terminal, and the board shows it only to whoever has the board open. This watches the agent
//! session ledger on the board's own clock: a row that has said `waiting` for a few seconds is
//! announced once, through the configured `notification` and through the page's own desktop
//! notification (`/api/state` and `/api/boards` carry `waits`), unless that session's terminal
//! is open on the board.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::board::view::{board_sessions, socket_key};
use crate::board::{Server, Session, WaitNotice, settings_now, target_of};
use crate::infra::{clock::now_secs, notify, shell};

/// The pace of the sweep, as `sweep_gates`.
const EVERY: Duration = Duration::from_secs(2);

/// How long a row has to have been waiting before it is announced: a prompt answered at once,
/// or one the agent's own permission rules settle, is not worth an interruption.
pub const NOTIFY_AFTER_SECS: i64 = 5;

/// One wait: the ledger row's session id and when the row turned `waiting`. A row that waits
/// again after working has a new `since`, so it is a new wait.
type Key = (String, i64);

#[derive(Default)]
struct State {
    /// Whether the waits that were already there when this started have been taken in.
    seeded: bool,
    /// The waits that have been looked at, announced or not, so that none is looked at twice.
    handled: HashSet<Key>,
    /// The announced waits that go on, by the slug of the board that announced them.
    notices: HashMap<String, Vec<(Key, WaitNotice)>>,
}

/// What is known of the waits, shared by every board of the process.
#[derive(Default)]
pub struct WaitWatch {
    state: Mutex<State>,
    /// How many board terminals are open on each window, by `target_key`.
    open: Mutex<HashMap<String, usize>>,
}

/// Counts one open board terminal on a window, until it is dropped.
pub struct OpenGuard {
    watch: Arc<WaitWatch>,
    key: String,
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        let mut open = self.watch.open.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = open.get_mut(&self.key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                open.remove(&self.key);
            }
        }
    }
}

/// What names a tmux window across boards: the server it is on, and its id.
pub fn target_key(socket: Option<&str>, window: &str) -> String {
    format!("{}\t{window}", socket_key(socket).display())
}

/// A wait the sweep found worth announcing, and where its terminal would be.
#[derive(Debug, PartialEq, Eq)]
struct Picked {
    key: Key,
    notice: WaitNotice,
    /// `target_key` of the session's window, when it has one a terminal can be opened on.
    target: Option<String>,
}

/// The waits that have lasted `after` seconds and were not looked at yet.
fn due(waiting: &[Key], handled: &HashSet<Key>, now: i64, after: i64) -> Vec<Key> {
    waiting
        .iter()
        .filter(|key| !handled.contains(*key) && now - key.1 >= after)
        .cloned()
        .collect()
}

/// The sessions of one board that the `due` waits are about. A session counts when it runs, no
/// gate holds it (a gate has its own notice) and its row still says `waiting` from the same
/// moment. A hub counts only on its own board, which lists every hub of the repository.
fn pick(sessions: &[Session], due: &[Key], nwo: &str, slug: &str) -> Vec<Picked> {
    sessions
        .iter()
        .filter(|session| session.present && session.waiting.is_none())
        .filter(|session| {
            session.kind != "hub"
                || crate::kernel::identity::slug_for(nwo, session.key.as_deref()) == slug
        })
        .filter_map(|session| {
            let state = session.agent_session.as_ref()?;
            if state.status.as_deref() != Some("waiting") {
                return None;
            }
            let key = (state.session_id.clone()?, state.updated_at?);
            if !due.contains(&key) {
                return None;
            }
            let name = [&session.title, &session.task_title]
                .into_iter()
                .flatten()
                .map(|title| title.trim())
                .find(|title| !title.is_empty())
                .unwrap_or(&session.id)
                .to_string();
            Some(Picked {
                notice: WaitNotice {
                    agent_session_id: key.0.clone(),
                    since: key.1,
                    session: session.id.clone(),
                    kind: session.kind.clone(),
                    name,
                    request: state.request.clone(),
                },
                target: target_of(session)
                    .map(|(socket, window)| target_key(socket.as_deref(), &window)),
                key,
            })
        })
        .collect()
}

/// What `tool_summary` (transport/cli/hook.rs) makes of an `AskUserQuestion` request.
const QUESTION_TOOL: &str = "AskUserQuestion";

/// What the configured notification says.
fn message(name: &str, request: Option<&str>) -> String {
    let request = request.map(str::trim).filter(|request| !request.is_empty());
    match request {
        Some(QUESTION_TOOL) => format!("{name} is asking a question"),
        Some(request) => match request.strip_prefix("AskUserQuestion: ") {
            Some(question) => format!("{name} is asking: {question}"),
            None => format!("{name} is waiting: {request}"),
        },
        None => format!("{name} is waiting for input"),
    }
}

/// Where the claim on one wait is kept. Several processes can watch one ledger (a board per
/// hub when no resident server runs, or one started by hand beside it), and what each has
/// handled is its own, so the claim is a file they all see.
fn marker_path(root: &Path, key: &Key) -> std::path::PathBuf {
    let safe: String = key
        .0
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    root.join("wait-notified").join(format!("{safe}-{}", key.1))
}

/// How long a marker is kept if nothing removed it: a process that ended while its wait went on
/// leaves one behind.
const MARKER_KEPT: Duration = Duration::from_secs(86_400);

/// Remove the markers older than `kept`, by their modification time. Errors are ignored: a marker
/// that stays is a few bytes.
fn prune_old_markers(root: &Path, kept: Duration) {
    let Ok(entries) = std::fs::read_dir(root.join("wait-notified")) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| std::time::SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > kept);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Take the claim on a wait: true for the one process that makes the marker, false for any
/// that finds it. A marker that cannot be made for another reason is taken as claimed, since
/// a notification twice is better than none.
fn claim(root: &Path, key: &Key) -> bool {
    let path = marker_path(root, key);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::AlreadyExists,
    }
}

impl WaitWatch {
    /// The waits announced on board `slug` that still go on.
    pub fn notices(&self, slug: &str) -> Vec<WaitNotice> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .notices
            .get(slug)
            .map(|notices| notices.iter().map(|(_, notice)| notice.clone()).collect())
            .unwrap_or_default()
    }

    /// Note that a board terminal is open on the window `key` names, for as long as the guard
    /// lives.
    pub fn terminal_open(self: &Arc<Self>, key: &str) -> OpenGuard {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        *open.entry(key.to_string()).or_insert(0) += 1;
        OpenGuard {
            watch: Arc::clone(self),
            key: key.to_string(),
        }
    }

    fn is_open(&self, key: &str) -> bool {
        let open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        open.contains_key(key)
    }

    /// Watch for as long as the process lives. `root` is the ledger's state directory and
    /// `boards` is asked each round, so a board opened since is watched too.
    pub fn run(&self, root: &Path, boards: impl Fn() -> Vec<Arc<Server>>) {
        loop {
            // A round that panics is a failed round, not the end of the watch, as in
            // `sweep_gates`.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.round(root, &boards, now_secs());
            }));
            std::thread::sleep(EVERY);
        }
    }

    fn round(&self, root: &Path, boards: &impl Fn() -> Vec<Arc<Server>>, now: i64) {
        // A ledger that cannot be listed is a round of nothing: no row is taken to be gone.
        let Ok(waiting) = crate::registry::waiting_agent_sessions(root) else {
            return;
        };
        let due = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let still: HashSet<&Key> = waiting.iter().collect();
            // A wait that is over needs no claim: its moment never comes again.
            for key in state.handled.iter().filter(|key| !still.contains(key)) {
                let _ = std::fs::remove_file(marker_path(root, key));
            }
            state.handled.retain(|key| still.contains(key));
            for notices in state.notices.values_mut() {
                notices.retain(|(key, _)| still.contains(key));
            }
            if !state.seeded {
                prune_old_markers(root, MARKER_KEPT);
                // Waits that began before this did not wait for it: announcing them now would
                // ring for every prompt that is up when the board starts.
                state.seeded = true;
                state.handled.extend(waiting);
                return;
            }
            due(&waiting, &state.handled, now, NOTIFY_AFTER_SECS)
        };
        if due.is_empty() {
            return;
        }
        // Only now, when there is something to look for: opening the boards and listing their
        // sessions asks git and `ps`.
        let mut matched: HashSet<Key> = HashSet::new();
        let mut announced: Vec<(String, Picked)> = Vec::new();
        let mut rings: Vec<String> = Vec::new();
        for server in boards() {
            let left: Vec<Key> = due
                .iter()
                .filter(|key| !matched.contains(*key))
                .cloned()
                .collect();
            if left.is_empty() {
                break;
            }
            let repo = &server.ctx.repo;
            let settings = settings_now(&server);
            let sessions = board_sessions(&server, &settings);
            for picked in pick(&sessions, &left, &repo.nwo, &repo.slug) {
                matched.insert(picked.key.clone());
                // Claimed first, open terminal or not: only the process that makes the marker
                // may run the configured command, and one that sees the person at the
                // terminal keeps every other process from ringing it too.
                let mine = claim(root, &picked.key);
                // The person is at that terminal: they have seen it.
                if picked
                    .target
                    .as_deref()
                    .is_some_and(|key| self.is_open(key))
                {
                    continue;
                }
                // The page's notice is this board's own, whoever wins the claim: a browser
                // served by any process still gets its desktop notification.
                if mine
                    && let Some(command) = notify::repo_command(
                        &settings.notification,
                        &repo.nwo,
                        &repo.repo,
                        &message(&picked.notice.name, picked.notice.request.as_deref()),
                    )
                {
                    rings.push(command);
                }
                announced.push((repo.slug.clone(), picked));
            }
        }
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            // Every due wait, matched or not: one that belongs to no session this process
            // lists is not looked for again, and one that is found is announced once.
            state.handled.extend(due);
            for (slug, picked) in announced {
                state
                    .notices
                    .entry(slug)
                    .or_default()
                    .push((picked.key, picked.notice));
            }
        }
        // Outside the lock, each on a thread of its own, and the result is not looked at: a
        // notifier that fails or hangs must not stop the watch or the board showing the wait.
        for command in rings {
            std::thread::spawn(move || {
                let _ = shell::run_shell(&command);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::SessionAgentState;
    use crate::infra::terminal::SessionTerminal;

    fn key(id: &str, since: i64) -> Key {
        (id.to_string(), since)
    }

    fn session(id: &str, kind: &str, row: &str, since: i64, request: Option<&str>) -> Session {
        Session {
            id: id.to_string(),
            kind: kind.to_string(),
            agent: "claude".to_string(),
            terminal: SessionTerminal {
                backend: "tmux".to_string(),
                socket: None,
                session: None,
                window: Some("@3".to_string()),
                pane: None,
            },
            hub: None,
            key: None,
            worktree: "/w".to_string(),
            branch: None,
            task: None,
            title: None,
            task_title: None,
            conversation: None,
            present: true,
            stale: false,
            pid: None,
            started_at: None,
            phase: None,
            phase_at: None,
            phases: Vec::new(),
            last_activity_at: None,
            last_line: None,
            attached: None,
            waiting: None,
            agent_session: Some(SessionAgentState {
                session_id: Some(row.to_string()),
                status: Some("waiting".to_string()),
                pending: None,
                updated_at: Some(since),
                last_event_at: Some(since),
                activity: None,
                request: request.map(str::to_string),
                subagents: 0,
                error: None,
            }),
        }
    }

    #[test]
    fn a_wait_is_due_after_a_few_seconds_and_only_once() {
        let waiting = vec![key("a", 100), key("b", 98), key("c", 90)];
        let handled: HashSet<Key> = [key("c", 90)].into();
        assert_eq!(due(&waiting, &handled, 102, 5), Vec::<Key>::new());
        assert_eq!(due(&waiting, &handled, 103, 5), vec![key("b", 98)]);
        assert_eq!(
            due(&waiting, &handled, 105, 5),
            vec![key("a", 100), key("b", 98)]
        );
        // The same row waiting again is a new wait.
        assert_eq!(due(&[key("c", 120)], &handled, 125, 5), vec![key("c", 120)]);
    }

    #[test]
    fn a_wait_is_picked_by_its_row_and_moment() {
        let sessions = vec![
            session("worker-a", "worker", "row-a", 100, Some("Bash: make")),
            session("worker-b", "worker", "row-b", 100, None),
        ];
        let picked = pick(&sessions, &[key("row-a", 100)], "o/r", "o-r");
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].notice.session, "worker-a");
        assert_eq!(picked[0].notice.name, "worker-a");
        assert_eq!(picked[0].notice.request.as_deref(), Some("Bash: make"));
        assert_eq!(picked[0].target, Some(target_key(None, "@3")));
        // The row waits again, from another moment: not this wait.
        assert!(pick(&sessions, &[key("row-a", 101)], "o/r", "o-r").is_empty());
    }

    #[test]
    fn a_session_that_is_gone_gated_or_no_longer_waiting_is_not_picked() {
        let due = [key("row", 100)];
        let mut gone = session("worker-a", "worker", "row", 100, None);
        gone.present = false;
        let mut gated = session("worker-a", "worker", "row", 100, None);
        gated.waiting = Some(crate::board::SessionWaiting {
            id: "g".to_string(),
            kind: "question".to_string(),
            hub: "hub".to_string(),
            slug: "o-r".to_string(),
            title: None,
            opened_at: "20260101T000000Z".to_string(),
            count: 1,
            options: Vec::new(),
            choices: Vec::new(),
            focus: None,
        });
        let mut moved = session("worker-a", "worker", "row", 100, None);
        moved.agent_session.as_mut().unwrap().status = Some("running".to_string());
        for session in [gone, gated, moved] {
            assert!(pick(&[session], &due, "o/r", "o-r").is_empty());
        }
    }

    #[test]
    fn a_hub_counts_only_on_its_own_board() {
        let own = crate::kernel::identity::slug_for("o/r", None);
        let parent = crate::kernel::identity::slug_for("o/r", Some("ALPHA-1"));
        let mut hub = session("hub", "hub", "row", 100, None);
        hub.title = Some("hub-o-r".to_string());
        let mut parent_hub = session("hub-ALPHA-1", "hub", "row2", 100, None);
        parent_hub.key = Some("ALPHA-1".to_string());
        let due = [key("row", 100), key("row2", 100)];
        let on = |slug: &str| -> Vec<String> {
            pick(&[hub.clone(), parent_hub.clone()], &due, "o/r", slug)
                .into_iter()
                .map(|p| p.notice.session)
                .collect()
        };
        assert_eq!(on(&own), vec!["hub"]);
        assert_eq!(on(&parent), vec!["hub-ALPHA-1"]);
        assert_eq!(
            pick(&[hub.clone()], &due, "o/r", &own)[0].notice.name,
            "hub-o-r"
        );
    }

    #[test]
    fn the_message_says_who_waits_and_on_what() {
        assert_eq!(
            message("worker-a", Some("Bash: make")),
            "worker-a is waiting: Bash: make"
        );
        assert_eq!(message("worker-a", None), "worker-a is waiting for input");
        assert_eq!(
            message("worker-a", Some("AskUserQuestion: Which fruit?")),
            "worker-a is asking: Which fruit?"
        );
        assert_eq!(
            message("worker-a", Some("AskUserQuestion")),
            "worker-a is asking a question"
        );
        assert_eq!(
            message("worker-a", Some("  ")),
            "worker-a is waiting for input"
        );
    }

    #[test]
    fn a_wait_is_claimed_by_one_process_only() {
        let dir = tempfile::tempdir().unwrap();
        let wait = key("row/../x", 100);
        assert!(claim(dir.path(), &wait));
        assert!(!claim(dir.path(), &wait));
        // Another moment of the same row is another wait.
        assert!(claim(dir.path(), &key("row/../x", 101)));
        assert!(marker_path(dir.path(), &wait).starts_with(dir.path().join("wait-notified")));
    }

    #[test]
    fn markers_older_than_a_day_go_and_newer_ones_stay() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (key("old", 1), key("new", 2));
        assert!(claim(dir.path(), &old) && claim(dir.path(), &new));
        let file = std::fs::File::options()
            .write(true)
            .open(marker_path(dir.path(), &old))
            .unwrap();
        file.set_modified(std::time::SystemTime::now() - Duration::from_secs(2 * 86_400))
            .unwrap();
        prune_old_markers(dir.path(), MARKER_KEPT);
        assert!(!marker_path(dir.path(), &old).exists());
        assert!(marker_path(dir.path(), &new).exists());
        // No directory is nothing to do.
        prune_old_markers(&dir.path().join("none"), MARKER_KEPT);
    }

    #[test]
    fn an_open_terminal_is_counted_until_its_guard_is_dropped() {
        let watch = Arc::new(WaitWatch::default());
        let k = target_key(None, "@3");
        assert!(!watch.is_open(&k));
        let first = watch.terminal_open(&k);
        let second = watch.terminal_open(&k);
        drop(first);
        assert!(watch.is_open(&k));
        drop(second);
        assert!(!watch.is_open(&k));
    }
}
