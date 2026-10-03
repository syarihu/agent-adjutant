//! The sessions the board lists and the git state of each.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::Value;

use crate::kernel::runner;
use crate::messaging;
use crate::session;
use crate::task;

use super::Server;
use super::state::{
    GateCache, Listing, linked_task_title, parent_hub_id, session_terminal, settings_now,
    socket_key, split_main, waiting_hub, waiting_worker, worker_session_ids,
};

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

/// The sessions this board lists, hubs first and then the workers of `linked_paths`, as the
/// page reads them and as a board terminal resolves an id. `worker_data` is asked for a
/// worker's status and branch by its place in `linked_paths`.
///
/// With `only`, the one session of that id: the others are skipped before anything is read or
/// run for them, so that the one and the whole list are the same code and cannot drift.
pub(super) fn sessions_of(
    server: &Server,
    settings: &crate::kernel::config::Settings,
    hubs: &[session::RepoHub],
    linked_paths: &[String],
    listing: Listing<'_>,
    only: Option<&str>,
    mut worker_data: impl FnMut(usize, &str) -> (messaging::WorkerStatus, Option<String>),
) -> Vec<session::Session> {
    let repo = &server.ctx.repo;
    let terminal_settings = &settings.terminal;
    let skipped = |id: &str| only.is_some_and(|wanted| wanted != id);
    // What tmux says, asked once per socket per poll and only for a socket a listed session
    // needs: its record's own, or the settings' when the record says none.
    let mut views: HashMap<PathBuf, TmuxView> = HashMap::new();
    // Read once per poll, for the hubs of the sessions listed, so a worker under a parent-task
    // hub shows its gate on the repository board too. Read-only — closing a resumed gate stays
    // with the board that owns the hub's directory.
    let mut gates = GateCache {
        state_dir: crate::infra::paths::state_dir(),
        read: HashMap::new(),
    };
    let worker_waiting = |gates: &mut GateCache,
                          hub_id: &str,
                          worktree: &str,
                          started: Option<&str>,
                          phase_at: Option<i64>| {
        let hub = hubs.iter().find(|h| h.id == hub_id)?;
        waiting_worker(hub, gates.of(&hub.slug), worktree, started, phase_at)
    };

    let hub_agent = runner::agent_from_runner(
        settings
            .hub_runner
            .as_deref()
            .unwrap_or(runner::DEFAULT_HUB_RUNNER),
    );
    let worker_agent = runner::agent_from_runner(
        settings
            .agent_runner
            .as_deref()
            .unwrap_or(runner::DEFAULT_AGENT_RUNNER),
    );

    let Listing {
        processes,
        main_branch,
        with_lines,
    } = listing;
    // The last line of a session's pane, when the page asked for it: read for a session that
    // runs in tmux, and cached by pane (see `LastLines`).
    let mut screens: HashSet<String> = HashSet::new();
    let mut last_line = |views: &mut HashMap<PathBuf, TmuxView>,
                         terminal: &crate::infra::terminal::SessionTerminal,
                         agent: &str,
                         present: bool,
                         activity: Option<i64>| {
        if !with_lines || !present {
            return None;
        }
        let pane = tmux_pane_of(views, terminal)?;
        let key = format!(
            "{}\t{pane}",
            socket_key(terminal.socket.as_deref()).display()
        );
        screens.insert(key.clone());
        let agent =
            crate::infra::agent::Agent::parse(agent).unwrap_or(crate::infra::agent::Agent::Generic);
        server.last_lines.look(
            &key,
            activity,
            Instant::now(),
            crate::infra::clock::now_secs(),
            || {
                let screen =
                    crate::infra::terminal::look_at_tmux_pane(terminal.socket.as_deref(), &pane)?;
                crate::terminal::last_output_line(agent, &screen)
            },
        )
    };

    let main_branch = || main_branch.clone();
    let mut sessions: Vec<session::Session> = Vec::new();

    // 1. Hub sessions from hubs
    for h in hubs.iter().filter(|h| !skipped(&h.id)) {
        let record = crate::infra::fs::read_json(&messaging::hub_record_path(&h.slug));
        let terminal =
            session_terminal(record.as_ref(), terminal_settings, &mut views, h.state.pid);
        let (last_activity_at, attached) = tmux_activity(&mut views, &terminal);
        let line = last_line(
            &mut views,
            &terminal,
            &hub_agent,
            h.state.present,
            last_activity_at,
        );

        sessions.push(session::Session {
            id: h.id.clone(),
            conversation: messaging::hub_session(&h.slug).map(|s| s.session_id),
            kind: "hub".to_string(),
            agent: hub_agent.clone(),
            terminal,
            hub: None,
            key: h.key.clone(),
            worktree: repo.main.clone(),
            branch: main_branch(),
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
            waiting: waiting_hub(h, &gates.of(&h.slug).open),
        });
    }

    // 2. Worker sessions from linked worktrees
    // Whether the main checkout is listed below as `worker-main`, which a worktree of that
    // name would otherwise collide with.
    let main_listed =
        crate::infra::fs::read_json(&messaging::worker_record_path(Path::new(&repo.main)))
            .is_some()
            || messaging::worker_session(Path::new(&repo.main)).is_some();
    let worker_ids = worker_session_ids(linked_paths, main_listed);
    for (index, (path, id)) in linked_paths.iter().zip(worker_ids).enumerate() {
        if skipped(&id) {
            continue;
        }
        let (status, branch) = worker_data(index, path);
        let wt_path = Path::new(path);
        let record_json = crate::infra::fs::read_json(&messaging::worker_record_path(wt_path));
        let saved_session = messaging::worker_session(wt_path);
        let parent_hub = parent_hub_id(repo, hubs, messaging::worker_hub_key(wt_path).as_deref());
        let started_at = record_json
            .as_ref()
            .and_then(|r| r.get("startedAt"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let conversation = saved_session.as_ref().map(|s| s.session_id.clone());

        let terminal = session_terminal(
            record_json.as_ref(),
            terminal_settings,
            &mut views,
            status.pid,
        );
        let (last_activity_at, attached) = tmux_activity(&mut views, &terminal);
        let line = last_line(
            &mut views,
            &terminal,
            &worker_agent,
            status.present,
            last_activity_at,
        );
        let waiting = worker_waiting(
            &mut gates,
            &parent_hub,
            path,
            started_at.as_deref(),
            status.phase_at,
        );

        let saved_title = saved_session.and_then(|s| s.title);
        let task_id = messaging::worker_task(wt_path);

        let title = status.title.or(saved_title);
        let task_title = task_id.as_deref().and_then(|id| {
            let slug = crate::kernel::identity::slug_for(
                &repo.nwo,
                messaging::worker_hub_key(wt_path).as_deref(),
            );
            linked_task_title(&gates.state_dir, &slug, id)
        });

        sessions.push(session::Session {
            id,
            conversation,
            kind: "worker".to_string(),
            agent: worker_agent.clone(),
            terminal,
            hub: Some(parent_hub),
            key: None,
            worktree: path.clone(),
            branch,
            task: task_id,
            title,
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
        });
    }

    // Also check worker in main checkout if one exists
    let main_record_path = messaging::worker_record_path(Path::new(&repo.main));
    if skipped("worker-main") {
        return sessions;
    }
    if let Some(record_json) = crate::infra::fs::read_json(&main_record_path) {
        let status = messaging::worker_status_with(processes, Path::new(&repo.main));
        let parent_hub = parent_hub_id(
            repo,
            hubs,
            messaging::worker_hub_key(Path::new(&repo.main)).as_deref(),
        );
        let started_at = record_json
            .get("startedAt")
            .and_then(Value::as_str)
            .map(str::to_string);

        let terminal = session_terminal(
            Some(&record_json),
            terminal_settings,
            &mut views,
            status.pid,
        );
        let (last_activity_at, attached) = tmux_activity(&mut views, &terminal);
        let line = last_line(
            &mut views,
            &terminal,
            &worker_agent,
            status.present,
            last_activity_at,
        );
        let waiting = worker_waiting(
            &mut gates,
            &parent_hub,
            &repo.main,
            started_at.as_deref(),
            status.phase_at,
        );

        let task_id = record_json
            .get("task")
            .and_then(Value::as_str)
            .map(str::to_string);

        let task_title = task_id.as_deref().and_then(|id| {
            let slug = crate::kernel::identity::slug_for(
                &repo.nwo,
                messaging::worker_hub_key(Path::new(&repo.main)).as_deref(),
            );
            linked_task_title(&gates.state_dir, &slug, id)
        });
        sessions.push(session::Session {
            id: "worker-main".to_string(),
            conversation: messaging::worker_session(Path::new(&repo.main)).map(|s| s.session_id),
            kind: "worker".to_string(),
            agent: worker_agent.clone(),
            terminal,
            hub: Some(parent_hub),
            key: None,
            worktree: repo.main.clone(),
            branch: main_branch(),
            task: task_id,
            title: status.title,
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
        });
    } else if let Some(saved) = messaging::worker_session(Path::new(&repo.main)) {
        let parent_hub = parent_hub_id(repo, hubs, saved.hub.as_deref());
        let terminal = session_terminal(None, terminal_settings, &mut views, None);
        let (last_activity_at, attached) = tmux_activity(&mut views, &terminal);
        // Not present, so there is no pane to read.
        let line = last_line(
            &mut views,
            &terminal,
            &worker_agent,
            false,
            last_activity_at,
        );
        let waiting = worker_waiting(&mut gates, &parent_hub, &repo.main, None, None);

        let task_title = saved.task.as_deref().and_then(|id| {
            let slug = crate::kernel::identity::slug_for(&repo.nwo, saved.hub.as_deref());
            linked_task_title(&gates.state_dir, &slug, id)
        });
        sessions.push(session::Session {
            id: "worker-main".to_string(),
            conversation: Some(saved.session_id.clone()),
            kind: "worker".to_string(),
            agent: worker_agent.clone(),
            terminal,
            hub: Some(parent_hub),
            key: None,
            worktree: repo.main.clone(),
            branch: main_branch(),
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
        });
    }
    if with_lines {
        server.last_lines.keep_only(&screens);
    }
    sessions
}

/// The session `id` of this board, resolved without listing the others: no `ps` or `git` for
/// another worktree, no gates of another hub. Equal to its entry in the list `state` carries.
pub(in crate::cmd) fn board_session(
    server: &Server,
    settings: &crate::kernel::config::Settings,
    id: &str,
) -> Option<session::Session> {
    let repo = &server.ctx.repo;
    let listed = crate::kernel::identity::worktrees(&repo.main).unwrap_or_default();
    let (main_branch, linked) = split_main(&repo.main, listed);
    let linked_paths: Vec<String> = linked.iter().map(|w| w.path.clone()).collect();
    // A `ps` for each of the few it is asked about, not the whole process table.
    let processes = messaging::ProcessTable::each();
    let hubs = messaging::all_repo_hubs_among_with(&processes, repo, &linked_paths);
    sessions_of(
        server,
        settings,
        &hubs,
        &linked_paths,
        Listing {
            processes: &processes,
            main_branch,
            with_lines: false,
        },
        Some(id),
        |index, path| {
            (
                messaging::worker_status_with(&processes, Path::new(path)),
                linked[index].branch.clone(),
            )
        },
    )
    .into_iter()
    .next()
}

/// How long the git check of one session may take in all. A worktree on a slow disk or a
/// network mount must not hold a connection thread indefinitely.
const GIT_CHECK_SECS: u64 = 10;

/// The session `id` of this board, from the board's own records.
pub(in crate::cmd) fn find_session(
    server: &Server,
    settings: &crate::kernel::config::Settings,
    id: &str,
) -> Result<session::Session, String> {
    board_session(server, settings, id).ok_or_else(|| format!("no such session: {id}"))
}

/// The slug of the hub the worker in `worktree` reports to, from its own record: the same one
/// `parent_hub_id` and the hub listing arrive at, without listing the hubs.
fn worker_hub_slug(repo: &crate::kernel::identity::RepoInfo, worktree: &Path) -> String {
    match messaging::worker_hub_key(worktree) {
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
pub(in crate::cmd) fn git_state_of(
    server: &Server,
    session: &session::Session,
) -> Result<Option<crate::kernel::worktree_state::GitState>, String> {
    // The task's own base, when it has one: work meant for a release branch is not merged
    // because it is in the default branch.
    let base = session.task.as_deref().and_then(|task_id| {
        let slug = worker_hub_slug(&server.ctx.repo, Path::new(&session.worktree));
        task::load(
            &task::dir(&crate::infra::paths::state_dir(), &slug),
            task_id,
        )
        .ok()?
        .base
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(GIT_CHECK_SECS);
    crate::kernel::worktree_state::worktree_git_state(
        Path::new(&session.worktree),
        base.as_deref(),
        deadline,
    )
}

/// What one session's worktree holds that no remote has, asked when a person looks rather than
/// on every poll.
pub(super) fn session_git(server: &Server, id: &str) -> Result<Value, String> {
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    match git_state_of(server, &session)? {
        Some(state) => {
            serde_json::to_value(state).map_err(|e| format!("cannot describe the worktree: {e}"))
        }
        None => Err(format!("{} does not exist", session.worktree)),
    }
}
