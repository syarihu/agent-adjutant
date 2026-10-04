//! The two things the board can change, and the requests that change them.

use std::path::Path;

use serde_json::{Value, json};

use crate::messaging;

use super::Server;
use super::registry::forget_board;
use super::routes::{hub_route, task_id_in};
use super::state::settings_now;

// ── the two things the board can change ──────────────────────────────

pub(super) fn create_task(server: &Server, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let (task, handed) = crate::cmd::task::create(&server.ctx, &input)?;
    Ok(json!({ "task": task, "handed": handed_json(handed) }))
}

pub(super) fn update_task(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let (task, handed) = crate::cmd::task::update(&server.ctx, id, &input)?;
    Ok(json!({ "task": task, "handed": handed_json(handed) }))
}

/// The three buttons a card has for the worker behind it: raise its tab, open its worktree in
/// the editor, close its tab. All through the same templates the commands use.
///
/// Only a worktree of this checkout is acted on. The path comes from the page, and these run
/// commands — `ide` a template of the person's own choosing — so a path the board did not
/// list is refused rather than handed on.
pub(super) fn act_on_worktree(server: &Server, action: &str, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let worktree = input
        .get("worktree")
        .and_then(Value::as_str)
        .ok_or("a worktree is required")?;
    let known = crate::kernel::identity::linked_worktrees(&server.ctx.repo.main)?;
    if !known.iter().any(|w| w == worktree) {
        return Err(format!("not a worktree of this repository: {worktree}"));
    }
    let path = Path::new(worktree);
    let settings = settings_now(server);
    match action {
        "focus" => {
            let done = crate::cmd::focus_worker(&settings, path, false)?;
            Ok(json!({
                "present": done.is_some(),
                "ran": done.as_ref().is_some_and(|d| d.ran),
            }))
        }
        "ide" => {
            let command = crate::infra::ide::open_command(settings.ide.as_deref(), worktree)
                .ok_or("ide is not set: put your editor command in the config's ide key")?;
            crate::infra::terminal::run_shell(&command)?;
            Ok(json!({ "ran": true }))
        }
        "close" => {
            let closed = crate::cmd::close(Some(&server.ctx.repo.nwo), worktree, true, false)?;
            Ok(json!({ "closed": closed }))
        }
        other => Err(format!("no such action: {other}")),
    }
}

/// Raise the hub's tab: 「タブで話す」 on a gate the hub opened, which sits in the main
/// checkout where there is no worker to raise.
pub(super) fn focus_hub(server: &Server) -> Result<Value, String> {
    let repo = &server.ctx.repo;
    let status = messaging::hub_status(&server.ctx.state, &repo.slug, &repo.hub_name);
    let Some(pid) = status.pid.filter(|_| status.present) else {
        return Ok(json!({ "present": false, "ran": false }));
    };
    let settings = settings_now(server);
    let done = crate::infra::terminal::focus(&settings.terminal, pid, &repo.hub_name, false)?;
    Ok(json!({ "present": true, "ran": done.ran }))
}

/// How a hub is to be started, from the `start` a request names: `auto` when it names none.
pub(in crate::cmd) fn hub_start_of(input: &Value) -> Result<crate::cmd::HubStart, String> {
    match input.get("start").and_then(Value::as_str).unwrap_or("auto") {
        "auto" => Ok(crate::cmd::HubStart::Auto),
        "resume" => Ok(crate::cmd::HubStart::Resume),
        "new" => Ok(crate::cmd::HubStart::New),
        other => Err(format!("no such start: {other}")),
    }
}

/// The context that starts `hub`: addressed by its key, refused when a parent hub's key is not
/// known (it could only be started as some other hub).
fn hub_start_context(
    server: &Server,
    hub: &crate::session::RepoHub,
    settings: crate::kernel::config::Settings,
) -> Result<crate::cmd::Context, String> {
    if hub.parent && hub.key.is_none() {
        return Err(
            "the key of this hub is not known; start it with adj hub --hub <key>".to_string(),
        );
    }
    Ok(crate::cmd::Context {
        repo: server.ctx.repo.clone().addressed(hub.key.as_deref())?,
        resolved: server.ctx.resolved.clone(),
        state: server.ctx.state.clone(),
        settings,
    })
}

/// The context that stops `hub`. Addressed by the slug the hub was listed under: a hub whose
/// key cannot be told can still be stopped, and nothing here needs the key for it.
fn hub_stop_context(
    server: &Server,
    hub: &crate::session::RepoHub,
    settings: crate::kernel::config::Settings,
) -> crate::cmd::Context {
    let mut stopping = server.ctx.repo.clone();
    stopping.slug = hub.slug.clone();
    stopping.hub_name = hub.name.clone();
    crate::cmd::Context {
        repo: stopping,
        resolved: server.ctx.resolved.clone(),
        state: server.ctx.state.clone(),
        settings,
    }
}

/// Start, stop, close, reset or restart one of the repository's hubs from the board. `id` is the
/// `hubs[].id` the page was given, so the page can only name a hub this repository was found to
/// have.
pub(super) fn act_on_hub(server: &Server, path: &str, body: &[u8]) -> Result<Value, String> {
    let (id, action) = hub_route(path).ok_or("no such route")?;
    let id = id?;
    let id = id.as_str();
    let input: Value = match body.is_empty() {
        true => json!({}),
        false => serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?,
    };
    let repo = &server.ctx.repo;
    let hub = messaging::all_repo_hubs(&server.ctx.state, repo)
        .into_iter()
        .find(|h| h.id == id)
        .ok_or_else(|| format!("no such hub: {id}"))?;
    let settings = settings_now(server);
    match action {
        "start" => {
            let start = hub_start_of(&input)?;
            let ctx = hub_start_context(server, &hub, settings)?;
            match crate::cmd::start_hub(&ctx, start)? {
                crate::cmd::TabOutcome::Opened(done) => {
                    Ok(json!({ "started": true, "description": done.description }))
                }
                crate::cmd::TabOutcome::AlreadyRunning(status) => {
                    Ok(json!({ "alreadyRunning": true, "pid": status.pid }))
                }
            }
        }
        "reset" => {
            // Everything start would refuse is refused before the hub is stopped: a reset that
            // could not start again would only have taken the hub down.
            if !crate::cmd::hub_startable(&settings.terminal) {
                return Err(
                    "starting a hub from the board needs terminal.preset \"tmux\"".to_string(),
                );
            }
            let start_ctx = hub_start_context(server, &hub, settings.clone())?;
            let was_running = crate::cmd::stop_hub(&hub_stop_context(server, &hub, settings))?;
            match crate::cmd::start_hub(&start_ctx, crate::cmd::HubStart::New) {
                Ok(crate::cmd::TabOutcome::Opened(done)) => Ok(json!({
                    "reset": true,
                    "wasRunning": was_running,
                    "started": true,
                    "description": done.description,
                })),
                // A hub is up that this request did not start (nothing was running, or another
                // start won the race after the stop), so it is not a new conversation, and
                // the answer must not say it is.
                Ok(crate::cmd::TabOutcome::AlreadyRunning(status)) => Ok(json!({
                    "reset": false,
                    "wasRunning": was_running,
                    "alreadyRunning": true,
                    "pid": status.pid,
                })),
                Err(e) if was_running => Err(format!(
                    "stopped {}, but could not start it again: {e}",
                    hub.name
                )),
                Err(e) => Err(e),
            }
        }
        "restart" => {
            // Everything the start would refuse is refused before the hub is stopped, the
            // saved conversation included: a restart that cannot reopen it has only taken the
            // hub down. The saved session file is not touched; `--resume` reads it as it is.
            if let Some(refusal) = crate::cmd::board_actions::hub_resume_refusal(&settings) {
                return Err(refusal);
            }
            let start_ctx = hub_start_context(server, &hub, settings.clone())?;
            crate::cmd::hub_resume_check(&start_ctx)?;
            let restarting = crate::cmd::board_actions::Restarting::claim(&hub.slug, &hub.name)?;
            let was_running = crate::cmd::stop_hub(&hub_stop_context(server, &hub, settings))?;
            match crate::cmd::start_hub(&start_ctx, crate::cmd::HubStart::Resume) {
                Ok(crate::cmd::TabOutcome::Opened(done)) => {
                    // The window is open but the new hub has not registered yet: another
                    // restart now would stop it or open a second window beside it.
                    restarting.hold();
                    Ok(json!({
                        "restarted": true,
                        "wasRunning": was_running,
                        "started": true,
                        "description": done.description,
                    }))
                }
                // As for a reset: a hub is up that this request did not start, so the
                // answer must not say it was restarted.
                Ok(crate::cmd::TabOutcome::AlreadyRunning(status)) => Ok(json!({
                    "restarted": false,
                    "wasRunning": was_running,
                    "alreadyRunning": true,
                    "pid": status.pid,
                })),
                Err(e) if was_running => Err(format!(
                    "stopped {}, but could not start it again: {e}",
                    hub.name
                )),
                Err(e) => Err(e),
            }
        }
        "stop" | "close" => {
            let closing = action == "close";
            if closing {
                crate::cmd::closable_check(repo, &hub)?;
            }
            // A hub that will not stop is not closed: nothing is forgotten until it is gone.
            let was_running = crate::cmd::stop_hub(&hub_stop_context(server, &hub, settings))?;
            if closing {
                // `stop_hub` cleared a record naming the process it stopped; what is left
                // names none, unless a hub registered in the meantime, which stays.
                if !messaging::unregister_hub_if_unnamed(&server.ctx.state, &hub.slug)? {
                    return Err(format!("{} changed while it was being closed", hub.name));
                }
                forget_board(&server.ctx.state, &hub.slug)?;
                Ok(json!({ "closed": true, "wasRunning": was_running, "unread": hub.inbox_count }))
            } else {
                Ok(json!({ "stopped": true, "wasRunning": was_running }))
            }
        }
        other => Err(format!("no such action: {other}")),
    }
}

/// The review bots' comments on a Jules task's PR, for the side sheet to choose from. Asked
/// for when a person opens the list, not on every poll: it is a round trip to GitHub.
pub(super) fn review_findings(server: &Server, path: &str) -> Result<Value, String> {
    let id = task_id_in(path, "findings").ok_or("no such task")?;
    Ok(json!({ "findings": crate::cmd::jules_findings(&server.ctx, id)? }))
}

/// Post the chosen comments to the PR for Jules, in the name `gh` is signed in as.
pub(super) fn relay_findings(server: &Server, path: &str, body: &[u8]) -> Result<Value, String> {
    let id = task_id_in(path, "relay").ok_or("no such task")?;
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let chosen = input
        .get("comments")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .map(crate::cmd::JulesChosen::read)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    crate::cmd::jules_relay(
        &server.ctx,
        id,
        &chosen,
        input.get("note").and_then(Value::as_str),
    )
}

/// The board's 「再取得」: read the task's issue again, on a click and never on a poll.
pub(super) fn fetch_issue(server: &Server, path: &str) -> Result<Value, String> {
    let id = task_id_in(path, "issue").ok_or("no such task")?;
    let task = crate::cmd::task::fetch_issue(&server.ctx, id)?;
    Ok(json!({ "task": task }))
}

/// The board's 「PR を確認」: the same pass as `adj task refresh`, whose answer the page shows
/// in its log before it redraws.
pub(super) fn refresh_tasks(server: &Server) -> Result<Value, String> {
    let checked = crate::cmd::task::refresh(&server.ctx)?;
    Ok(crate::cmd::task::refresh_json(&checked))
}

pub(super) fn nudge_hub(server: &Server) -> Result<Value, String> {
    let handed = crate::cmd::task::nudge(&server.ctx)?;
    Ok(json!({ "handed": handed_json(Some(handed)) }))
}

pub(super) fn answer_gate(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let decision = input
        .get("decision")
        .and_then(Value::as_str)
        .ok_or("a decision is required")?;
    if decision == "close" || decision == "dismiss" {
        let gate = crate::cmd::gate::close(
            &server.ctx,
            id,
            input.get("comment").and_then(Value::as_str),
            false,
        )?;
        return Ok(json!({ "gate": gate, "closed": true }));
    }
    let (gate, told) = crate::cmd::gate::answer(
        &server.ctx,
        id,
        decision,
        input.get("choice").and_then(Value::as_str),
        input.get("comment").and_then(Value::as_str),
    )?;
    Ok(json!({ "gate": gate, "present": told.present, "woken": told.woken }))
}

/// What the page is told about the hand-over: whether the hub was there, and whether its
/// tab was poked. Both matter to the person — a hub that is down is not an error, it just
/// means the task waits.
fn handed_json(handed: Option<crate::cmd::Delivered>) -> Value {
    match handed {
        Some(d) => json!({
            "present": d.delivery.present,
            "woken": d.woken,
        }),
        None => Value::Null,
    }
}
