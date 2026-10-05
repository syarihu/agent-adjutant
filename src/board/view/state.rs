//! What the board reads: the state document and the caches behind it.

use std::path::Path;

use serde_json::{Value, json};

use crate::board::{Server, hub_resume_refusal, resume_refusal, settings_now};
use crate::gate;
use crate::infra::terminal;
use crate::kernel::identity::Worktree;
use crate::kernel::runner;
use crate::task;

use super::sessions::sessions_of;

/// `with_sessions` false leaves `sessions` empty: the page that merges several boards has no
/// use for them, and listing them is the dearest part of a poll. `with_lines` adds each
/// session's last line of output (`lastLine`), which reads its tmux pane: only the page that
/// shows it asks.
pub fn state(server: &Server, with_sessions: bool, with_lines: bool) -> Value {
    // Before the gates are read: a gate whose worker has moved on is closed here rather than
    // by a timer, since nothing in the server polls on one.
    let _ = crate::gate::close_resumed(&server.ctx);
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
    let mut workers: Vec<Value> = Vec::with_capacity(linked.len());
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

    let pending: Vec<Value> = crate::mail::pending(&server.ctx.state, &repo.slug)
        .messages
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
        "hubStart": { "available": crate::lifecycle::hub::hub_startable(&settings.terminal) },
        // Whether the board can open a terminal on a session that runs in tmux: the resident
        // server, on a machine that has tmux. Which sessions is for the page to read from
        // `sessions[].terminal` and `present`.
        "boardTerminal": { "available": server.resident && server.tmux.is_some() },
        // Whether the board can open a session in the person's own terminal, and through what:
        // `terminal.attach` when it is set, iTerm2 where that is installed.
        "sessionOpen": open_state(server, &settings),
        // Whether the board can resume a stopped worker, so the page offers it only where it
        // can work, and says why not where it cannot.
        "sessionResume": resume_state(&settings),
        // The same for restarting a running hub on its conversation, which needs the hub's own
        // resume line rather than the worker's.
        "hubResume": hub_resume_state(&settings),
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
        "gates": gate::list(&server.ctx.state, &server.ctx.repo.slug, gate::Shelf::Open),
    })
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

pub fn branch_of(worktree: &str) -> Option<String> {
    let output =
        crate::infra::git::git(&["-C", worktree, "branch", "--show-current"], None).ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// What `state` says about opening a session in the person's own terminal: whether the board
/// can, and through what. Known in advance so the page does not offer a button that can only
/// be refused.
pub(super) fn open_state(server: &Server, settings: &crate::kernel::config::Settings) -> Value {
    let attach = settings.terminal.attach.is_some();
    let iterm = terminal::iterm_available();
    json!({
        "available": server.resident && server.tmux.is_some() && (attach || iterm),
        "terminal": match (attach, iterm) {
            (true, _) => json!("terminal.attach"),
            (false, true) => json!("iTerm2"),
            (false, false) => Value::Null,
        },
    })
}

/// What `state` says about resuming: `available`, and the reason when it is not.
pub(super) fn resume_state(settings: &crate::kernel::config::Settings) -> Value {
    let refusal = resume_refusal(settings);
    json!({ "available": refusal.is_none(), "reason": refusal })
}

/// What `state` says about resuming a hub: `available`, and the reason when it is not.
pub(super) fn hub_resume_state(settings: &crate::kernel::config::Settings) -> Value {
    let refusal = hub_resume_refusal(settings);
    json!({ "available": refusal.is_none(), "reason": refusal })
}
