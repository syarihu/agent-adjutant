//! What a person does with a session from the board besides looking at it: reopen one that
//! ended, restart one that is running on the same conversation, open one in their own
//! terminal, remove the worktree of one that is finished, and start
//! a parent-task hub for a key nobody has started one for.
//!
//! Only the resident server serves these (see `route`): each reaches outside the repository's
//! own records, and a board a hub serves lives and dies with that hub. The HTTP layer only
//! routes. The session a request names is looked up in the board's own records, and the
//! worktree, socket and window come from there and never from the request.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::board_terminal::target_of;
use super::serve::{Server, find_session, git_state_of, hub_start_of, settings_now};
use super::session::{input_of, text};
use super::{Context, Resumed, TabOutcome, same_path};
use crate::infra::template::{Sub, render, sh_join, sh_quote};
use crate::infra::terminal;
use crate::kernel::runner;
use crate::kernel::worktree_state::GitState;
use crate::messaging;
use crate::task::{self, Executor, Status};

/// The context of the repository's own hub, as `adj work --resume` builds one, but from what the
/// server holds: the server's directory is nowhere near the repository, so nothing here may
/// resolve anything from it.
fn own_hub_context(
    server: &Server,
    settings: crate::kernel::config::Settings,
) -> Result<Context, String> {
    Ok(Context {
        repo: server.ctx.repo.clone().addressed(None)?,
        resolved: server.ctx.resolved.clone(),
        state: server.ctx.state.clone(),
        settings,
    })
}

// ── resume ───────────────────────────────────────────────────────────

/// Why no session can be resumed from the board with these settings, or `None` when one can.
/// The board-wide half of `resume`'s refusals, known without looking at a session.
pub(super) fn resume_refusal(settings: &crate::kernel::config::Settings) -> Option<String> {
    if !super::hub_startable(&settings.terminal) {
        return Some(
            "resuming a session from the board needs terminal.preset \"tmux\" and no terminal.spawn"
                .to_string(),
        );
    }
    // Only the built-in resume line knows how to reopen a Claude conversation. Another agent
    // given it would start something unrelated in the worktree and look like a resumed worker.
    let agent = runner::agent_from_runner(
        settings
            .agent_runner
            .as_deref()
            .unwrap_or(runner::DEFAULT_AGENT_RUNNER),
    );
    if settings.agent_resume_runner.is_none() && agent != "claude" {
        return Some(format!(
            "{agent} has no agentResumeRunner, so it cannot be resumed"
        ));
    }
    // The same refusal the resume itself would end in, so the page never offers a button that
    // can only fail.
    if let Err(refusal) =
        super::resume_template(settings.agent_resume_runner.as_deref(), "agentResumeRunner")
    {
        return Some(refusal);
    }
    None
}

/// What `state` says about resuming: `available`, and the reason when it is not.
pub(super) fn resume_state(settings: &crate::kernel::config::Settings) -> Value {
    let refusal = resume_refusal(settings);
    json!({ "available": refusal.is_none(), "reason": refusal })
}

/// Reopen the worker session `id` in a new tab, as `adj work --resume` does.
///
/// The worker goes back under the hub that dispatched it without being told which: the saved
/// session remembers it (rewritten on every link), and `adj worker --resume` reads it there.
/// Forwarding this server's own hub would re-file the worker under whichever hub the board
/// happens to be for.
pub(super) fn resume(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    input_of(body)?;
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    if session.kind != "worker" {
        return Err("only a worker session can be resumed".to_string());
    }
    if session.present {
        return Err("the session is running".to_string());
    }
    let worktree = Path::new(&session.worktree);
    if messaging::is_starting(worktree, crate::infra::clock::now_secs()) {
        return Err("the session is starting".to_string());
    }
    if let Some(refusal) = resume_refusal(&settings) {
        return Err(refusal);
    }
    let ctx = own_hub_context(server, settings)?;
    let repo = ctx.repo.nwo.clone();
    let done =
        match super::resume_worker(&ctx, Some(&repo), None, &session.worktree, "", None, false)? {
            Resumed::Opened(done) => done,
            Resumed::Full(refusal) => return Err(refusal),
        };
    let hub_running = messaging::all_repo_hubs(&server.ctx.state, &server.ctx.repo)
        .iter()
        .any(|h| Some(&h.id) == session.hub.as_ref() && h.state.present);
    Ok(json!({
        "resumed": true,
        "description": done.description,
        "hub": session.hub,
        "hubRunning": hub_running,
    }))
}

/// Why no hub can be resumed from the board with these settings, or `None` when one can: what
/// `hub_startable` and `hubResumeRunner` say, known without looking at a hub. The per-hub half
/// (a saved conversation, a parent key that is known) is checked by the restart itself.
pub(super) fn hub_resume_refusal(settings: &crate::kernel::config::Settings) -> Option<String> {
    if !super::hub_startable(&settings.terminal) {
        return Some(
            "restarting a hub from the board needs terminal.preset \"tmux\" and no terminal.spawn"
                .to_string(),
        );
    }
    if let Err(refusal) =
        super::resume_template(settings.hub_resume_runner.as_deref(), "hubResumeRunner")
    {
        return Some(refusal);
    }
    super::own_hub_runner_refusal(settings)
}

/// What `state` says about resuming a hub: `available`, and the reason when it is not.
pub(super) fn hub_resume_state(settings: &crate::kernel::config::Settings) -> Value {
    let refusal = hub_resume_refusal(settings);
    json!({ "available": refusal.is_none(), "reason": refusal })
}

/// How long a hub's restart keeps refusing another one after it has answered. The answer comes
/// when the tmux window is open, a moment before the new `adj hub` has registered, and a second
/// restart in between would stop the new hub or open a stray window beside it. The page's
/// `RESTART_MS` is the same length: it shows 「再起動しています…」 for as long.
const RESTART_HOLD: Duration = Duration::from_secs(45);

enum Slot {
    InFlight,
    /// Answered at this moment, with the new process possibly not up yet.
    Held(Instant),
}

/// The hubs and worktrees being restarted, by key. What it guarantees: while a restart of a key
/// runs, or for `RESTART_HOLD` after a hub's restart answered with a window opened, another
/// restart of that key is refused. It does not look at the process table, so a hub that is up
/// sooner is still held until the time is over; a restart that failed releases at once.
static RESTARTING: Mutex<Option<HashMap<String, Slot>>> = Mutex::new(None);

/// Holds `key` in `RESTARTING` while it lives, and on every way out of the restart lets go,
/// unless `hold` turned the claim into the timed hold.
pub(super) struct Restarting {
    key: String,
    held: bool,
}

impl Restarting {
    pub(super) fn claim(key: &str, what: &str) -> Result<Restarting, String> {
        Self::claim_at(key, what, Instant::now())
    }

    /// `claim`, with the time it is made at handed in so that the hold can be tested.
    fn claim_at(key: &str, what: &str, now: Instant) -> Result<Restarting, String> {
        let mut held = RESTARTING.lock().unwrap_or_else(|e| e.into_inner());
        let map = held.get_or_insert_with(HashMap::new);
        match map.get(key) {
            Some(Slot::InFlight) => return Err(format!("{what} is already restarting")),
            Some(Slot::Held(at)) if now.saturating_duration_since(*at) < RESTART_HOLD => {
                return Err(format!(
                    "{what} was restarted a moment ago and is coming up"
                ));
            }
            _ => {}
        }
        map.insert(key.to_string(), Slot::InFlight);
        Ok(Restarting {
            key: key.to_string(),
            held: false,
        })
    }

    /// The restart answered with a new process on its way: keep refusing for `RESTART_HOLD`
    /// instead of letting go.
    pub(super) fn hold(self) {
        self.hold_at(Instant::now());
    }

    fn hold_at(mut self, now: Instant) {
        let mut held = RESTARTING.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = held.as_mut() {
            map.insert(self.key.clone(), Slot::Held(now));
        }
        self.held = true;
    }
}

impl Drop for Restarting {
    fn drop(&mut self) {
        if self.held {
            return;
        }
        let mut held = RESTARTING.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = held.as_mut() {
            map.remove(&self.key);
        }
    }
}

/// Stop the worker session `id` and start it again on the same conversation, in one step:
/// what closing it and then resuming it would do, with every refusal made before anything is
/// closed. A restart that finds out afterwards that the conversation cannot be reopened has
/// stopped a worker for nothing.
///
/// A worker that does not go is not forced: nothing is started, and its record stays. Starting
/// beside it would be two workers in one worktree.
pub(super) fn restart(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    input_of(body)?;
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    if session.kind != "worker" {
        return Err("only a worker session can be restarted".to_string());
    }
    let worktree = Path::new(&session.worktree);
    if messaging::is_starting(worktree, crate::infra::clock::now_secs()) {
        return Err("the session is starting".to_string());
    }
    if let Some(refusal) = resume_refusal(&settings) {
        return Err(refusal);
    }
    super::saved_worker_session(worktree)?;
    // Without a way to close the window the restart would end up starting a second worker
    // beside the one it could not stop.
    if settings.terminal.close.is_off() {
        return Err(
            "terminal.close is off, so the session cannot be closed to restart it".to_string(),
        );
    }
    // Built before anything is closed: it does not depend on the close, and a failure here
    // after it would have closed the session for nothing.
    let ctx = own_hub_context(server, settings)?;
    let _restarting = Restarting::claim(&session.worktree, "the session")?;
    let was_running = messaging::worker_status(worktree).present;
    let repo = server.ctx.repo.nwo.clone();
    if !super::close(Some(&repo), &session.worktree, true, false)? {
        return Err(
            "the session could not be closed (pid still running or its record unreadable); \
             nothing was restarted"
                .to_string(),
        );
    }
    let done =
        match super::resume_worker(&ctx, Some(&repo), None, &session.worktree, "", None, false) {
            Ok(Resumed::Opened(done)) => done,
            Ok(Resumed::Full(refusal)) | Err(refusal) => {
                // Nothing was closed when nothing was running, and saying so would be false.
                return Err(match was_running {
                    true => format!("closed the session, but could not start it again: {refusal}"),
                    false => format!("could not start the session again: {refusal}"),
                });
            }
        };
    let hub_running = messaging::all_repo_hubs(&server.ctx.state, &server.ctx.repo)
        .iter()
        .any(|h| Some(&h.id) == session.hub.as_ref() && h.state.present);
    Ok(json!({
        "restarted": true,
        "wasRunning": was_running,
        "description": done.description,
        "hub": session.hub,
        "hubRunning": hub_running,
    }))
}

// ── open ─────────────────────────────────────────────────────────────

const NOT_SET: &str = "terminal.attach is not set: put the command that opens your terminal in the config's terminal.attach";

/// Numbers the sessions this process makes to open a terminal on, so that two requests never
/// share a name.
static NEXT: AtomicUsize = AtomicUsize::new(1);

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

/// Open the session `id` in the person's terminal.
///
/// Through a session of its own in the group of the original, as the board terminal does
/// (`terminal::board_attach_prepare_script`): a plain attach to the shared session would move
/// every other client's current window to this one.
pub(super) fn open(server: &Server, id: &str) -> Result<Value, String> {
    let version = server.tmux.ok_or("tmux 3.1 or later is not available")?;
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    let (socket, window) =
        target_of(&session).ok_or("the session is not running in a tmux window")?;
    let socket = socket.as_deref();
    let attach = settings.terminal.attach.as_deref();
    if attach.is_none() && !terminal::iterm_available() {
        return Err(NOT_SET.to_string());
    }

    // Before anything is made: a window that is gone must not start a server or leave a
    // session behind.
    let home =
        terminal::run_shell(&terminal::tmux_window_home_script(socket, &window)).map_err(|e| {
            let lower = e.to_ascii_lowercase();
            match lower.contains("can't find")
                || lower.contains("no server running")
                || lower.contains("error connecting")
            {
                true => "the tmux window is gone".to_string(),
                false => e,
            }
        })?;
    let group = terminal::parse_window_home(&home).ok_or("the tmux window is gone")?;

    let _ = terminal::run_shell(&terminal::board_sweep_script(socket));
    let name = format!(
        "{}{}-{}",
        terminal::OPEN_SESSION_PREFIX,
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    );
    terminal::run_shell(&terminal::board_attach_prepare_script(
        socket, &group, &name, &window,
    ))?;

    let opened = match attach {
        Some(template) => {
            let command = render(
                template,
                &[
                    (
                        "socket",
                        Sub::Raw(&sh_join(&terminal::tmux_socket_args(socket))),
                    ),
                    ("session", Sub::Quoted(&name)),
                    ("window", Sub::Quoted(&window)),
                ],
            );
            terminal::run_shell(&command)
                .map(|_| "terminal.attach".to_string())
                .map_err(|e| format!("terminal.attach failed: {e}"))
        }
        None => {
            // `-CC` is iTerm2's own way of drawing tmux's windows; any other terminal gets a
            // plain attach through its template.
            let line = terminal::native_attach_line(socket, &name, true, version >= (3, 4));
            terminal::iterm_attach(&line)
                .map(|_| "iTerm2".to_string())
                .map_err(|e| format!("iTerm2 could not open the session: {e}"))
        }
    };
    match opened {
        Ok(terminal) => Ok(json!({
            "opened": true,
            "description": format!("opened {id} in {terminal}"),
            "session": name,
            "window": window,
        })),
        Err(e) => {
            // Nothing attached to it, so nothing else is holding the group's windows.
            let _ = terminal::run_shell(&format!(
                "{} kill-session -t {}",
                terminal::tmux_cmd_prefix(socket),
                sh_quote(&format!("={name}"))
            ));
            Err(e)
        }
    }
}

// ── cleanup ──────────────────────────────────────────────────────────

/// Remove the worktree of the worker session `id` and its local branch, closing the session
/// first if it runs.
///
/// The hub's procedure says it is the only route by which anything is removed; from here the
/// board is a second one, and makes the same checks itself rather than asking the hub, which
/// may not be running. Refusals that cannot be forced (the ground under a worker still to
/// come) are errors. What would be lost is a `removed: false` with the reasons, answered 200
/// like `closed: false` is: the page keeps only `error` from a non-2xx answer, and the reasons
/// are what the person needs to decide whether to force.
pub(super) fn cleanup(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input = input_of(body)?;
    let force = match input.get("force") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(force)) => *force,
        Some(other) => return Err(format!("force has to be true or false, not {other}")),
    };
    let confirm = text(&input, "confirm")?;
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    let repo = &server.ctx.repo;
    if session.kind != "worker" {
        return Err("only a worker session has a worktree to remove".to_string());
    }
    let worktree = session.worktree.as_str();
    if same_path(worktree, &repo.main) {
        return Err("the main checkout is not a worktree to remove".to_string());
    }
    let listed = crate::kernel::identity::linked_worktrees(&repo.main)?;
    if !listed.iter().any(|w| same_path(w, worktree)) {
        return Err(format!("not a worktree of this repository: {worktree}"));
    }
    let name = Path::new(worktree)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if force && confirm != Some(name.as_str()) {
        return Err(format!("confirm has to be the worktree's name: {name}"));
    }

    // The tasks that name this worktree, in whichever hub they were made.
    let hubs = messaging::all_repo_hubs(&server.ctx.state, repo);
    let mut tasks: Vec<(String, task::Task)> = Vec::new();
    for hub in &hubs {
        for t in task::list(&task::dir(&server.ctx.state, &hub.slug)) {
            if t.worktree
                .as_deref()
                .is_some_and(|w| same_path(w, worktree))
            {
                tasks.push((hub.slug.clone(), t));
            }
        }
    }
    if tasks.iter().any(|(_, t)| t.status == Status::Queued) {
        return Err("a queued task is waiting in it".to_string());
    }
    // A Jules plan is written in a detached worktree with no commits, which looks finished.
    if tasks.iter().any(|(_, t)| {
        t.executor == Executor::Jules && t.status == Status::Dispatched && t.jules_session.is_none()
    }) {
        return Err("a Jules plan is being written in it".to_string());
    }
    if messaging::is_starting(Path::new(worktree), crate::infra::clock::now_secs()) {
        return Err("the session is starting".to_string());
    }

    let state = git_state_of(server, &session);
    let reasons = loss_reasons(&state);
    if !reasons.is_empty() && !force {
        return Ok(json!({
            "removed": false,
            "reasons": reasons,
            "git": state.as_ref().ok().and_then(|s| s.as_ref()),
        }));
    }
    let state = state.ok().flatten();

    let main = Path::new(&repo.main);
    let was_running = messaging::worker_status(Path::new(worktree)).present;
    // `close` answers `true` for a worker that is not there, so a stopped session is not
    // refused; `false` is a worker that may still be running, or a record that cannot be
    // read, and removing under either is what this must not do. Outside the dispatch lock,
    // which others give up waiting for after ten seconds.
    //
    // Only when something may be running: closing a worker that is gone clears its record, and
    // a removal that then fails would have taken the session's phase history for nothing.
    let may_run = match messaging::read_worker(Path::new(worktree)) {
        messaging::Recorded::Absent => false,
        messaging::Recorded::Unreadable => true,
        messaging::Recorded::Found(worker) => {
            messaging::worker_liveness(&worker) != messaging::Liveness::Gone
        }
    };
    if may_run && !super::close(Some(&repo.nwo), worktree, true, false)? {
        return Err("the session could not be closed; nothing was removed".to_string());
    }
    // Looked at again now that nothing is running: a worker that committed between the first
    // look and its tab closing has made work the first look did not see, and `branch -D`
    // below would drop it.
    if !force {
        let again = git_state_of(server, &session);
        let reasons = loss_reasons(&again);
        if !reasons.is_empty() {
            return Ok(json!({
                "removed": false,
                "closed": was_running,
                "reasons": reasons,
                "git": again.ok().flatten(),
            }));
        }
    }
    // The lock covers only the last look at the worker slots and the marker that says the
    // worktree is going, which `claim_worker_slot` refuses on under the same lock: a worker
    // that starts here from now on is turned away. The removal itself, which can take long on
    // a big worktree, runs after the lock is released, so that a dispatch elsewhere is not
    // made to wait for it.
    let removing = messaging::with_dispatch_lock(main, || -> Result<_, String> {
        if messaging::holds_worker_slot(Path::new(worktree), crate::infra::clock::now_secs()) {
            return Err("a session started in it meanwhile; nothing was removed".to_string());
        }
        messaging::mark_worktree_removing(main, Path::new(worktree))
    })??;
    let removed = remove_worktree(&server.ctx.state, &repo.main, worktree, force);
    // Released only now, whatever the removal came to.
    drop(removing);
    removed?;

    // The worker's record, its saved session and its outbox live inside the worktree, so the
    // session is off the board with it; nothing of it is kept in the state directory.
    let branch_name = state
        .as_ref()
        .and_then(|s| s.branch.clone())
        .or(session.branch.clone());
    let branch = match &branch_name {
        Some(branch) => match delete_branch(&repo.main, branch) {
            Ok(()) => json!({ "name": branch, "deleted": true }),
            Err(e) => json!({ "name": branch, "deleted": false, "error": e }),
        },
        None => json!({ "name": Value::Null, "deleted": false }),
    };

    let hooks = run_remove_hooks(server, worktree, &name);

    // After the removal, as the hub does: a card left in progress for a worktree that is gone
    // would stay on the board for good.
    let mut done = Vec::new();
    let mut task_errors = Vec::new();
    for (slug, t) in tasks
        .iter()
        .filter(|(_, t)| matches!(t.status, Status::Dispatched | Status::Pr))
    {
        let Some(hub) = hubs.iter().find(|h| &h.slug == slug) else {
            continue;
        };
        let mut addressed = repo.clone();
        addressed.slug = hub.slug.clone();
        addressed.hub_name = hub.name.clone();
        let ctx = Context {
            repo: addressed,
            resolved: server.ctx.resolved.clone(),
            state: server.ctx.state.clone(),
            settings: settings.clone(),
        };
        match super::task::update(&ctx, &t.id, &json!({ "status": "done", "handOver": false })) {
            Ok(_) => done.push(t.id.clone()),
            Err(e) => task_errors.push(json!({ "id": t.id, "error": e })),
        }
    }
    let mut reply = json!({
        "removed": true,
        "forced": force,
        "closed": was_running,
        "branch": branch,
        "tasks": done,
        "hooks": hooks,
    });
    if !task_errors.is_empty() {
        reply["taskErrors"] = json!(task_errors);
    }
    Ok(reply)
}

/// What removing the worktree would lose, as `{kind, detail}` for the page to list. A git that
/// could not answer is a reason of its own: not knowing is not the same as nothing to lose.
fn loss_reasons(state: &Result<Option<GitState>, String>) -> Vec<Value> {
    let state = match state {
        Err(e) => return vec![json!({ "kind": "git", "detail": e })],
        // The directory is gone: there is nothing left to lose.
        Ok(None) => return Vec::new(),
        Ok(Some(state)) => state,
    };
    let mut reasons = Vec::new();
    let uncommitted = &state.uncommitted;
    if uncommitted.files > 0 {
        reasons.push(json!({
            "kind": "uncommitted",
            "detail": format!(
                "{} changed file(s), +{} -{} lines",
                uncommitted.files, uncommitted.insertions, uncommitted.deletions
            ),
        }));
    }
    if uncommitted.untracked > 0 {
        reasons.push(json!({
            "kind": "untracked",
            "detail": format!("{} untracked path(s)", uncommitted.untracked),
        }));
    }
    if state.unpushed.count > 0 {
        reasons.push(json!({
            "kind": "unpushed",
            "detail": format!("{} commit(s) no remote has", state.unpushed.count),
        }));
    }
    reasons
}

/// `git worktree remove`, forced only when the person forced it.
///
/// Adjutant's own files in the worktree (the worker's record, its saved session, the brief) are
/// untracked in a repository that does not ignore `.claude`, and git refuses to remove a
/// worktree with untracked files. Only when git says no are the ones no commit tracks moved out
/// of the way for a second try, and they are put back if that fails too: until the worktree is
/// gone they are what lets the session be seen and resumed. A directory that is already gone
/// is removed by name, which drops git's record of that one worktree and no other.
fn remove_worktree(root: &Path, main: &str, worktree: &str, force: bool) -> Result<(), String> {
    let mut args = vec!["-C", main, "worktree", "remove"];
    if force || !Path::new(worktree).is_dir() {
        args.push("--force");
    }
    args.push(worktree);
    let first = match git_ok(&args) {
        Ok(()) => return Ok(()),
        Err(e) if force => return Err(e),
        Err(e) => e,
    };
    // git's own message is the one that says why, so it is what is returned.
    let aside = set_aside_own_files(root, worktree)
        .map_err(|e| format!("{first} (and adjutant's files could not be moved aside: {e})"))?;
    match git_ok(&args) {
        Ok(()) => {
            aside.discard();
            Ok(())
        }
        Err(e) => match aside.restore() {
            Ok(()) => Err(e),
            Err(kept) => Err(format!("{e} ({kept})")),
        },
    }
}

/// Files moved out of a worktree: where each was, and the copy kept outside it.
struct Aside {
    files: Vec<(std::path::PathBuf, std::path::PathBuf)>,
}

impl Aside {
    /// Put back every file whose place is empty. The directory holding the copies goes only
    /// when all of them are back; otherwise it stays and the error says where.
    fn restore(self) -> Result<(), String> {
        let mut failed = false;
        for (original, kept) in &self.files {
            // A file that is there is the worker's own, possibly newer: never overwritten.
            if !original.exists() && move_file(kept, original).is_err() {
                failed = true;
            }
        }
        match (failed, self.dir()) {
            (false, dir) => {
                if let Some(dir) = dir {
                    let _ = std::fs::remove_dir_all(dir);
                }
                Ok(())
            }
            (true, dir) => Err(format!(
                "adjutant's files could not all be put back; they are kept in {}",
                dir.map(|d| d.display().to_string()).unwrap_or_default()
            )),
        }
    }

    fn discard(self) {
        if let Some(dir) = self.dir() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn dir(&self) -> Option<&Path> {
        self.files.first().and_then(|(_, kept)| kept.parent())
    }
}

/// Rename, or copy and remove where the two places are on different file systems. A copy that
/// fails part way is removed, so that it cannot later stand in for the original.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if let Err(e) = std::fs::copy(from, to) {
        let _ = std::fs::remove_file(to);
        return Err(e);
    }
    // A copy that stays beside a source that could not be removed would be a file the list of
    // what was moved does not know.
    std::fs::remove_file(from).inspect_err(|_| {
        let _ = std::fs::remove_file(to);
    })
}

/// Move `.claude/adjutant-*` and `.claude/task-brief.md` out of `worktree` unless git tracks
/// them, into a directory of the state directory made when the first one is moved.
fn set_aside_own_files(root: &Path, worktree: &str) -> Result<Aside, String> {
    let listed =
        crate::infra::git::git(&["-C", worktree, "ls-files", "-z", "--", ".claude"], None)?;
    if !listed.status.success() {
        return Err(format!(
            "cannot list the tracked files: {}",
            String::from_utf8_lossy(&listed.stderr).trim()
        ));
    }
    let tracked = String::from_utf8_lossy(&listed.stdout).to_string();
    let tracked: Vec<&str> = tracked.split('\0').collect();
    let mut aside = Aside { files: Vec::new() };
    let Ok(entries) = std::fs::read_dir(Path::new(worktree).join(".claude")) else {
        return Ok(aside);
    };
    // Read to the end before anything is moved, so that the listing is not of a directory
    // that is changing under it.
    let entries: Vec<_> = entries.flatten().collect();
    let dir = root.join(format!(
        "cleanup-{}-{}-{}",
        std::process::id(),
        crate::infra::clock::now_secs(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let mut made = false;
    for entry in entries {
        let file = entry.file_name().to_string_lossy().to_string();
        let ours = file.starts_with("adjutant-") || file == "task-brief.md";
        if !ours || tracked.contains(&format!(".claude/{file}").as_str()) {
            continue;
        }
        // `create_dir` for the first: a directory already there is not ours and is not written
        // into.
        let step = (if made {
            Ok(())
        } else {
            std::fs::create_dir_all(dir.parent().unwrap_or(&dir))
                .and_then(|_| std::fs::create_dir(&dir))
                .inspect(|_| made = true)
        })
        .and_then(|_| move_file(&entry.path(), &dir.join(&file)));
        match step {
            Ok(()) => aside.files.push((entry.path(), dir.join(&file))),
            Err(e) => {
                let why = format!("{}: {e}", entry.path().display());
                // The directory may have been made for a file that never got into it.
                if made && aside.files.is_empty() {
                    let _ = std::fs::remove_dir(&dir);
                }
                return match aside.restore() {
                    Ok(()) => Err(why),
                    Err(kept) => Err(format!("{why}; {kept}")),
                };
            }
        }
    }
    Ok(aside)
}

fn git_ok(args: &[&str]) -> Result<(), String> {
    let out = crate::infra::git::git(args, None)?;
    match out.status.success() {
        true => Ok(()),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// The local branch only. A remote ref is never touched from here: the branch may be the
/// head of a PR that is still open.
fn delete_branch(main: &str, branch: &str) -> Result<(), String> {
    git_ok(&["-C", main, "branch", "-D", "--", branch])
}

/// The repository's `onWorktreeRemove` commands, run in the main checkout after the removal
/// with `{worktree}` and `{name}` filled in, as the hub does. Read from the config now, not
/// from the settings the server started with. A hook that fails is reported and does not
/// undo anything.
fn run_remove_hooks(server: &Server, worktree: &str, name: &str) -> Vec<Value> {
    let hooks = crate::kernel::config::resolve_config(&server.ctx.repo.nwo)
        .ok()
        .and_then(|resolved| resolved.config)
        .and_then(|config| config.get("onWorktreeRemove").cloned())
        .and_then(|hooks| hooks.as_array().cloned())
        .unwrap_or_default();
    hooks
        .iter()
        .filter_map(Value::as_str)
        .map(|template| {
            let command = render(
                template,
                &[
                    ("worktree", Sub::Quoted(worktree)),
                    ("name", Sub::Quoted(name)),
                ],
            );
            let ran = std::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .current_dir(&server.ctx.repo.main)
                .output();
            match ran {
                Ok(out) if out.status.success() => json!({ "command": command, "ok": true }),
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    let error = match stderr.is_empty() {
                        true => format!("exited with {}", out.status),
                        false => stderr,
                    };
                    json!({ "command": command, "ok": false, "error": error })
                }
                Err(e) => json!({ "command": command, "ok": false, "error": e.to_string() }),
            }
        })
        .collect()
}

// ── start a parent-task hub ──────────────────────────────────────────

/// Start the hub for the parent-task `key`, as `adj hub --hub KEY` does. The other way to
/// start a parent-task hub is by the id in `hubs[]`, which only exists once something points
/// at it.
pub(super) fn start_parent_hub(server: &Server, body: &[u8]) -> Result<Value, String> {
    let input = input_of(body)?;
    let key = text(&input, "key")?.ok_or("a key is required")?;
    let start = hub_start_of(&input)?;
    let settings = settings_now(server);
    let ctx = Context {
        repo: server.ctx.repo.clone().addressed(Some(key))?,
        resolved: server.ctx.resolved.clone(),
        state: server.ctx.state.clone(),
        settings,
    };
    let hub = json!({ "id": format!("hub-{key}"), "slug": ctx.repo.slug });
    match super::start_hub(&ctx, start)? {
        TabOutcome::Opened(done) => Ok(json!({
            "started": true,
            "description": done.description,
            "hub": hub,
        })),
        TabOutcome::AlreadyRunning(status) => Ok(json!({
            "alreadyRunning": true,
            "pid": status.pid,
            "hub": hub,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::{RESTART_HOLD, Restarting};
    use std::time::Instant;

    #[test]
    fn a_second_restart_of_the_same_target_is_refused_until_the_first_is_over() {
        let first = Restarting::claim("guard-test/a", "the session").unwrap();
        let again = Restarting::claim("guard-test/a", "the session");
        assert_eq!(again.err().unwrap(), "the session is already restarting");
        // Another target is not held up by it.
        let other = Restarting::claim("guard-test/b", "the session");
        assert!(other.is_ok());
        drop(first);
        assert!(Restarting::claim("guard-test/a", "the session").is_ok());
    }

    #[test]
    fn a_held_restart_refuses_another_until_the_hold_is_over() {
        let at = Instant::now();
        Restarting::claim_at("guard-test/held", "the hub", at)
            .unwrap()
            .hold_at(at);
        let soon = Restarting::claim_at("guard-test/held", "the hub", at + RESTART_HOLD / 2);
        assert!(soon.err().unwrap().contains("coming up"));
        let later = Restarting::claim_at("guard-test/held", "the hub", at + RESTART_HOLD);
        assert!(later.is_ok());
    }
}
