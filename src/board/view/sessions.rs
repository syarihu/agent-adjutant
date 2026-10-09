//! The sessions the board lists and the git state of each.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::board::{self, Server, settings_now};
use crate::kernel::runner;
use crate::task;

use super::state::{Lines, Listing, split_main};
use super::waiting::{GateCache, waiting_hub, waiting_worker};

/// How many characters of the agent's activity or request the board carries.
const AGENT_TEXT_CHARS: usize = 200;

// ── what the board reads ─────────────────────────────────────────────

/// Where a session runs: what its record says it was started in (`recorded`), or — for a
/// record written before it said so — the settings and a live look through tmux for the pid.
pub(super) fn session_terminal(
    recorded: Option<&crate::infra::terminal::SessionTerminal>,
    terminal_settings: &crate::infra::terminal::TerminalSettings,
    views: &mut HashMap<PathBuf, TmuxView>,
    pid: Option<u32>,
) -> crate::infra::terminal::SessionTerminal {
    if let Some(recorded) = recorded {
        return recorded.clone();
    }
    let backend = crate::infra::terminal::backend_name(terminal_settings);
    let tmux = backend == "tmux";
    // Asked of tmux only here: a session whose record says where it runs needs no look at the
    // settings' own server.
    let pane = pid.filter(|_| tmux).and_then(|p| {
        let view = tmux_view(views, terminal_settings.tmux_socket());
        crate::infra::terminal::find_matching_pane(&view.panes, Some(p), None)
    });
    crate::infra::terminal::SessionTerminal {
        backend: backend.to_string(),
        socket: terminal_settings
            .tmux_socket()
            .filter(|_| tmux)
            .map(str::to_string),
        session: tmux.then(|| terminal_settings.tmux_session().to_string()),
        window: pane.map(|p| p.window_id.clone()),
        pane: pane.map(|p| p.pane_id.clone()),
    }
}

/// The `hubs[]` id of the hub a worker names by `key`: the repository's own hub when it
/// names none, and one made from the key when no hub of that key was found.
pub(super) fn parent_hub_id(
    repo: &crate::kernel::identity::RepoInfo,
    hubs: &[crate::mail::RepoHub],
    key: Option<&str>,
) -> String {
    match key.map(str::trim).filter(|s| !s.is_empty()) {
        Some(key) => {
            let slug = crate::kernel::identity::slug_for(&repo.nwo, Some(key));
            hubs.iter()
                .find(|h| h.slug == slug)
                .map(|h| h.id.clone())
                .unwrap_or_else(|| format!("hub-{key}"))
        }
        None => "hub".to_string(),
    }
}

/// The title of task `id` in the task directory of the hub `slug`, if there is such a record.
/// Read here rather than taken from the page's own task list so that a worker under another
/// hub names its task as well.
pub(super) fn linked_task_title(state_dir: &Path, slug: &str, id: &str) -> Option<String> {
    let title = task::get(state_dir, slug, id).ok()?.title;
    Some(title.trim().to_string()).filter(|t| !t.is_empty())
}

/// The board ids of the workers in `paths`, in order: `worker-<name>` for the worktree's own
/// name, and `worker-<name>-<digest of the path>` when another of them has that name. A
/// worktree called `main` keeps `worker-main` unless the main checkout's own session
/// (`main_listed`) is on the board and has it. A name that is not shared keeps the id it always
/// had, so nothing that already holds one is told a new one.
pub fn worker_session_ids(paths: &[String], main_listed: bool) -> Vec<String> {
    let name_of = |path: &str| {
        Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let names: Vec<String> = paths.iter().map(|p| name_of(p)).collect();
    paths
        .iter()
        .zip(&names)
        .map(|(path, name)| {
            let shared = names.iter().filter(|other| *other == name).count() > 1
                || (main_listed && name == "main");
            match shared {
                true => format!(
                    "worker-{name}-{}",
                    crate::kernel::identity::short_digest(path)
                ),
                false => format!("worker-{name}"),
            }
        })
        .collect()
}

/// A tmux socket as a lookup key: the path of the server it names, so that no setting, a bare
/// name and the path a record kept for the same server are one key and one pair of `list-*`
/// calls. The directory is resolved when it can be, because `/tmp` is `/private/tmp` on a Mac.
pub fn socket_key(socket: Option<&str>) -> PathBuf {
    #[cfg(unix)]
    let uid = unsafe { libc::getuid() };
    #[cfg(not(unix))]
    let uid = 0;
    socket_key_in(
        socket,
        std::env::var("TMUX").ok().as_deref(),
        std::env::var("TMUX_TMPDIR").ok().as_deref(),
        uid,
    )
}

/// `socket_key` with the environment it reads handed in.
pub fn socket_key_in(
    socket: Option<&str>,
    tmux_env: Option<&str>,
    tmpdir: Option<&str>,
    uid: u32,
) -> PathBuf {
    let path = crate::infra::terminal::tmux_socket_path(socket, tmux_env, tmpdir, uid);
    match (
        path.parent().and_then(|dir| dir.canonicalize().ok()),
        path.file_name(),
    ) {
        (Some(dir), Some(leaf)) => dir.join(leaf),
        _ => path,
    }
}

/// What one tmux server said about its panes and clients in one poll.
pub(super) struct TmuxView {
    pub(super) panes: Vec<crate::infra::terminal::TmuxPane>,
    /// Clients attached to each window, by window id.
    attached: HashMap<String, u32>,
}

impl TmuxView {
    fn look(socket: Option<&str>) -> Self {
        let panes = crate::infra::terminal::list_tmux_panes(socket).unwrap_or_default();
        let clients = crate::infra::terminal::list_tmux_clients(socket);
        let attached = crate::infra::terminal::attached_counts(&panes, &clients);
        TmuxView { panes, attached }
    }
}

/// What the tmux server on `socket` says, asked the first time it is needed and kept after.
/// The first spelling of a server's socket is the one tmux is run with.
pub(super) fn tmux_view<'a>(
    views: &'a mut HashMap<PathBuf, TmuxView>,
    socket: Option<&str>,
) -> &'a TmuxView {
    views
        .entry(socket_key(socket))
        .or_insert_with(|| TmuxView::look(socket))
}

/// When a session's tmux window last had activity and how many clients are on it, from the
/// server its own record names — which is not always the settings' one. Both `None` for a
/// session that is not in tmux or whose window is not there.
fn tmux_activity(
    views: &mut HashMap<PathBuf, TmuxView>,
    terminal: &crate::infra::terminal::SessionTerminal,
) -> (Option<i64>, Option<u32>) {
    let Some(window) = terminal
        .window
        .as_deref()
        .filter(|_| terminal.backend == "tmux")
    else {
        return (None, None);
    };
    let view = tmux_view(views, terminal.socket.as_deref());
    let Some(pane) = view.panes.iter().find(|p| p.window_id == window) else {
        return (None, None);
    };
    (pane.window_activity, view.attached.get(window).copied())
}

/// The pane whose screen stands for a session: the one its record names, else the first of its
/// window's. `None` for a session that is not in tmux or whose window is not there.
fn tmux_pane_of(
    views: &mut HashMap<PathBuf, TmuxView>,
    terminal: &crate::infra::terminal::SessionTerminal,
) -> Option<String> {
    let window = terminal
        .window
        .as_deref()
        .filter(|_| terminal.backend == "tmux")?;
    let view = tmux_view(views, terminal.socket.as_deref());
    let first = view.panes.iter().find(|p| p.window_id == window)?;
    Some(
        terminal
            .pane
            .clone()
            .unwrap_or_else(|| first.pane_id.clone()),
    )
}

/// What one poll shares: asked once and kept for every session it lists.
struct Poll<'a> {
    server: &'a Server,
    settings: &'a crate::kernel::config::Settings,
    hubs: &'a [crate::mail::RepoHub],
    listing: Listing<'a>,
    only: Option<&'a str>,
    /// What tmux says, asked once per socket per poll and only for a socket a listed session
    /// needs: its record's own, or the settings' when the record says none.
    views: HashMap<PathBuf, TmuxView>,
    /// Read once per poll, for the hubs of the sessions listed, so a worker under a parent-task
    /// hub shows its gate on the repository board too. Read-only — closing a resumed gate stays
    /// with the board that owns the hub's directory.
    gates: GateCache,
    hub_agent: String,
    worker_agent: String,
    /// The panes whose last line was read this poll.
    screens: HashSet<String>,
    /// The worktrees whose diff this poll asked for.
    diffs: HashSet<String>,
}

impl Poll<'_> {
    fn skipped(&self, id: &str) -> bool {
        self.only.is_some_and(|wanted| wanted != id)
    }

    fn terminal(
        &mut self,
        recorded: Option<&crate::infra::terminal::SessionTerminal>,
        pid: Option<u32>,
    ) -> crate::infra::terminal::SessionTerminal {
        session_terminal(recorded, &self.settings.terminal, &mut self.views, pid)
    }

    fn tmux_activity(
        &mut self,
        terminal: &crate::infra::terminal::SessionTerminal,
    ) -> (Option<i64>, Option<u32>) {
        tmux_activity(&mut self.views, terminal)
    }

    /// The last line of a session's pane, when the page asked for it: read for a session that
    /// runs in tmux, and cached by pane (see `LastLines`).
    fn last_line(
        &mut self,
        is_hub: bool,
        terminal: &crate::infra::terminal::SessionTerminal,
        agent: &str,
        present: bool,
        activity: Option<i64>,
    ) -> Option<String> {
        let asked = match self.listing.lines {
            Lines::None => false,
            Lines::Hubs => is_hub,
            Lines::All => true,
        };
        if !asked || !present {
            return None;
        }
        let pane = tmux_pane_of(&mut self.views, terminal)?;
        let key = format!(
            "{}\t{pane}",
            socket_key(terminal.socket.as_deref()).display()
        );
        self.screens.insert(key.clone());
        let agent =
            crate::infra::agent::Agent::parse(agent).unwrap_or(crate::infra::agent::Agent::Generic);
        self.server.last_lines.look(
            &key,
            activity,
            Instant::now(),
            crate::infra::clock::now_secs(),
            || {
                let screen =
                    crate::infra::terminal::look_at_tmux_pane(terminal.socket.as_deref(), &pane)?;
                crate::mail::last_output_line(agent, &screen)
            },
        )
    }

    /// What the agent's hooks last said about a session that runs, joined by its session id and
    /// then by its process (never by where it runs). A ledger that cannot be listed is said in
    /// `error`, not taken for a session with no row.
    fn agent_session(
        &self,
        present: bool,
        identity: crate::registry::AgentIdentity,
    ) -> Option<board::SessionAgentState> {
        if !present {
            return None;
        }
        match crate::registry::agent_session_of(
            &self.server.ctx.state,
            self.listing.processes,
            &identity,
        ) {
            Ok(Some(row)) => Some(agent_state_of(&row)),
            Ok(None) => None,
            Err(error) => Some(board::SessionAgentState {
                session_id: None,
                status: None,
                pending: None,
                updated_at: None,
                last_event_at: None,
                activity: None,
                request: None,
                model: None,
                context_percent: None,
                last_message: None,
                last_message_at: None,
                last_prompt_at: None,
                subagents: Vec::new(),
                error: Some(error),
            }),
        }
    }

    fn worker_waiting(
        &mut self,
        hub_id: &str,
        worktree: &str,
        started: Option<&str>,
        phase_at: Option<i64>,
    ) -> Option<board::SessionWaiting> {
        let hubs = self.hubs;
        let hub = hubs.iter().find(|h| h.id == hub_id)?;
        waiting_worker(hub, self.gates.of(&hub.slug), worktree, started, phase_at)
    }
}

/// A ledger row as the board carries it: the first line of the activity and the request, cut
/// short, the model and context use as the status line showed them, the last message of the
/// turn whole, and the sub-agents that run.
fn agent_state_of(row: &crate::registry::AgentSession) -> board::SessionAgentState {
    let first_line = |text: &Option<String>| {
        let line = text.as_deref()?.lines().next()?.trim();
        (!line.is_empty()).then(|| super::waiting::cut_chars(line, AGENT_TEXT_CHARS))
    };
    board::SessionAgentState {
        session_id: Some(row.session_id.clone()).filter(|id| !id.is_empty()),
        status: row.status.as_ref().map(|s| s.as_str().to_string()),
        pending: row.pending_status.as_ref().map(|s| s.as_str().to_string()),
        updated_at: row.updated_at,
        last_event_at: row.last_event_at,
        activity: first_line(&row.activity),
        request: first_line(&row.request),
        model: row.model.clone().filter(|m| !m.trim().is_empty()),
        context_percent: row
            .context_percent
            .filter(|p| p.is_finite())
            .map(|p| p.round().clamp(0.0, 100.0) as u8),
        // The whole text, line breaks and all, not the first line the other two are.
        last_message: row
            .last_message
            .as_deref()
            .filter(|message| !message.trim().is_empty())
            .map(|message| super::waiting::cut_chars(message, crate::registry::LAST_MESSAGE_CHARS)),
        last_message_at: row.last_message_at,
        last_prompt_at: row.last_prompt_at,
        subagents: row
            .subagents
            .iter()
            .map(|sub| board::SessionSubagent {
                id: sub.id.clone(),
                kind: sub.kind.clone(),
                started_at: sub.started_at,
                activity: first_line(&sub.activity),
            })
            .collect(),
        error: None,
    }
}

/// The sessions this board lists, hubs first and then the workers of `linked_paths`, as the
/// page reads them and as a board terminal resolves an id. `worker_data` is asked for a
/// worker's status and branch by its place in `linked_paths`.
///
/// With `only`, the one session of that id: the others are skipped before anything is read or
/// run for them, so that the one and the whole list are the same code and cannot drift.
pub(super) fn sessions_of(
    server: &Server,
    settings: &crate::kernel::config::Settings,
    hubs: &[crate::mail::RepoHub],
    linked_paths: &[String],
    listing: Listing<'_>,
    only: Option<&str>,
    worker_data: impl FnMut(usize, &str) -> (crate::registry::WorkerStatus, Option<String>),
) -> Vec<board::Session> {
    let mut poll = Poll {
        server,
        settings,
        hubs,
        listing,
        only,
        views: HashMap::new(),
        gates: GateCache {
            state_dir: server.ctx.state.clone(),
            read: HashMap::new(),
        },
        hub_agent: runner::agent_from_runner(
            settings
                .hub_runner
                .as_deref()
                .unwrap_or(runner::DEFAULT_HUB_RUNNER),
        ),
        worker_agent: runner::agent_from_runner(
            settings
                .agent_runner
                .as_deref()
                .unwrap_or(runner::DEFAULT_AGENT_RUNNER),
        ),
        screens: HashSet::new(),
        diffs: HashSet::new(),
    };
    let mut sessions = hub_sessions(&mut poll);
    sessions.extend(worker_sessions(&mut poll, linked_paths, worker_data));
    sessions.extend(main_worker_session(&mut poll));
    // Only a poll that reached the main checkout's turn forgets the lines nobody asked for, as
    // before the split. A listing that failed left the workers out, which is not the same as
    // their panes being gone.
    if poll.listing.lines == Lines::All && poll.listing.listed && !poll.skipped("worker-main") {
        server.last_lines.keep_only(&poll.screens);
    }
    // The same for the diffs, and the reading of what is due starts here, off this thread.
    if poll.listing.diffs && poll.listing.listed && only.is_none() {
        server.diffs.keep_only(&poll.diffs);
        server.diffs.refresh(Instant::now());
    }
    sessions
}

/// One session per hub, in the order the hubs were listed.
fn hub_sessions(poll: &mut Poll) -> Vec<board::Session> {
    let server = poll.server;
    let repo = &server.ctx.repo;
    let hubs = poll.hubs;
    let mut sessions = Vec::new();
    for h in hubs {
        if poll.skipped(&h.id) {
            continue;
        }
        let recorded = match crate::registry::read_hub_record(&server.ctx.state, &h.slug) {
            crate::registry::Recorded::Found(r) => (r.terminal, r.ps_started),
            _ => (None, None),
        };
        let (recorded, ps_started) = recorded;
        let terminal = poll.terminal(recorded.as_ref(), h.state.pid);
        let (last_activity_at, attached) = poll.tmux_activity(&terminal);
        let agent = poll.hub_agent.clone();
        let line = poll.last_line(true, &terminal, &agent, h.state.present, last_activity_at);
        let waiting = waiting_hub(h, &poll.gates.of(&h.slug).open);
        let conversation =
            crate::registry::hub_session(&server.ctx.state, &h.slug).map(|s| s.session_id);
        let agent_session = poll.agent_session(
            h.state.present,
            crate::registry::AgentIdentity {
                session_id: conversation.as_deref(),
                pid: h.state.pid,
                ps_started: ps_started.as_deref(),
            },
        );

        sessions.push(board::Session {
            id: h.id.clone(),
            conversation,
            kind: "hub".to_string(),
            agent,
            terminal,
            hub: None,
            key: h.key.clone(),
            worktree: repo.main.clone(),
            branch: poll.listing.main_branch.clone(),
            task: None,
            title: Some(h.name.clone()),
            task_title: None,
            present: h.state.present,
            stale: h.state.stale,
            pid: h.state.pid,
            started_at: h.state.started_at.clone(),
            phase: None,
            phase_at: None,
            phases: Vec::new(),
            last_activity_at,
            last_line: line,
            attached,
            waiting,
            agent_session,
            uncommitted: None,
            uncommitted_error: None,
            branch_pr: None,
            branch_pr_error: None,
        });
    }
    sessions
}

/// What tells one worker's session from another's: who it is and where its facts came from.
struct WorkerSource {
    id: String,
    worktree: String,
    branch: Option<String>,
    status: crate::registry::WorkerStatus,
    record: Option<crate::registry::WorkerRecord>,
    conversation: Option<String>,
    title_fallback: Option<String>,
    task: Option<String>,
}

/// One session per linked worktree, in listing order.
fn worker_sessions(
    poll: &mut Poll,
    linked_paths: &[String],
    mut worker_data: impl FnMut(usize, &str) -> (crate::registry::WorkerStatus, Option<String>),
) -> Vec<board::Session> {
    let repo = &poll.server.ctx.repo;
    let mut sessions = Vec::new();
    // Whether the main checkout is listed below as `worker-main`, which a worktree of that
    // name would otherwise collide with.
    let main_listed = matches!(
        crate::registry::read_worker_record(Path::new(&repo.main)),
        crate::registry::Recorded::Found(_)
    ) || crate::registry::worker_session(Path::new(&repo.main)).is_some();
    let worker_ids = worker_session_ids(linked_paths, main_listed);
    for (index, (path, id)) in linked_paths.iter().zip(worker_ids).enumerate() {
        if poll.skipped(&id) {
            continue;
        }
        let (status, branch) = worker_data(index, path);
        let wt_path = Path::new(path);
        let record = match crate::registry::read_worker_record(wt_path) {
            crate::registry::Recorded::Found(record) => Some(record),
            _ => None,
        };
        let saved_session = crate::registry::worker_session(wt_path);
        sessions.push(worker_session(
            poll,
            WorkerSource {
                id,
                worktree: path.clone(),
                branch,
                status,
                record,
                conversation: saved_session.as_ref().map(|s| s.session_id.clone()),
                title_fallback: saved_session.and_then(|s| s.title),
                task: crate::registry::worker_task(wt_path),
            },
        ));
    }
    sessions
}

/// The session of one worker, whichever checkout it runs in.
fn worker_session(poll: &mut Poll, source: WorkerSource) -> board::Session {
    let repo = &poll.server.ctx.repo;
    let hubs = poll.hubs;
    let WorkerSource {
        id,
        worktree,
        branch,
        status,
        record,
        conversation,
        title_fallback,
        task,
    } = source;
    let hub_key = crate::registry::worker_hub_key(Path::new(&worktree));
    let parent_hub = parent_hub_id(repo, hubs, hub_key.as_deref());
    let started_at = record.as_ref().and_then(|r| r.started_at.clone());

    let terminal = poll.terminal(
        record
            .as_ref()
            .and_then(crate::registry::WorkerRecord::terminal)
            .as_ref(),
        status.pid,
    );
    let (last_activity_at, attached) = poll.tmux_activity(&terminal);
    let agent = poll.worker_agent.clone();
    let line = poll.last_line(false, &terminal, &agent, status.present, last_activity_at);
    let waiting = poll.worker_waiting(
        &parent_hub,
        &worktree,
        started_at.as_deref(),
        status.phase_at,
    );
    let task_title = task.as_deref().and_then(|id| {
        let slug = crate::kernel::identity::slug_for(&repo.nwo, hub_key.as_deref());
        linked_task_title(&poll.gates.state_dir, &slug, id)
    });
    let agent_session = poll.agent_session(
        status.present,
        crate::registry::AgentIdentity {
            session_id: conversation.as_deref(),
            pid: status.pid,
            ps_started: record.as_ref().and_then(|r| r.ps_started.as_deref()),
        },
    );
    // Asked for on a poll that lists the sessions, and read in the background: what the poll
    // shows is what an earlier one asked for.
    if poll.listing.diffs {
        poll.server.diffs.want(&worktree, status.present);
        poll.diffs.insert(worktree.clone());
    }
    let (uncommitted, uncommitted_error) = poll.server.diffs.look(&worktree);
    let branch_pr = match (&task, &branch, &poll.server.pr_poll) {
        (None, Some(branch), Some(pr_poll)) => pr_poll.branch_pr(&repo.nwo, branch),
        _ => (None, None),
    };
    let (branch_pr, branch_pr_error) = branch_pr;

    board::Session {
        id,
        conversation,
        kind: "worker".to_string(),
        agent,
        terminal,
        hub: Some(parent_hub),
        key: None,
        worktree,
        branch,
        task,
        title: status.title.or(title_fallback),
        task_title,
        present: status.present,
        stale: status.stale,
        pid: status.pid,
        started_at,
        phase: status.phase,
        phase_at: status.phase_at,
        phases: status.phases,
        last_activity_at,
        last_line: line,
        attached,
        waiting,
        agent_session,
        uncommitted,
        uncommitted_error,
        branch_pr,
        branch_pr_error,
    }
}

/// The main checkout's own session, if it has a record or a saved conversation.
fn main_worker_session(poll: &mut Poll) -> Option<board::Session> {
    if poll.skipped("worker-main") {
        return None;
    }
    let server = poll.server;
    let repo = &server.ctx.repo;
    let hubs = poll.hubs;
    let main = Path::new(&repo.main);
    match (
        crate::registry::read_worker_record(main),
        crate::registry::worker_session(main),
    ) {
        (crate::registry::Recorded::Found(record), saved) => {
            let status = crate::registry::worker_status_with(poll.listing.processes, main);
            let branch = poll.listing.main_branch.clone();
            Some(worker_session(
                poll,
                WorkerSource {
                    id: "worker-main".to_string(),
                    worktree: repo.main.clone(),
                    branch,
                    status,
                    // The raw task, not `worker_task`'s trimmed one.
                    task: record.task.clone(),
                    record: Some(record),
                    conversation: saved.map(|s| s.session_id),
                    // The main checkout's title is its status's alone.
                    title_fallback: None,
                },
            ))
        }
        (_, Some(saved)) => {
            let parent_hub = parent_hub_id(repo, hubs, saved.hub.as_deref());
            let terminal = poll.terminal(None, None);
            let (last_activity_at, attached) = poll.tmux_activity(&terminal);
            let agent = poll.worker_agent.clone();
            // Not present, so there is no pane to read.
            let line = poll.last_line(false, &terminal, &agent, false, last_activity_at);
            let waiting = poll.worker_waiting(&parent_hub, &repo.main, None, None);

            let task_title = saved.task.as_deref().and_then(|id| {
                let slug = crate::kernel::identity::slug_for(&repo.nwo, saved.hub.as_deref());
                linked_task_title(&poll.gates.state_dir, &slug, id)
            });
            Some(board::Session {
                id: "worker-main".to_string(),
                conversation: Some(saved.session_id.clone()),
                kind: "worker".to_string(),
                agent,
                terminal,
                hub: Some(parent_hub),
                key: None,
                worktree: repo.main.clone(),
                branch: poll.listing.main_branch.clone(),
                task: saved.task,
                title: saved.title,
                task_title,
                present: false,
                stale: false,
                pid: None,
                started_at: None,
                phase: None,
                phase_at: None,
                phases: Vec::new(),
                last_activity_at,
                last_line: line,
                attached,
                waiting,
                agent_session: None,
                uncommitted: None,
                uncommitted_error: None,
                branch_pr: None,
                branch_pr_error: None,
            })
        }
        (_, None) => None,
    }
}

/// The session `only` of this board, or all of them for `None`, with no more than `only` asks
/// for read or run: no `ps` or `git` for another worktree, no gates of another hub.
fn board_sessions_of(
    server: &Server,
    settings: &crate::kernel::config::Settings,
    only: Option<&str>,
) -> (Vec<board::Session>, bool) {
    let repo = &server.ctx.repo;
    // Whether the worktrees could be listed: a failure leaves the main checkout alone, which is
    // not the same as the repository having no other.
    let listed = crate::kernel::identity::worktrees(&repo.main);
    let complete = listed.is_ok();
    let listed = listed.unwrap_or_default();
    let (main_branch, linked) = split_main(&repo.main, listed);
    let linked_paths: Vec<String> = linked.iter().map(|w| w.path.clone()).collect();
    // A `ps` for each of the few it is asked about, not the whole process table.
    let processes = crate::registry::ProcessTable::each();
    let hubs =
        crate::mail::all_repo_hubs_among_with(&server.ctx.state, &processes, repo, &linked_paths);
    let sessions = sessions_of(
        server,
        settings,
        &hubs,
        &linked_paths,
        Listing {
            processes: &processes,
            main_branch,
            lines: Lines::None,
            diffs: false,
            listed: complete,
        },
        only,
        |index, path| {
            (
                crate::registry::worker_status_with(&processes, Path::new(path)),
                linked[index].branch.clone(),
            )
        },
    );
    (sessions, complete)
}

/// The session `id` of this board, resolved without listing the others. Equal to its entry in
/// the list `state` carries.
pub fn board_session(
    server: &Server,
    settings: &crate::kernel::config::Settings,
    id: &str,
) -> Option<board::Session> {
    board_sessions_of(server, settings, Some(id))
        .0
        .into_iter()
        .next()
}

/// Every session of this board, from its own records and with no pane read, and whether the
/// list is the whole of them (`false` when the worktrees could not be listed). What a job that
/// watches the sessions reads, so that it and the page agree on who is waiting.
pub fn board_sessions(
    server: &Server,
    settings: &crate::kernel::config::Settings,
) -> (Vec<board::Session>, bool) {
    board_sessions_of(server, settings, None)
}

/// How long the git check of one session may take in all. A worktree on a slow disk or a
/// network mount must not hold a connection thread indefinitely.
const GIT_CHECK_SECS: u64 = 10;

/// The session `id` of this board, from the board's own records.
pub fn find_session(
    server: &Server,
    settings: &crate::kernel::config::Settings,
    id: &str,
) -> Result<board::Session, String> {
    board_session(server, settings, id).ok_or_else(|| format!("no such session: {id}"))
}

/// The slug of the hub the worker in `worktree` reports to, from its own record: the same one
/// `parent_hub_id` and the hub listing arrive at, without listing the hubs.
fn worker_hub_slug(repo: &crate::kernel::identity::RepoInfo, worktree: &Path) -> String {
    match crate::registry::worker_hub_key(worktree) {
        Some(key) => crate::kernel::identity::slug_for(&repo.nwo, Some(&key)),
        None => match &repo.hub {
            Some(_) => repo
                .clone()
                .addressed(None)
                .map(|default| default.slug)
                .unwrap_or_else(|_| repo.slug.clone()),
            None => repo.slug.clone(),
        },
    }
}

/// What one session's worktree holds that no remote has, `None` when the directory is gone.
/// The path comes from the board's own record of the session, never from the request.
pub fn git_state_of(
    server: &Server,
    session: &board::Session,
) -> Result<Option<crate::kernel::worktree_state::GitState>, String> {
    // The task's own base, when it has one: work meant for a release branch is not merged
    // because it is in the default branch.
    let base = session.task.as_deref().and_then(|task_id| {
        let slug = worker_hub_slug(&server.ctx.repo, Path::new(&session.worktree));
        task::get(&server.ctx.state, &slug, task_id).ok()?.base
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(GIT_CHECK_SECS);
    crate::kernel::worktree_state::worktree_git_state(
        Path::new(&session.worktree),
        base.as_deref(),
        deadline,
    )
}

/// What one session's worktree holds that no remote has, asked when a person looks rather than
/// on every poll. `None` when the worktree is gone.
pub fn session_git(
    server: &Server,
    id: &str,
) -> Result<Option<crate::kernel::worktree_state::GitState>, String> {
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    git_state_of(server, &session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{AgentSession, AgentStatus, Subagent};

    #[test]
    fn a_ledger_row_is_carried_by_its_words_first_lines_model_and_subagents() {
        let row = AgentSession {
            status: Some(AgentStatus::Other("thinking".to_string())),
            pending_status: Some(AgentStatus::Done),
            updated_at: Some(5),
            last_event_at: Some(6),
            activity: Some(format!("Bash: {}\nsecond line", "x".repeat(500))),
            request: Some("\n".to_string()),
            model: Some("Opus 5".to_string()),
            context_percent: Some(42.6),
            last_message: Some("First line.\nSecond line.".to_string()),
            last_message_at: Some(7),
            last_prompt_at: Some(8),
            subagents: vec![
                Subagent {
                    id: "a1".to_string(),
                    kind: Some("Explore".to_string()),
                    started_at: Some(3),
                    activity: Some(format!("Grep: {}\nmore", "y".repeat(500))),
                    ..Subagent::default()
                },
                Subagent {
                    id: "a2".to_string(),
                    ..Subagent::default()
                },
            ],
            ..AgentSession::default()
        };
        let state = agent_state_of(&row);
        assert_eq!(state.status.as_deref(), Some("thinking"));
        assert_eq!(state.pending.as_deref(), Some("done"));
        assert_eq!((state.updated_at, state.last_event_at), (Some(5), Some(6)));
        let activity = state.activity.unwrap();
        assert_eq!(activity.chars().count(), AGENT_TEXT_CHARS + 1);
        assert!(activity.ends_with('…') && !activity.contains('\n'));
        assert_eq!(state.request, None);
        assert_eq!(state.model.as_deref(), Some("Opus 5"));
        assert_eq!(state.context_percent, Some(43));
        assert_eq!(
            state.last_message.as_deref(),
            Some("First line.\nSecond line.")
        );
        assert_eq!(state.last_message_at, Some(7));
        assert_eq!(state.last_prompt_at, Some(8));
        assert_eq!(state.subagents.len(), 2);
        assert_eq!(state.subagents[0].kind.as_deref(), Some("Explore"));
        assert_eq!(state.subagents[0].started_at, Some(3));
        let sub_activity = state.subagents[0].activity.clone().unwrap();
        assert_eq!(sub_activity.chars().count(), AGENT_TEXT_CHARS + 1);
        assert_eq!(state.subagents[1].activity, None);
        assert_eq!(state.error, None);
    }

    #[test]
    fn a_last_message_is_cut_long_and_a_blank_one_is_none() {
        let of = |message: String| {
            agent_state_of(&AgentSession {
                last_message: Some(message),
                ..AgentSession::default()
            })
            .last_message
        };
        let cut = of(format!(
            "{}\nmore",
            "z".repeat(crate::registry::LAST_MESSAGE_CHARS)
        ));
        let cut = cut.unwrap();
        assert_eq!(cut.chars().count(), crate::registry::LAST_MESSAGE_CHARS + 1);
        assert!(cut.ends_with('…'));
        assert_eq!(of("  \n".to_string()), None);
    }

    #[test]
    fn a_context_percent_that_is_not_a_figure_or_is_out_of_range_is_cut_to_one() {
        let of = |percent: f64| {
            agent_state_of(&AgentSession {
                context_percent: Some(percent),
                ..AgentSession::default()
            })
            .context_percent
        };
        assert_eq!(of(f64::NAN), None);
        assert_eq!(of(-5.0), Some(0));
        assert_eq!(of(180.0), Some(100));
    }
}
