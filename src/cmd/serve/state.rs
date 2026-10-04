//! What the board reads: the state document and the caches behind it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::gate;
use crate::kernel::identity::Worktree;
use crate::kernel::runner;
use crate::messaging;
use crate::session;
use crate::task;

use super::Server;
use super::sessions::{TmuxView, sessions_of, tmux_view};

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
    hubs: &[session::RepoHub],
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
    let title = task::load(&task::dir(state_dir, slug), id).ok()?.title;
    Some(title.trim().to_string()).filter(|t| !t.is_empty())
}

/// The board ids of the workers in `paths`, in order: `worker-<name>` for the worktree's own
/// name, and `worker-<name>-<digest of the path>` when another of them has that name. A
/// worktree called `main` keeps `worker-main` unless the main checkout's own session
/// (`main_listed`) is on the board and has it. A name that is not shared keeps the id it always
/// had, so nothing that already holds one is told a new one.
pub(super) fn worker_session_ids(paths: &[String], main_listed: bool) -> Vec<String> {
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

/// `with_sessions` false leaves `sessions` empty: the page that merges several boards has no
/// use for them, and listing them is the dearest part of a poll. `with_lines` adds each
/// session's last line of output (`lastLine`), which reads its tmux pane: only the page that
/// shows it asks.
pub(super) fn state(server: &Server, with_sessions: bool, with_lines: bool) -> Value {
    // Before the gates are read: a gate whose worker has moved on is closed here rather than
    // by a timer, since nothing in the server polls on one.
    let _ = crate::cmd::gate::close_resumed(&server.ctx);
    let repo = &server.ctx.repo;
    let tasks = with_records(
        task::list(&crate::cmd::task::dir(&server.ctx)),
        gate::list(&crate::cmd::gate::records_dir(&server.ctx)),
        gate::list_of_kind(
            &crate::cmd::gate::answered_dir(&server.ctx),
            gate::Kind::Plan,
        ),
    );

    let now = crate::infra::clock::now_secs();
    let settings = settings_now(server);
    // After the records are joined, from the same values the page gets: a card shows the last
    // answer about its session, and an old answer is asked again behind the page's back.
    let tasks: Vec<Value> = tasks
        .into_iter()
        .map(|mut t| {
            if let Some(seen) = server.jules.look(&server.ctx, &settings.jules_key, &t) {
                t["jules"] = seen;
            }
            t
        })
        .collect();
    let shown: std::collections::HashSet<String> = tasks
        .iter()
        .filter_map(|t| t["jules"]["session"].as_str().map(str::to_string))
        .collect();
    server.jules.keep_only(&shown);
    // One `git worktree list` and one `ps` serve every question below, so what a poll costs
    // does not grow with the number of worktrees. The `ps` is only run if a record names a pid.
    let processes = messaging::ProcessTable::snapshot();
    // The board shows what it can; `adj work` is the one that refuses on a failed listing.
    let listed = crate::kernel::identity::worktrees(&repo.main).unwrap_or_default();
    let (main_branch, linked) = split_main(&repo.main, listed);
    let linked_paths: Vec<String> = linked.iter().map(|w| w.path.clone()).collect();
    // Counted as `adj work` counts, main checkout included, though it is not listed below.
    let mut busy = usize::from(messaging::holds_worker_slot_with(
        &processes,
        Path::new(&repo.main),
        now,
    ));
    let mut hubs = messaging::all_repo_hubs_among_with(&processes, repo, &linked_paths);
    let slugs: Vec<String> = hubs.iter().map(|h| h.slug.clone()).collect();
    for h in &mut hubs {
        h.title = server.hub_titles.look(&server.ctx, h, &slugs);
    }
    server
        .hub_titles
        .keep_only(&slugs.iter().cloned().collect());
    // The repository's own hub is one of `hubs`; asked separately only if it is not there.
    let hub = hubs
        .iter()
        .find(|h| h.slug == repo.slug)
        .map(|h| h.state.clone())
        .unwrap_or_else(|| {
            let status = messaging::hub_status_with(&processes, &repo.slug, &repo.hub_name);
            session::RepoHubState {
                present: status.present,
                stale: status.stale,
                pid: status.pid,
                started_at: status.started_at,
            }
        });
    let mut workers_data = Vec::with_capacity(linked.len());
    let mut workers: Vec<Value> = Vec::with_capacity(linked.len());
    for Worktree { path, branch } in &linked {
        let status = messaging::worker_status_with(&processes, Path::new(path));
        // A present worker holds a slot without asking `ps` again; the rest are asked
        // the way `adj work` asks, so the header and the refusal cannot disagree.
        if status.present || messaging::holds_worker_slot_with(&processes, Path::new(path), now) {
            busy += 1;
        }
        let branch = branch.clone();
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        let task = messaging::worker_task(Path::new(path));
        workers.push(json!({
            "worktree": path,
            "name": name,
            "branch": branch.clone(),
            "present": status.present,
            "stale": status.stale,
            "title": status.title,
            // The task this worker reports for, which is what the card joins on: a worker
            // with none is a session that has no card until it is linked.
            "task": task,
            "phase": status.phase,
            "phaseAt": status.phase_at,
        }));
        workers_data.push((status, branch));
    }

    let sessions = if with_sessions {
        sessions_of(
            server,
            &settings,
            &hubs,
            &linked_paths,
            Listing {
                processes: &processes,
                main_branch,
                with_lines,
            },
            None,
            |index, _| workers_data[index].clone(),
        )
    } else {
        Vec::new()
    };

    let pending: Vec<Value> = messaging::list(&repo.slug)
        .iter()
        .map(|entry| {
            json!({
                "name": entry.name,
                "subject": entry.subject,
                "from": entry.from,
                "kind": entry.kind,
                "worktree": entry.worktree,
            })
        })
        .collect();

    json!({
        "repo": repo.nwo,
        "main": repo.main,
        "hubName": repo.hub_name,
        "hub": {
            "present": hub.present,
            "stale": hub.stale,
            "pid": hub.pid,
            "startedAt": hub.started_at,
        },
        "hubs": hubs,
        // Whether the resident server serves this board, which is also what tells the page
        // it lives under a path of its own.
        "resident": server.resident,
        // Whether the PR poll is running and whether it is failing, for the one line the page
        // shows when it is. `null` where nothing polls. Read from memory: this is polled every
        // couple of seconds and must not reach GitHub.
        "prPoll": server.pr_poll.as_ref().map(|p| p.health_json()),
        // Whether the board may start a hub: only where the settings mean a tmux window.
        "hubStart": { "available": crate::cmd::hub_startable(&settings.terminal) },
        // Whether the board can open a terminal on a session that runs in tmux: the resident
        // server, on a machine that has tmux. Which sessions is for the page to read from
        // `sessions[].terminal` and `present`.
        "boardTerminal": { "available": server.resident && server.tmux.is_some() },
        // Whether the board can open a session in the person's own terminal, and through what:
        // `terminal.attach` when it is set, iTerm2 where that is installed.
        "sessionOpen": crate::cmd::board_actions::open_state(server, &settings),
        // Whether the board can resume a stopped worker, so the page offers it only where it
        // can work, and says why not where it cannot.
        "sessionResume": crate::cmd::board_actions::resume_state(&settings),
        // The same for restarting a running hub on its conversation, which needs the hub's own
        // resume line rather than the worker's.
        "hubResume": crate::cmd::board_actions::hub_resume_state(&settings),
        // The command line a hub runs, as configured: the server sends the template with its
        // placeholders in place, and the Sessions sidebar fills in only `{name}` to show it.
        "hubRunner": settings
            .hub_runner
            .as_deref()
            .unwrap_or(runner::DEFAULT_HUB_RUNNER),
        // The agent a session started from the board runs, which is the only one its dialog offers.
        "sessionStart": {
            "agent": runner::agent_from_runner(
                settings.agent_runner.as_deref().unwrap_or(runner::DEFAULT_AGENT_RUNNER),
            ),
        },
        "sessions": sessions,
        "tasks": tasks,
        "workers": workers,
        // The slot count `adj work` decides by, counted the same way — a worker still
        // starting up holds one — so the header and the refusal cannot disagree.
        "workerSlots": {
            "busy": busy,
            "max": settings.max_workers,
        },
        // Minutes in one phase before a card is flagged. `0` = never.
        "stuckAfterMinutes": settings.stuck_after_minutes,
        // Whether the IDE buttons can do anything, and where to set it when they cannot. Read
        // on every poll, so an `ide` written into the config shows up without a restart.
        "ideConfigured": crate::infra::ide::configured(settings.ide.as_deref()),
        "configPath": crate::kernel::config::config_path().to_string_lossy(),
        "now": now,
        "pending": pending,
        "gates": gate::list(&crate::cmd::gate::dir(&server.ctx)),
    })
}

/// A tmux socket as a lookup key: the path of the server it names, so that no setting, a bare
/// name and the path a record kept for the same server are one key and one pair of `list-*`
/// calls. The directory is resolved when it can be, because `/tmp` is `/private/tmp` on a Mac.
pub(super) fn socket_key(socket: Option<&str>) -> PathBuf {
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
pub(super) fn socket_key_in(
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

/// How soon a pane's screen is read again. A busy agent moves its window's activity on every
/// poll, and reading its screen that often would be most of what a poll costs.
pub(super) const LAST_LINE_MIN_AGE: Duration = Duration::from_secs(5);

/// A pane's last line, with the window activity it was read at and when.
struct LastRead {
    activity: Option<i64>,
    at: Instant,
    /// The epoch second it was read in, against a window activity that has one-second
    /// resolution: activity in the same second may have come after the read.
    secs: i64,
    line: Option<String>,
}

/// The last line of output of each pane a page has asked for, by pane.
#[derive(Default)]
pub(super) struct LastLines {
    read: Mutex<HashMap<String, LastRead>>,
}

impl LastLines {
    /// The line of `key`, which `read` produces only when the window has had activity since
    /// the last read and that was at least `LAST_LINE_MIN_AGE` ago.
    pub(super) fn look(
        &self,
        key: &str,
        activity: Option<i64>,
        now: Instant,
        secs: i64,
        read: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        if let Some(last) = self.read.lock().ok()?.get(key) {
            let recent = now.duration_since(last.at) < LAST_LINE_MIN_AGE;
            // Unknown activity says nothing of whether the pane moved, and a read that found
            // nothing may only have been early: both are read again once the age is up.
            let unchanged = last.line.is_some()
                && activity.is_some_and(|a| last.activity == Some(a) && last.secs > a);
            if recent || unchanged {
                return last.line.clone();
            }
        }
        // Not under the lock: reading the pane runs a command.
        let line = read();
        if let Ok(mut all) = self.read.lock() {
            all.insert(
                key.to_string(),
                LastRead {
                    activity,
                    at: now,
                    secs,
                    line: line.clone(),
                },
            );
        }
        line
    }

    /// Forgets the panes that are no longer listed.
    pub(super) fn keep_only(&self, keys: &HashSet<String>) {
        if let Ok(mut all) = self.read.lock() {
            all.retain(|key, _| keys.contains(key));
        }
    }
}

fn session_waiting(
    hub: &session::RepoHub,
    open: &[&gate::Gate],
) -> Option<session::SessionWaiting> {
    let first = open.first()?;
    Some(session::SessionWaiting {
        id: first.id.clone(),
        kind: first.kind.as_str().to_string(),
        hub: hub.id.clone(),
        slug: hub.slug.clone(),
        title: Some(first.title.clone()).filter(|t| !t.is_empty()),
        opened_at: first.opened_at.clone(),
        count: open.len(),
        options: if first.options.is_empty() {
            first.kind.default_options()
        } else {
            first.options.clone()
        },
        choices: first
            .choices
            .iter()
            .map(|c| session::WaitingChoice {
                id: c.id.clone(),
                label: c.label.clone(),
            })
            .collect(),
        focus: first
            .focus
            .as_deref()
            .map(|f| cut_chars(f, WAITING_FOCUS_CHARS))
            .filter(|f| !f.is_empty()),
    })
}

/// How much of a gate's focus the Sessions banner carries.
const WAITING_FOCUS_CHARS: usize = 400;

/// `text` cut to at most `max` characters, on a character boundary, with an ellipsis when cut.
pub(super) fn cut_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text.to_string(),
    }
}

/// One hub's open gates, and what could show their workers moved on, read once per poll.
pub(super) struct HubGates {
    pub(super) open: Vec<gate::Gate>,
    signals: Vec<gate::Gate>,
}

impl HubGates {
    fn read(state_dir: &Path, slug: &str) -> Self {
        let dir = gate::dir(state_dir, slug);
        let open = gate::list(&dir);
        // Left unread when nothing waits on a worker: the archives only grow.
        let signals = if open.iter().any(|g| g.wait && !g.answered_by_hub()) {
            gate::resume_signals(
                &open,
                &dir,
                &gate::records_dir(state_dir, slug),
                &gate::answered_dir(state_dir, slug),
            )
        } else {
            Vec::new()
        };
        HubGates { open, signals }
    }
}

/// The gates of the hubs a poll reaches, each hub's read the first time one of its sessions
/// asks.
pub(super) struct GateCache {
    pub(super) state_dir: PathBuf,
    pub(super) read: HashMap<String, HubGates>,
}

impl GateCache {
    pub(super) fn of(&mut self, slug: &str) -> &HubGates {
        self.read
            .entry(slug.to_string())
            .or_insert_with(|| HubGates::read(&self.state_dir, slug))
    }
}

/// The gate a worker is waiting to have answered: the oldest still open in its hub's gate
/// directory that it opened from `worktree` and has not moved on from. "Moved on" is judged as
/// `close_resumed` judges it, from the same signals, but only reads: a hub's directory is
/// closed by that hub's board.
pub(super) fn waiting_worker(
    hub: &session::RepoHub,
    gates: &HubGates,
    worktree: &str,
    started: Option<&str>,
    phase_at: Option<i64>,
) -> Option<session::SessionWaiting> {
    let phase_at = phase_at.map(crate::infra::clock::utc_stamp);
    let open: Vec<&gate::Gate> = gates
        .open
        .iter()
        .filter(|g| g.worktree == worktree && g.wait && !g.answered_by_hub())
        .filter(|g| {
            let later = gates
                .signals
                .iter()
                .filter(|s| s.worktree == g.worktree && s.id != g.id && s.opened_at > g.opened_at)
                .map(|s| s.opened_at.as_str())
                .min();
            gate::resumed_at(g, started, phase_at.as_deref(), later).is_none()
        })
        .collect();
    session_waiting(hub, &open)
}

/// What a hub is waiting on: the gates it opened for a person to answer.
pub(super) fn waiting_hub(
    hub: &session::RepoHub,
    gates: &[gate::Gate],
) -> Option<session::SessionWaiting> {
    let open: Vec<&gate::Gate> = gates
        .iter()
        .filter(|g| g.wait && g.answered_by_hub())
        .collect();
    session_waiting(hub, &open)
}

/// What one poll has already asked of the system, so `sessions_of` does not ask again: the
/// process table, and the branch the main checkout's listing entry names.
pub(super) struct Listing<'a> {
    pub(super) processes: &'a messaging::ProcessTable,
    pub(super) main_branch: Option<String>,
    /// Whether each session carries the last line of its pane: reading it runs a command per
    /// session, so only the page that shows it asks.
    pub(super) with_lines: bool,
}

/// The main checkout's branch and the linked worktrees, out of one listing. The branch is
/// asked of git when the listing does not name the main checkout at all.
pub(super) fn split_main(main: &str, listed: Vec<Worktree>) -> (Option<String>, Vec<Worktree>) {
    let mut main_branch = None;
    let mut found_main = false;
    let mut linked = Vec::with_capacity(listed.len());
    for worktree in listed {
        if Path::new(&worktree.path) == Path::new(main) {
            found_main = true;
            main_branch = worktree.branch;
        } else {
            linked.push(worktree);
        }
    }
    if !found_main {
        main_branch = branch_of(main);
    }
    (main_branch, linked)
}

/// The tasks as the board reads them, each live one with what its worker recorded without
/// stopping (`records`, oldest first, each with its diff's byte length as `diffSize` in place
/// of the diff) and the plan a person approved (`approvedPlan`, whose
/// `answeredAt` is when).
///
/// Joined here rather than written onto the task record: a record belongs to the gate
/// directory, and a copy on the task would be a second place for it that can disagree.
/// A finished task gets neither — nobody reads its card for them, and the archive only grows.
pub(super) fn with_records(
    tasks: Vec<task::Task>,
    records: Vec<gate::Gate>,
    answered: Vec<gate::Gate>,
) -> Vec<Value> {
    tasks
        .into_iter()
        .filter_map(|t| {
            let live = !matches!(t.status, task::Status::Done | task::Status::Cancelled);
            let mut value = serde_json::to_value(&t).ok()?;
            if live {
                let mine = |g: &&gate::Gate| g.task.as_deref() == Some(t.id.as_str());
                let records: Vec<&gate::Gate> = records.iter().filter(mine).collect();
                // The latest, because a plan sent back with `changes` is opened again, and the
                // one that was approved last is the one being worked to.
                let plan = answered
                    .iter()
                    .filter(mine)
                    .filter(|g| g.kind == gate::Kind::Plan)
                    .filter(|g| matches!(g.decision.as_deref(), Some("approve" | "choice")))
                    .max_by(|a, b| a.answered_at.cmp(&b.answered_at));
                // Without their diffs, which are most of what a poll weighs: the page reads
                // one from the task's history when it shows it, and `diffSize` says it is there.
                value["records"] = records
                    .into_iter()
                    .map(|r| {
                        let mut record = json!(r);
                        if let Some(diff) = record.as_object_mut().and_then(|f| f.remove("diff"))
                            && let Some(diff) = diff.as_str()
                        {
                            record["diffSize"] = json!(diff.len());
                        }
                        record
                    })
                    .collect();
                value["approvedPlan"] = json!(plan);
                // Whose turn the PR is, from what the last read kept on the record. Derived
                // here, on every poll, so the rule can change without rewriting a record.
                value["prTurn"] = json!(t.pr_status.as_ref().and_then(task::pr_turn));
            }
            Some(value)
        })
        .collect()
}

/// Everything one task's gates left behind, for its full view: the gates a person answered
/// (`answered`) and the records its worker kept (`records`), each oldest first.
///
/// Asked for by the page when it opens the view rather than joined into `/api/state`: the
/// archive only grows, and reading all of it on every poll would cost more each day. The
/// records are here as well as on the task in `/api/state` because a finished task's are not
/// there, and a review is meant to stay readable after the work is done.
pub(super) fn task_history(server: &Server, path: &str) -> Result<Value, String> {
    let id = history_id(path).ok_or("no such task")?;
    Ok(history_of(
        id,
        gate::list(&crate::cmd::gate::answered_dir(&server.ctx)),
        gate::list(&crate::cmd::gate::records_dir(&server.ctx)),
    ))
}

/// The task id in `/api/tasks/{id}/history`, when there is exactly one.
pub(super) fn history_id(path: &str) -> Option<&str> {
    path.strip_prefix("/api/tasks/")
        .and_then(|rest| rest.strip_suffix("/history"))
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

pub(super) fn history_of(id: &str, answered: Vec<gate::Gate>, records: Vec<gate::Gate>) -> Value {
    let mine = |g: &gate::Gate| g.task.as_deref() == Some(id);
    json!({
        "answered": answered.into_iter().filter(mine).collect::<Vec<_>>(),
        "records": records.into_iter().filter(mine).collect::<Vec<_>>(),
    })
}

/// The settings as `adj work` would read them now. Resolved on every poll rather than taken
/// from the ones the server started with, because `adj work` reads the config each time it
/// runs, and a limit changed under a running board would otherwise show one number while
/// dispatches are refused by another.
pub(in crate::cmd) fn settings_now(server: &Server) -> crate::kernel::config::Settings {
    crate::kernel::config::resolve_config(&server.ctx.repo.nwo)
        .map(|resolved| resolved.settings)
        .unwrap_or_else(|_| server.ctx.settings.clone())
}

pub(super) fn branch_of(worktree: &str) -> Option<String> {
    let output =
        crate::infra::git::git(&["-C", worktree, "branch", "--show-current"], None).ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}
