//! What the board reads: the state document and what it is joined from.

use std::path::Path;

use serde::Serialize;

use crate::board::jobs::{JulesSeen, PollHealth};
use crate::board::{Server, Session, hub_resume_refusal, resume_refusal, settings_now};
use crate::gate;
use crate::infra::terminal;
use crate::kernel::identity::Worktree;
use crate::kernel::runner;
use crate::task;

use super::columns::{HumanCol, waits_on_person};
use super::sessions::sessions_of;

/// The state document `/api/state` sends. Every key the page reads is a field here, and the
/// transport only serializes it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardState {
    pub repo: String,
    pub main: String,
    pub hub_name: String,
    pub hub: HubPresence,
    pub hubs: Vec<crate::mail::RepoHub>,
    /// Whether the resident server serves this board, which is also what tells the page
    /// it lives under a path of its own.
    pub resident: bool,
    /// Whether the PR poll is running and whether it is failing, for the one line the page
    /// shows when it is. `null` where nothing polls. Read from memory: this is polled every
    /// couple of seconds and must not reach GitHub.
    pub pr_poll: Option<PollHealth>,
    /// Whether the board may start a hub: only where the settings mean a tmux window.
    pub hub_start: Available,
    /// Whether the board can open a terminal on a session that runs in tmux: the resident
    /// server, on a machine that has tmux. Which sessions is for the page to read from
    /// `sessions[].terminal` and `present`.
    pub board_terminal: Available,
    /// Whether the board can open a session in the person's own terminal, and through what:
    /// `terminal.attach` when it is set, iTerm2 where that is installed.
    pub session_open: SessionOpen,
    /// Whether the board can resume a stopped worker, so the page offers it only where it
    /// can work, and says why not where it cannot.
    pub session_resume: Availability,
    /// The same for restarting a running hub on its conversation, which needs the hub's own
    /// resume line rather than the worker's.
    pub hub_resume: Availability,
    /// The command line a hub runs, as configured: the server sends the template with its
    /// placeholders in place, and the Sessions sidebar fills in only `{name}` to show it.
    pub hub_runner: String,
    /// The agent a session started from the board runs, which is the only one its dialog offers.
    pub session_start: SessionStart,
    pub sessions: Vec<Session>,
    pub tasks: Vec<TaskCard>,
    /// The tasks of the parent-task hubs of this repository, which only the repository's own
    /// board lists: its workers include theirs, and a card for each says so. Apart from `tasks`
    /// because those are this board's own, which every action on the page assumes.
    pub hub_tasks: Vec<TaskCard>,
    pub workers: Vec<WorkerRow>,
    /// The slot count `adj work` decides by, counted the same way — a worker still
    /// starting up holds one — so the header and the refusal cannot disagree.
    pub worker_slots: WorkerSlots,
    /// Minutes in one phase before a card is flagged. `0` = never.
    pub stuck_after_minutes: f64,
    /// Whether the IDE buttons can do anything, and where to set it when they cannot. Read
    /// on every poll, so an `ide` written into the config shows up without a restart.
    pub ide_configured: bool,
    pub config_path: String,
    pub now: i64,
    pub pending: Vec<PendingRow>,
    pub gates: Vec<GateCard>,
}

/// The repository's own hub. Unlike `mail::RepoHubState`, `pid` and `startedAt` are written
/// as `null` when there is none.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubPresence {
    pub present: bool,
    pub stale: bool,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
}

/// A thing the board can or cannot do.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub available: bool,
}

/// A thing the board can or cannot do, and why not.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Availability {
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOpen {
    pub available: bool,
    pub terminal: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStart {
    pub agent: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSlots {
    pub busy: usize,
    pub max: Option<u32>,
}

/// One linked worktree and what its worker says.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerRow {
    pub worktree: String,
    pub name: Option<String>,
    pub branch: Option<String>,
    pub present: bool,
    pub stale: bool,
    pub title: Option<String>,
    /// The task this worker reports for, which is what the card joins on: a worker
    /// with none is a session that has no card until it is linked.
    pub task: Option<String>,
    /// The slug of the hub this worker reports to: a task id is only unique within one hub, so
    /// the page joins a worker to a card on this and the id together.
    pub hub_slug: String,
    pub phase: Option<String>,
    pub phase_at: Option<i64>,
}

/// A message waiting in the hub's inbox.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRow {
    pub name: String,
    pub subject: String,
    pub from: String,
    pub kind: String,
    pub worktree: Option<String>,
}

/// A task as the board's card: the record itself, with what is joined to it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCard {
    #[serde(flatten)]
    pub task: task::Task,
    /// Absent on a finished task.
    #[serde(flatten)]
    pub live: Option<LiveCard>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jules: Option<JulesSeen>,
    /// Whether its PR is the person's ball, see `waits_on_person`. On every card, false on a
    /// finished one; set by `state`, which has the worker rows and Jules's answer.
    pub waits_on_person: bool,
    /// Set on a card of a parent-task hub shown on the repository's board; absent on this
    /// board's own tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_hub: Option<OwnerHub>,
}

/// The parent-task hub a card on the repository's board belongs to.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerHub {
    pub slug: String,
    pub key: Option<String>,
    /// The column its open gate puts it in, else the PR's, as `humanColOf` reads on the page.
    pub human_col: Option<HumanCol>,
}

/// What a live task's card carries beyond the record.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveCard {
    pub records: Vec<RecordCard>,
    pub approved_plan: Option<gate::Gate>,
    pub pr_turn: Option<task::PrTurn>,
}

/// A record as the card carries it: the gate without its diff, and the diff's size.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordCard {
    #[serde(flatten)]
    pub gate: gate::Gate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_size: Option<usize>,
    pub answered_by_hub: bool,
}

impl RecordCard {
    fn of(record: &gate::Gate) -> Self {
        let mut gate = record.clone();
        let diff_size = gate.diff.take().map(|d| d.len());
        if diff_size.is_some() {
            // A key kept from disk that the card writes itself: the card's value wins.
            gate.extra.remove("diffSize");
        }
        let answered_by_hub = gate.answered_by_hub();
        // Written on every record, so a key kept from disk gives way to the card's value.
        gate.extra.remove("answeredByHub");
        RecordCard {
            gate,
            diff_size,
            answered_by_hub,
        }
    }
}

/// An open gate as the board sends it. The column is on the gate rather than the card so that
/// the page, dropping a gate it has just answered, moves the card at once.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateCard {
    #[serde(flatten)]
    pub gate: gate::Gate,
    pub human_col: HumanCol,
    pub answered_by_hub: bool,
}

impl GateCard {
    pub fn of(mut gate: gate::Gate) -> Self {
        // Keys kept from disk that the card writes itself: the card's value wins.
        for key in ["humanCol", "answeredByHub"] {
            gate.extra.remove(key);
        }
        GateCard {
            human_col: HumanCol::of_gate(gate.kind),
            answered_by_hub: gate.answered_by_hub(),
            gate,
        }
    }
}

/// `with_sessions` false leaves `sessions` empty: the page that merges several boards has no
/// use for them, and listing them is the dearest part of a poll. `with_lines` adds each
/// session's last line of output (`lastLine`), which reads its tmux pane: only the page that
/// shows it asks.
pub fn state(server: &Server, with_sessions: bool, with_lines: bool) -> BoardState {
    let repo = &server.ctx.repo;
    let tasks = with_records(
        task::list(&server.ctx.state, &server.ctx.repo.slug),
        gate::list(
            &server.ctx.state,
            &server.ctx.repo.slug,
            gate::Shelf::Record,
        ),
        gate::list_of_kind(
            &server.ctx.state,
            &server.ctx.repo.slug,
            gate::Shelf::Answered,
            gate::Kind::Plan,
        ),
    );

    let now = crate::infra::clock::now_secs();
    let settings = settings_now(server);
    // After the records are joined, from the same values the page gets: a card shows the last
    // answer about its session, and an old answer is asked again behind the page's back.
    let mut tasks: Vec<TaskCard> = tasks
        .into_iter()
        .map(|mut card| {
            card.jules = server
                .jules
                .look(&server.ctx, &settings.jules_key, &card.task);
            if card.jules.is_some() {
                // A key kept from disk that the card writes itself: the card's value wins.
                card.task.extra.remove("jules");
            }
            card
        })
        .collect();
    let shown: std::collections::HashSet<String> = tasks
        .iter()
        .filter_map(|card| card.jules.as_ref().map(|j| j.session().to_string()))
        .collect();
    server.jules.keep_only(&shown);
    // One `git worktree list` and one `ps` serve every question below, so what a poll costs
    // does not grow with the number of worktrees. The `ps` is only run if a record names a pid.
    let processes = crate::registry::ProcessTable::snapshot();
    // The board shows what it can; `adj work` is the one that refuses on a failed listing.
    let listed = crate::kernel::identity::worktrees(&repo.main).unwrap_or_default();
    let (main_branch, linked) = split_main(&repo.main, listed);
    let linked_paths: Vec<String> = linked.iter().map(|w| w.path.clone()).collect();
    // Counted as `adj work` counts, main checkout included, though it is not listed below.
    let mut busy = usize::from(crate::registry::holds_worker_slot_with(
        &processes,
        Path::new(&repo.main),
        now,
    ));
    let mut hubs =
        crate::mail::all_repo_hubs_among_with(&server.ctx.state, &processes, repo, &linked_paths);
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
            let status = crate::registry::hub_status_with(
                &server.ctx.state,
                &processes,
                &repo.slug,
                &repo.hub_name,
            );
            crate::mail::RepoHubState {
                present: status.present,
                stale: status.stale,
                pid: status.pid,
                started_at: status.started_at,
            }
        });
    let mut workers_data = Vec::with_capacity(linked.len());
    let mut workers: Vec<WorkerRow> = Vec::with_capacity(linked.len());
    for Worktree { path, branch } in &linked {
        let status = crate::registry::worker_status_with(&processes, Path::new(path));
        // A present worker holds a slot without asking `ps` again; the rest are asked
        // the way `adj work` asks, so the header and the refusal cannot disagree.
        if status.present
            || crate::registry::holds_worker_slot_with(&processes, Path::new(path), now)
        {
            busy += 1;
        }
        let branch = branch.clone();
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        let task = crate::registry::worker_task(Path::new(path));
        workers.push(WorkerRow {
            worktree: path.clone(),
            name,
            branch: branch.clone(),
            present: status.present,
            stale: status.stale,
            title: status.title.clone(),
            task,
            hub_slug: crate::kernel::identity::slug_for(
                &repo.nwo,
                crate::registry::worker_hub_key(Path::new(path)).as_deref(),
            ),
            phase: status.phase.clone(),
            phase_at: status.phase_at,
        });
        workers_data.push((status, branch));
    }

    // Task ids are unique only per hub, so a card joins only the workers that report to this hub.
    let own: Vec<WorkerRow> = workers
        .iter()
        .filter(|w| w.hub_slug == repo.slug)
        .cloned()
        .collect();
    for card in &mut tasks {
        let row = worker_of(&card.task, &own);
        card.waits_on_person = waits_on_person(
            &card.task,
            row.map(|w| w.phase.as_deref()),
            card.jules.as_ref(),
        );
    }

    let hub_tasks = hub_task_cards(&server.ctx.state, repo, &hubs, &workers);

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

    let pending: Vec<PendingRow> = crate::mail::pending(&server.ctx.state, &repo.slug)
        .messages
        .iter()
        .map(|entry| PendingRow {
            name: entry.name.clone(),
            subject: entry.subject.clone(),
            from: entry.from.clone(),
            kind: entry.kind.clone(),
            worktree: entry.worktree.clone(),
        })
        .collect();

    BoardState {
        repo: repo.nwo.clone(),
        main: repo.main.clone(),
        hub_name: repo.hub_name.clone(),
        hub: HubPresence {
            present: hub.present,
            stale: hub.stale,
            pid: hub.pid,
            started_at: hub.started_at,
        },
        hubs,
        resident: server.resident,
        pr_poll: server.pr_poll.as_ref().map(|p| p.health()),
        hub_start: Available {
            available: crate::lifecycle::hub::hub_startable(&settings.terminal),
        },
        board_terminal: Available {
            available: server.resident && server.tmux.is_some(),
        },
        session_open: open_state(server, &settings),
        session_resume: resume_state(&settings),
        hub_resume: hub_resume_state(&settings),
        hub_runner: settings
            .hub_runner
            .as_deref()
            .unwrap_or(runner::DEFAULT_HUB_RUNNER)
            .to_string(),
        session_start: SessionStart {
            agent: runner::agent_from_runner(
                settings
                    .agent_runner
                    .as_deref()
                    .unwrap_or(runner::DEFAULT_AGENT_RUNNER),
            ),
        },
        sessions,
        tasks,
        hub_tasks,
        workers,
        worker_slots: WorkerSlots {
            busy,
            max: settings.max_workers,
        },
        stuck_after_minutes: settings.stuck_after_minutes,
        ide_configured: crate::infra::ide::configured(settings.ide.as_deref()),
        config_path: crate::kernel::config::config_path()
            .to_string_lossy()
            .to_string(),
        now,
        pending,
        gates: gate::list(&server.ctx.state, &server.ctx.repo.slug, gate::Shelf::Open)
            .into_iter()
            .map(GateCard::of)
            .collect(),
    }
}

/// The tasks of the parent-task hubs of this repository, for the repository's own board: the
/// workers it lists include theirs, and each should have a card rather than a session that
/// names a task nobody here knows. Read-only, from the records of each hub's directory, and
/// empty on a parent-task hub's own board, which shows only its own tasks. A hub whose
/// directory cannot be read has no cards, not an error. No Jules lookup: that answer belongs
/// to the board that owns the task.
pub fn hub_task_cards(
    state_dir: &Path,
    repo: &crate::kernel::identity::RepoInfo,
    hubs: &[crate::mail::RepoHub],
    workers: &[WorkerRow],
) -> Vec<TaskCard> {
    if repo.hub.is_some() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for h in hubs.iter().filter(|h| h.parent && h.slug != repo.slug) {
        let open = gate::list(state_dir, &h.slug, gate::Shelf::Open);
        let cards = with_records(
            task::list(state_dir, &h.slug),
            gate::list(state_dir, &h.slug, gate::Shelf::Record),
            gate::list_of_kind(state_dir, &h.slug, gate::Shelf::Answered, gate::Kind::Plan),
        );
        let own: Vec<WorkerRow> = workers
            .iter()
            .filter(|w| w.hub_slug == h.slug)
            .cloned()
            .collect();
        for mut card in cards {
            card.waits_on_person = waits_on_person(
                &card.task,
                worker_of(&card.task, &own).map(|w| w.phase.as_deref()),
                None,
            );
            let human_col = open
                .iter()
                .find(|g| g.task.as_deref() == Some(card.task.id.as_str()))
                .map(|g| HumanCol::of_gate(g.kind))
                .or_else(|| card.waits_on_person.then_some(HumanCol::PrReview));
            card.owner_hub = Some(OwnerHub {
                slug: h.slug.clone(),
                key: h.key.clone(),
                human_col,
            });
            out.push(card);
        }
    }
    out
}

/// The worker row the page joins a task to (`workerOf`): the first in the task's worktree that
/// names no task or this one.
pub fn worker_of<'a>(task: &task::Task, workers: &'a [WorkerRow]) -> Option<&'a WorkerRow> {
    let worktree = task.worktree.as_deref()?;
    workers
        .iter()
        .find(|w| w.worktree == worktree && w.task.as_deref().is_none_or(|id| id == task.id))
}

/// What one poll has already asked of the system, so `sessions_of` does not ask again: the
/// process table, and the branch the main checkout's listing entry names.
pub(super) struct Listing<'a> {
    pub(super) processes: &'a crate::registry::ProcessTable,
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
pub fn with_records(
    tasks: Vec<task::Task>,
    records: Vec<gate::Gate>,
    answered: Vec<gate::Gate>,
) -> Vec<TaskCard> {
    tasks
        .into_iter()
        .map(|t| {
            let live =
                (!matches!(t.status, task::Status::Done | task::Status::Cancelled)).then(|| {
                    let mine = |g: &&gate::Gate| g.task.as_deref() == Some(t.id.as_str());
                    // The latest, because a plan sent back with `changes` is opened again, and the
                    // one that was approved last is the one being worked to.
                    let plan = answered
                        .iter()
                        .filter(mine)
                        .filter(|g| g.kind == gate::Kind::Plan)
                        .filter(|g| matches!(g.decision.as_deref(), Some("approve" | "choice")))
                        .max_by(|a, b| a.answered_at.cmp(&b.answered_at));
                    LiveCard {
                        // Without their diffs, which are most of what a poll weighs: the page reads
                        // one from the task's history when it shows it, and `diffSize` says it is there.
                        records: records.iter().filter(mine).map(RecordCard::of).collect(),
                        approved_plan: plan.cloned(),
                        // Whose turn the PR is, from what the last read kept on the record. Derived
                        // here, on every poll, so the rule can change without rewriting a record.
                        pr_turn: t.pr_status.as_ref().and_then(task::pr_turn),
                    }
                });
            let mut t = t;
            if live.is_some() {
                // Keys kept from disk that the card writes itself: the card's value wins.
                for key in ["records", "approvedPlan", "prTurn"] {
                    t.extra.remove(key);
                }
            }
            // Written on every card, finished ones too.
            for key in ["waitsOnPerson", "ownerHub"] {
                t.extra.remove(key);
            }
            TaskCard {
                task: t,
                live,
                jules: None,
                waits_on_person: false,
                owner_hub: None,
            }
        })
        .collect()
}

pub fn branch_of(worktree: &str) -> Option<String> {
    let output =
        crate::infra::git::git(&["-C", worktree, "branch", "--show-current"], None).ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// What `state` says about opening a session in the person's own terminal: whether the board
/// can, and through what. Known in advance so the page does not offer a button that can only
/// be refused.
pub(super) fn open_state(
    server: &Server,
    settings: &crate::kernel::config::Settings,
) -> SessionOpen {
    let attach = settings.terminal.attach.is_some();
    let iterm = terminal::iterm_available();
    SessionOpen {
        available: server.resident && server.tmux.is_some() && (attach || iterm),
        terminal: match (attach, iterm) {
            (true, _) => Some("terminal.attach"),
            (false, true) => Some("iTerm2"),
            (false, false) => None,
        },
    }
}

/// What `state` says about resuming: `available`, and the reason when it is not.
pub(super) fn resume_state(settings: &crate::kernel::config::Settings) -> Availability {
    let refusal = resume_refusal(settings);
    Availability {
        available: refusal.is_none(),
        reason: refusal,
    }
}

/// What `state` says about resuming a hub: `available`, and the reason when it is not.
pub(super) fn hub_resume_state(settings: &crate::kernel::config::Settings) -> Availability {
    let refusal = hub_resume_refusal(settings);
    Availability {
        available: refusal.is_none(),
        reason: refusal,
    }
}
