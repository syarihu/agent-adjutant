//! Telling the person that a session is waiting on them, or that a gate opened, when nobody is
//! looking at it.
//!
//! A permission prompt or a question stops a worker or a hub until somebody answers in its
//! terminal, and the board shows it only to whoever has the board open. This watches the agent
//! session ledger on the board's own clock: a row that has said `waiting` for a few seconds is
//! announced once, unless that session's terminal is open on the board. One channel rings per
//! event: the client's own desktop notification while the desktop app or a board page that may
//! notify is open, the notifier otherwise. With `notification` unset, the target is the app if one
//! is open on any server process (`registry::notify_clients`), else a page, else none. It rings
//! the wait itself when it polls this process, a notice is queued for it when it polls another
//! one (that process does not show this board), and nothing rings when its 確認待ち is off; with
//! neither open the notifier is `terminal-notifier` if it is installed; with a command configured it is a page of this process (see `page_seen`) and the
//! notifier is that command. A wait is not claimed while a client is open; if it lapses and the
//! session still waits, the notifier rings it then, a late reminder rather than silence. A gate that opens is rung the same way
//! (`gates.rs`). `/api/state` and `/api/boards` carry every such wait as `waits`, which the page
//! lists in 「いまの仕事」 on a resident server and marks on the session's card on a board served
//! alone. A wait that was not announced (it was already up when the watch started, or its
//! terminal was open) is listed all the same, marked `quiet`. A session held by a gate has the
//! gate's own notice, so its wait is not listed while the gate is open; it is looked at again
//! every `HELD_RECHECK_SECS`, and listed once the gate is gone and the row still waits.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::board::view::{board_sessions, socket_key};
use crate::board::{Server, Session, WaitNotice, settings_now, target_of};
use crate::infra::terminal::Hook;
use crate::infra::{clock::now_secs, notify, shell};
use crate::registry::{WaitRoute, fresh, local_notify_clients, notify_clients, wait_route};

mod gates;

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
    /// The waits that have been looked at, listed or not, so that none is looked at twice.
    handled: HashSet<Key>,
    /// The waits that were already up when the watch started: listed, never announced.
    quiet: HashSet<Key>,
    /// The waits found held by a gate, and when each was last seen so: not handled, so that the
    /// wait is listed once the gate closes.
    held: HashMap<Key, i64>,
    /// The listed waits that go on, by the slug of the board that listed them.
    notices: HashMap<String, Vec<(Key, WaitNotice)>>,
    /// The waits listed but left to the page's own notification while it is fresh: neither
    /// claimed nor rung, and not looked for again until the page lapses.
    page_held: HashSet<Key>,
    /// The same for a wait left to the desktop app or a board page of any server process, which
    /// is what `notification` unset leaves it to (`registry::notify_clients`).
    client_held: HashSet<Key>,
    /// What the gate sweep has looked at.
    gates: gates::GateState,
}

/// What is known of the waits, shared by every board of the process.
#[derive(Default)]
pub struct WaitWatch {
    state: Mutex<State>,
    /// How many board terminals are open on each window, by `target_key`.
    open: Mutex<HashMap<String, usize>>,
    /// When a page that may notify was last seen, or `0`.
    page_at: AtomicI64,
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

/// How long a wait found held by a gate is left alone before it is looked at again, so that a
/// gate open for hours does not have the boards listed every round.
const HELD_RECHECK_SECS: i64 = 10;

/// The waits that have lasted `after` seconds and were not looked at yet, none of `skip` (the ones
/// left to a fresh page). A held one is looked at again once `HELD_RECHECK_SECS` have passed
/// since it was last seen held.
fn due(
    waiting: &[Key],
    handled: &HashSet<Key>,
    held: &HashMap<Key, i64>,
    skip: &HashSet<Key>,
    now: i64,
    after: i64,
) -> Vec<Key> {
    waiting
        .iter()
        .filter(|key| {
            !handled.contains(*key)
                && !skip.contains(*key)
                && now - key.1 >= after
                && held
                    .get(*key)
                    .is_none_or(|at| now - at >= HELD_RECHECK_SECS)
        })
        .cloned()
        .collect()
}

/// The `due` waits whose session runs and waits from the same moment but is held by a gate: not
/// listed yet, and not to be settled, since the gate may close while the row goes on waiting.
fn held_by_gate(sessions: &[Session], due: &[Key]) -> Vec<Key> {
    sessions
        .iter()
        .filter(|session| session.present && session.waiting.is_some())
        .filter_map(|session| {
            let state = session.agent_session.as_ref()?;
            if state.status.as_deref() != Some("waiting") {
                return None;
            }
            let key = (state.session_id.clone()?, state.updated_at?);
            due.contains(&key).then_some(key)
        })
        .collect()
}

/// The sessions of one board that the `due` waits are about. A session counts when it runs, no
/// gate holds it (a gate has its own notice, and `held_by_gate` keeps the wait for later) and
/// its row still says `waiting` from the same moment. A hub counts only on its own board, which
/// lists every hub of the repository.
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
                    quiet: false,
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

/// How long a wait no session is found for is looked for again while a listing keeps failing,
/// so a board that never opens does not have every board listed every round for good.
const RETRY_SECS: i64 = 60;

/// The due waits that are done with this round, none of those `held` by a gate: the ones a
/// session was found for, and, when every board and session was listed, the others too. A
/// listing that failed is not an answer that nothing waits there, until the wait is older than
/// `RETRY_SECS`.
fn settled(
    due: &[Key],
    matched: &HashSet<Key>,
    held: &HashSet<Key>,
    complete: bool,
    now: i64,
) -> Vec<Key> {
    due.iter()
        .filter(|key| !held.contains(*key))
        .filter(|key| complete || matched.contains(*key) || now - key.1 > RETRY_SECS)
        .cloned()
        .collect()
}

/// Who rings a wait.
#[derive(Debug, PartialEq, Eq)]
enum Ring {
    /// Nobody: quiet, or another process has the claim.
    No,
    /// The configured notification, now.
    Now,
    /// A notice in the shared queue, for the app or page that is open on another process and so
    /// cannot see this board: `notification` unset only.
    Queue,
    /// The page, while it is fresh: left unclaimed, so that the configured notification takes
    /// it if the page lapses first.
    Later,
}

/// What to do with a wait a session was found for: who rings it, and whether it is listed as
/// `quiet`. A wait that was up when the watch started is listed and never rings, and takes no
/// claim, so that the process that did see it begin keeps the marker. One whose terminal is open
/// was seen by the person, but still holds the claim so no other process rings it. While a page
/// that may notify is open (`page`) the wait is left to it and takes no claim. `claim` is asked
/// at most once.
fn route(seeded: bool, open: bool, page: bool, claim: impl FnOnce() -> bool) -> (Ring, bool) {
    if seeded {
        return (Ring::No, true);
    }
    if open {
        claim();
        return (Ring::No, true);
    }
    if page {
        return (Ring::Later, false);
    }
    if claim() {
        (Ring::Now, false)
    } else {
        (Ring::No, false)
    }
}

/// What the page rings a wait under (`notifyWaitTag` in my-work-notify.js: `wait-<ledger row
/// id>/<since>`). It names the wait and not who observed it: one worker's wait can be seen from
/// its repository's board and from a parent hub's, and a notice queued for it must be the same
/// event the page rings from either. The row id is the agent's session id, which the markers
/// already take as unique across boards.
fn wait_tag(key: &Key) -> String {
    format!("wait-{}/{}", key.0, key.1)
}

/// Whether a notice was queued. When it was not, this process's claim `marker` is removed, so
/// that the wait is not taken for rung and another round (or process) can ring it.
fn queued_or_released(queued: Result<String, String>, marker: &Path) -> bool {
    if queued.is_ok() {
        return true;
    }
    let _ = std::fs::remove_file(marker);
    false
}

/// What a wait the claim said to ring is, with `notification` unset: the claim only settles that
/// this process is the one to act on it, and the route says how. The target client reads another
/// process (`Queue`), has 確認待ち off (`Drop`), or nobody is open (`Notifier`, the command).
fn built_in_ring(ring: Ring, route: WaitRoute) -> Ring {
    match (ring, route) {
        (Ring::Now, WaitRoute::Queue(_)) => Ring::Queue,
        (Ring::Now, WaitRoute::Drop) => Ring::No,
        (ring, _) => ring,
    }
}

/// Put `notice` in `list`, in place of the one already there for `key`: a wait left to the page
/// is picked again once the page lapses.
fn list_notice(list: &mut Vec<(Key, WaitNotice)>, key: Key, notice: WaitNotice) {
    match list.iter_mut().find(|(k, _)| *k == key) {
        Some(entry) => entry.1 = notice,
        None => list.push((key, notice)),
    }
}

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

/// A name made of characters that are safe in a file name.
fn safe_name(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Where the claim on one wait is kept. Several processes can watch one ledger (a board per
/// hub when no resident server runs, or one started by hand beside it), and what each has
/// handled is its own, so the claim is a file they all see.
fn marker_path(root: &Path, key: &Key) -> std::path::PathBuf {
    root.join("wait-notified")
        .join(format!("{}-{}", safe_name(&key.0), key.1))
}

/// How long a marker is kept. The sweep is the only thing that removes one: a wait is the pair
/// of row id and `updatedAt`, so a marker is never claimed by a later wait.
const MARKER_KEPT: Duration = Duration::from_secs(86_400);

/// Remove the markers in `dir` older than `kept`, by their modification time. Errors are ignored:
/// a marker that stays is a few bytes.
fn prune_old_markers(dir: &Path, kept: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
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

/// Take the claim on `path`: true for the one process that makes the marker, false for any
/// that finds it. A marker that cannot be made for another reason is taken as claimed, since
/// a notification twice is better than none.
fn claim(path: &Path) -> bool {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::AlreadyExists,
    }
}

/// Run each command on a thread of its own, and do not look at the result: a notifier that
/// fails or hangs must not stop the watch or the board showing the wait.
/// The commands that ring the notices no desktop app or board page took in time (a window that
/// closed, or a server that stopped, between the notice being queued and being taken), each
/// once, with the built-in notifier: `terminal-notifier` when it is installed, nothing when it
/// is not. Several servers run this, and taking a notice is a rename, so one of them gets it.
fn expired_notice_commands(
    root: &Path,
    now: i64,
    built_in: &dyn Fn(&str, &str) -> Option<String>,
) -> Vec<String> {
    crate::registry::take_expired_notices(root, now)
        .iter()
        .filter_map(|notice| built_in(&notice.title, &notice.message))
        .collect()
}

fn spawn_all(commands: Vec<String>) {
    for command in commands {
        std::thread::spawn(move || {
            let _ = shell::run_shell(&command);
        });
    }
}

impl WaitWatch {
    /// The waits on board `slug` that still go on, announced or `quiet`.
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

    /// Note a poll of a page of this process that may show desktop notifications for waits and
    /// gates. A page that may not is not noted and does not clear what another page noted: two
    /// browsers can differ, and one that cannot ring must not make the server ring beside the
    /// one that can. The configured notification takes over once the last such poll is older
    /// than `PAGE_FRESH_SECS`. This is for a configured `notification`; with it unset the
    /// answer is every server process's, read from `registry::notify_clients`.
    pub fn page_seen(&self, now: i64) {
        self.page_at.fetch_max(now, Ordering::Relaxed);
    }

    /// Whether a page that may notify is open: it polled within `PAGE_FRESH_SECS`.
    fn page_fresh(&self, now: i64) -> bool {
        fresh(self.page_at.load(Ordering::Relaxed), now)
    }

    /// Whether the wait or gate of a board whose `notification` is `hook` is left to a client
    /// that is open. With `notification` unset that is the target client (the app, else a page)
    /// when it polls this process (`route`, read once for the round); a configured command, or
    /// `false`, is left only to a page of this process, as it always was.
    fn left_to_client(&self, hook: &Hook, route: WaitRoute, now: i64) -> bool {
        if matches!(hook, Hook::BuiltIn) {
            route == WaitRoute::Leave
        } else {
            self.page_fresh(now)
        }
    }

    /// Watch for as long as the process lives. `root` is the ledger's state directory, `slugs`
    /// names the hubs whose gates are watched, and `boards` is asked each round, so a board
    /// opened since is watched too. It answers with the boards and whether it has them all: one
    /// it could not open is not the same as none.
    pub fn run(
        &self,
        root: &Path,
        slugs: impl Fn() -> Vec<String>,
        boards: impl Fn() -> (Vec<Arc<Server>>, bool),
    ) {
        loop {
            // A round that panics is a failed round, not the end of the watch, as in
            // `sweep_gates`.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let now = now_secs();
                spawn_all(self.gate_round(root, &slugs(), &boards, now));
                self.round(root, &boards, now);
                spawn_all(expired_notice_commands(root, now, &notify::default_command));
            }));
            std::thread::sleep(EVERY);
        }
    }

    fn round(&self, root: &Path, boards: &impl Fn() -> (Vec<Arc<Server>>, bool), now: i64) {
        // A ledger that cannot be listed is a round of nothing: no row is taken to be gone.
        let Ok(waiting) = crate::registry::waiting_agent_sessions(root) else {
            return;
        };
        let page = self.page_fresh(now);
        let wait_at = wait_route(notify_clients(root), local_notify_clients(root), now);
        let (due, quiet) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let still: HashSet<&Key> = waiting.iter().collect();
            state.handled.retain(|key| still.contains(key));
            state.quiet.retain(|key| still.contains(key));
            state.held.retain(|key, _| still.contains(key));
            state.page_held.retain(|key| still.contains(key));
            state.client_held.retain(|key| still.contains(key));
            for notices in state.notices.values_mut() {
                notices.retain(|(key, _)| still.contains(key));
            }
            if !state.seeded {
                prune_old_markers(&root.join("wait-notified"), MARKER_KEPT);
                prune_old_markers(&root.join("gate-notified"), MARKER_KEPT);
                // Waits that began before this did not wait for it: announcing them now would
                // ring for every prompt that is up when the board starts. They are still
                // looked up below, to be listed.
                state.seeded = true;
                state.quiet.extend(waiting.iter().cloned());
            }
            // The waits left to the page are not looked for again while it stays fresh, so that
            // they do not have the boards listed every round.
            let mut skip = HashSet::new();
            if page {
                skip.extend(state.page_held.iter().cloned());
            }
            if wait_at == WaitRoute::Leave {
                skip.extend(state.client_held.iter().cloned());
            }
            (
                due(
                    &waiting,
                    &state.handled,
                    &state.held,
                    &skip,
                    now,
                    NOTIFY_AFTER_SECS,
                ),
                state.quiet.clone(),
            )
        };
        if due.is_empty() {
            return;
        }
        // Only now, when there is something to look for: opening the boards and listing their
        // sessions asks git and `ps`.
        let mut matched: HashSet<Key> = HashSet::new();
        let mut held: HashSet<Key> = HashSet::new();
        let mut later: HashSet<Key> = HashSet::new();
        let mut later_client: HashSet<Key> = HashSet::new();
        let mut retry: HashSet<Key> = HashSet::new();
        let mut announced: Vec<(String, Picked)> = Vec::new();
        let mut rings: Vec<String> = Vec::new();
        let (servers, mut complete) = boards();
        for server in servers {
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
            let (sessions, listed) = board_sessions(&server, &settings);
            // A session whose ledger row could not be read looks like one that does not wait.
            complete &= listed
                && !sessions
                    .iter()
                    .any(|s| s.agent_session.as_ref().is_some_and(|a| a.error.is_some()));
            held.extend(held_by_gate(&sessions, &left));
            for picked in pick(&sessions, &left, &repo.nwo, &repo.slug) {
                matched.insert(picked.key.clone());
                // Claimed first, open terminal or not: only the process that makes the marker
                // may run the configured command, and one that sees the person at the
                // terminal keeps every other process from ringing it too.
                let open = picked
                    .target
                    .as_deref()
                    .is_some_and(|key| self.is_open(key));
                let built_in = matches!(settings.notification, Hook::BuiltIn);
                let left = self.left_to_client(&settings.notification, wait_at, now);
                let (ring, is_quiet) = self::route(quiet.contains(&picked.key), open, left, || {
                    claim(&marker_path(root, &picked.key))
                });
                let ring = if built_in {
                    built_in_ring(ring, wait_at)
                } else {
                    ring
                };
                let mut picked = picked;
                picked.notice.quiet = is_quiet;
                // The page's notice is this board's own, whoever wins the claim: a browser
                // served by any process still gets its desktop notification, and it is the
                // only one that rings for a wait left to it.
                match ring {
                    Ring::Now => {
                        if let Some(command) = notify::repo_command(
                            &settings.notification,
                            &repo.nwo,
                            &repo.repo,
                            &message(&picked.notice.name, picked.notice.request.as_deref()),
                        ) {
                            rings.push(command);
                        }
                    }
                    Ring::Queue => {
                        // Claimed above, so queued once: the client that takes it rings it. A
                        // notice that could not be queued gives the claim back and leaves the
                        // wait unsettled, so that a later round tries again.
                        let queued = crate::registry::enqueue_notice_tagged(
                            root,
                            &repo.nwo,
                            &notify::repo_title(&repo.repo),
                            &message(&picked.notice.name, picked.notice.request.as_deref()),
                            now,
                            Some(&wait_tag(&picked.key)),
                        );
                        if !queued_or_released(queued, &marker_path(root, &picked.key)) {
                            retry.insert(picked.key.clone());
                        }
                    }
                    Ring::Later => {
                        if built_in {
                            later_client.insert(picked.key.clone());
                        } else {
                            later.insert(picked.key.clone());
                        }
                    }
                    Ring::No => {}
                }
                announced.push((repo.slug.clone(), picked));
            }
        }
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            // The waits that were found, and, after a complete sweep, the rest of the due ones:
            // one that belongs to no session this process lists is not looked for again. After
            // an incomplete one they are tried again next round. A wait left to the page is not
            // done with: it is looked at again once the page lapses.
            let unsettled: HashSet<Key> = held
                .iter()
                .chain(&later)
                .chain(&later_client)
                .chain(&retry)
                .cloned()
                .collect();
            state
                .handled
                .extend(settled(&due, &matched, &unsettled, complete, now));
            for key in &matched {
                state.held.remove(key);
                if !later.contains(key) {
                    state.page_held.remove(key);
                }
                if !later_client.contains(key) {
                    state.client_held.remove(key);
                }
            }
            state.page_held.extend(later);
            state.client_held.extend(later_client);
            for key in held.difference(&matched) {
                state.held.insert(key.clone(), now);
            }
            for (slug, picked) in announced {
                list_notice(
                    state.notices.entry(slug).or_default(),
                    picked.key,
                    picked.notice,
                );
            }
        }
        // Outside the lock.
        spawn_all(rings);
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
            prompt: None,
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
                model: None,
                context_percent: None,
                last_message: None,
                last_message_at: None,
                last_prompt_at: None,
                subagents: Vec::new(),
                error: None,
            }),
            uncommitted: None,
            uncommitted_error: None,
            unpushed: None,
            cleanup_kept_at: None,
            branch_pr: None,
            branch_pr_error: None,
        }
    }

    #[test]
    fn a_wait_is_due_after_a_few_seconds_and_only_once() {
        let waiting = vec![key("a", 100), key("b", 98), key("c", 90)];
        let handled: HashSet<Key> = [key("c", 90)].into();
        assert_eq!(
            due(&waiting, &handled, &HashMap::new(), &HashSet::new(), 102, 5),
            Vec::<Key>::new()
        );
        assert_eq!(
            due(&waiting, &handled, &HashMap::new(), &HashSet::new(), 103, 5),
            vec![key("b", 98)]
        );
        assert_eq!(
            due(&waiting, &handled, &HashMap::new(), &HashSet::new(), 105, 5),
            vec![key("a", 100), key("b", 98)]
        );
        // The same row waiting again is a new wait.
        assert_eq!(
            due(
                &[key("c", 120)],
                &handled,
                &HashMap::new(),
                &HashSet::new(),
                125,
                5
            ),
            vec![key("c", 120)]
        );
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
    fn an_incomplete_listing_leaves_the_unmatched_waits_to_the_next_round() {
        let due = [key("a", 1), key("b", 2)];
        let matched: HashSet<Key> = [key("a", 1)].into();
        assert_eq!(
            settled(&due, &matched, &HashSet::new(), true, 10),
            vec![key("a", 1), key("b", 2)]
        );
        assert_eq!(
            settled(&due, &matched, &HashSet::new(), false, 10),
            vec![key("a", 1)]
        );
        assert!(settled(&due, &HashSet::new(), &HashSet::new(), false, 10).is_empty());
        // Not for good: past `RETRY_SECS` an unmatched wait is let go even then.
        assert_eq!(
            settled(&due, &matched, &HashSet::new(), false, 2 + RETRY_SECS + 1),
            vec![key("a", 1), key("b", 2)]
        );
    }

    #[test]
    fn a_wait_is_rung_by_one_channel_and_listed_quietly_when_it_was_there_first_or_seen() {
        let mut asked = 0;
        // Normal: claimed, rung, announced.
        assert_eq!(
            route(false, false, false, || {
                asked += 1;
                true
            }),
            (Ring::Now, false)
        );
        // Another process holds the claim: listed, not rung, not quiet.
        assert_eq!(route(false, false, false, || false), (Ring::No, false));
        // The person is at the terminal: claimed so nobody else rings, listed quiet.
        assert_eq!(route(false, true, false, || true), (Ring::No, true));
        // Up before the watch started: listed quiet, and the claim is never asked.
        assert_eq!(
            route(true, false, false, || {
                asked += 1;
                true
            }),
            (Ring::No, true)
        );
        assert_eq!(asked, 1);
        // A page that may notify is open: left to it, no claim taken.
        assert_eq!(
            route(false, false, true, || panic!("claimed")),
            (Ring::Later, false)
        );
        // The terminal open beats the page, and still holds the claim.
        let mut claimed = false;
        assert_eq!(
            route(false, true, true, || {
                claimed = true;
                true
            }),
            (Ring::No, true)
        );
        assert!(claimed);
        // Up before the watch started beats the page.
        assert_eq!(
            route(true, false, true, || panic!("claimed")),
            (Ring::No, true)
        );
    }

    #[test]
    fn a_page_is_open_for_ninety_seconds_after_it_was_seen() {
        assert_eq!(crate::registry::PAGE_FRESH_SECS, 90);
        let watch = WaitWatch::default();
        assert!(!watch.page_fresh(100));
        watch.page_seen(100);
        assert!(watch.page_fresh(150));
        // An older poll that lands late does not move it back.
        watch.page_seen(90);
        assert!(watch.page_fresh(190));
        assert!(!watch.page_fresh(191));
    }

    #[test]
    fn only_a_configured_command_is_left_to_this_process_s_page() {
        let watch = WaitWatch::default();
        watch.page_seen(100);
        let command = Hook::Command("say".into());
        // Unset: left only when the target client polls this process.
        assert!(watch.left_to_client(&Hook::BuiltIn, WaitRoute::Leave, 150));
        assert!(!watch.left_to_client(&Hook::BuiltIn, WaitRoute::Notifier, 150));
        assert!(!watch.left_to_client(
            &Hook::BuiltIn,
            WaitRoute::Queue(crate::registry::Client::App),
            150
        ));
        assert!(!watch.left_to_client(&Hook::BuiltIn, WaitRoute::Drop, 150));
        assert!(watch.left_to_client(&command, WaitRoute::Notifier, 150));
        assert!(!watch.left_to_client(&command, WaitRoute::Leave, 191));
        assert!(!watch.left_to_client(&Hook::Off, WaitRoute::Leave, 191));
    }

    #[test]
    fn a_notice_that_could_not_be_queued_gives_the_claim_back() {
        let dir = tempfile::tempdir().unwrap();
        let marker = marker_path(dir.path(), &key("row-1", 1700));
        assert!(claim(&marker));
        assert!(queued_or_released(Ok("id".into()), &marker));
        assert!(marker.exists(), "a queued notice keeps its claim");
        assert!(!queued_or_released(Err("disk full".into()), &marker));
        assert!(!marker.exists());
        // Released: the next round's claim succeeds again.
        assert!(claim(&marker));
        // Nothing to remove is not an error.
        assert!(!queued_or_released(
            Err("x".into()),
            &dir.path().join("absent")
        ));
    }

    #[test]
    fn a_queued_wait_is_named_as_the_page_names_its_own() {
        assert_eq!(wait_tag(&key("row-1", 1700)), "wait-row-1/1700");
    }

    #[test]
    fn a_claimed_wait_is_queued_dropped_or_rung_by_the_route() {
        use crate::registry::Client;
        assert_eq!(
            built_in_ring(Ring::Now, WaitRoute::Queue(Client::Page)),
            Ring::Queue
        );
        assert_eq!(built_in_ring(Ring::Now, WaitRoute::Drop), Ring::No);
        assert_eq!(built_in_ring(Ring::Now, WaitRoute::Notifier), Ring::Now);
        // A claim lost to another process stays lost.
        assert_eq!(
            built_in_ring(Ring::No, WaitRoute::Queue(Client::App)),
            Ring::No
        );
        assert_eq!(built_in_ring(Ring::Later, WaitRoute::Leave), Ring::Later);
    }

    #[test]
    fn a_notice_nobody_took_is_rung_by_the_notifier_once() {
        let dir = tempfile::tempdir().unwrap();
        crate::registry::enqueue_notice(dir.path(), "o/r", "adjutant / r", "done", 100).unwrap();
        let notifier = |title: &str, message: &str| Some(format!("tn {title} {message}"));
        let at = 100 + crate::registry::NOTICE_EXPIRY_SECS;
        assert!(expired_notice_commands(dir.path(), at - 1, &notifier).is_empty());
        assert_eq!(
            expired_notice_commands(dir.path(), at, &notifier),
            ["tn adjutant / r done"]
        );
        assert!(expired_notice_commands(dir.path(), at, &notifier).is_empty());
        // Without terminal-notifier it is dropped, not kept for ever.
        crate::registry::enqueue_notice(dir.path(), "o/r", "t", "m", 100).unwrap();
        assert!(expired_notice_commands(dir.path(), at, &|_, _| None).is_empty());
        assert!(crate::registry::take_expired_notices(dir.path(), at).is_empty());
    }

    #[test]
    fn a_wait_left_to_the_page_is_not_due_until_the_page_lapses() {
        let wait = key("row", 100);
        let waiting = [wait.clone()];
        let skip: HashSet<Key> = [wait.clone()].into();
        let (handled, held) = (HashSet::new(), HashMap::new());
        assert!(due(&waiting, &handled, &held, &skip, 200, 5).is_empty());
        assert_eq!(
            due(&waiting, &handled, &held, &HashSet::new(), 200, 5),
            vec![wait]
        );
    }

    #[test]
    fn a_wait_left_to_the_page_is_not_settled_even_after_a_complete_listing() {
        let due = [key("a", 1), key("b", 2)];
        let matched: HashSet<Key> = [key("a", 1), key("b", 2)].into();
        let later: HashSet<Key> = [key("b", 2)].into();
        assert_eq!(settled(&due, &matched, &later, true, 10), vec![key("a", 1)]);
    }

    #[test]
    fn a_wait_listed_again_replaces_its_notice() {
        let notice = |quiet| WaitNotice {
            agent_session_id: "row".to_string(),
            since: 100,
            session: "s".to_string(),
            kind: "worker".to_string(),
            name: "n".to_string(),
            request: None,
            quiet,
        };
        let mut list = Vec::new();
        list_notice(&mut list, key("row", 100), notice(false));
        list_notice(&mut list, key("row", 100), notice(true));
        list_notice(&mut list, key("row", 101), notice(false));
        assert_eq!(list.len(), 2);
        assert!(list[0].1.quiet);
    }

    #[test]
    fn a_wait_held_by_a_gate_is_kept_for_later_and_listed_once_the_gate_is_gone() {
        let gated = |since| {
            let mut s = session("worker-a", "worker", "row", since, None);
            s.waiting = Some(crate::board::SessionWaiting {
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
            s
        };
        let wait = key("row", 100);
        let due_now = [wait.clone()];
        // Found held, not picked, and not settled even after a complete listing.
        let held: HashSet<Key> = held_by_gate(&[gated(100)], &due_now).into_iter().collect();
        assert_eq!(held, [wait.clone()].into());
        assert!(pick(&[gated(100)], &due_now, "o/r", "o-r").is_empty());
        assert!(settled(&due_now, &HashSet::new(), &held, true, 200).is_empty());
        // A row that moved on, or a session that is gone, is not held.
        assert!(held_by_gate(&[gated(101)], &due_now).is_empty());
        // Left alone until HELD_RECHECK_SECS have passed since it was seen held.
        let seen: HashMap<Key, i64> = [(wait.clone(), 110)].into();
        let none = HashSet::new();
        assert!(
            due(
                std::slice::from_ref(&wait),
                &none,
                &seen,
                &none,
                110 + HELD_RECHECK_SECS - 1,
                5
            )
            .is_empty()
        );
        assert_eq!(
            due(
                std::slice::from_ref(&wait),
                &none,
                &seen,
                &none,
                110 + HELD_RECHECK_SECS,
                5
            ),
            vec![wait.clone()]
        );
        // The gate is gone and the row still waits: it is picked.
        let free = session("worker-a", "worker", "row", 100, None);
        assert_eq!(pick(&[free], &due_now, "o/r", "o-r").len(), 1);
    }

    #[test]
    fn a_wait_is_claimed_by_one_process_only() {
        let dir = tempfile::tempdir().unwrap();
        let wait = key("row/../x", 100);
        assert!(claim(&marker_path(dir.path(), &wait)));
        assert!(!claim(&marker_path(dir.path(), &wait)));
        // Another moment of the same row is another wait.
        assert!(claim(&marker_path(dir.path(), &key("row/../x", 101))));
        assert!(marker_path(dir.path(), &wait).starts_with(dir.path().join("wait-notified")));
    }

    #[test]
    fn markers_older_than_a_day_go_and_newer_ones_stay() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (key("old", 1), key("new", 2));
        assert!(claim(&marker_path(dir.path(), &old)) && claim(&marker_path(dir.path(), &new)));
        let file = std::fs::File::options()
            .write(true)
            .open(marker_path(dir.path(), &old))
            .unwrap();
        file.set_modified(std::time::SystemTime::now() - Duration::from_secs(2 * 86_400))
            .unwrap();
        prune_old_markers(&dir.path().join("wait-notified"), MARKER_KEPT);
        assert!(!marker_path(dir.path(), &old).exists());
        assert!(marker_path(dir.path(), &new).exists());
        // No directory is nothing to do.
        prune_old_markers(&dir.path().join("none"), MARKER_KEPT);
        // Any directory is the one pruned, as the gates' markers are.
        let gates = dir.path().join("gate-notified");
        std::fs::create_dir_all(&gates).unwrap();
        std::fs::write(gates.join("a-b"), "").unwrap();
        prune_old_markers(&gates, MARKER_KEPT);
        assert!(gates.join("a-b").exists());
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
