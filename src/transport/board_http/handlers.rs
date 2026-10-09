//! The two things the board can change, and the requests that change them.

use std::path::Path;

use serde_json::{Value, json};

use super::routes::HubAction;
use super::sessions::{input_of, text};
use crate::board::hub::{Reopened, close, find, reset, restart, start, start_for_key, stop};
use crate::board::{Server, settings_now};
use crate::jules::{findings as jules_findings, relay as jules_relay};
use crate::lifecycle::hub::{HubStart, TabOutcome};

// ── the two things the board can change ──────────────────────────────

pub(super) fn create_task(server: &Server, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    // An instruction to `create`, not a field of the task.
    let hand_over = input
        .get("handOver")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let new = crate::task::NewTask::from_json(&input)?;
    let (task, handed) = crate::task::create(&server.ctx, new, hand_over)?;
    Ok(json!({ "task": task, "handed": handed.as_ref().map(crate::mail::Handed::from) }))
}

pub(super) fn update_task(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    // An instruction to `update`, not a field of the task.
    let hand_over = input
        .get("handOver")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let patch = crate::task::TaskPatch::from_json(&input)?;
    let (task, handed) = crate::task::update(&server.ctx, id, &patch, hand_over)?;
    Ok(json!({ "task": task, "handed": handed.as_ref().map(crate::mail::Handed::from) }))
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
            let done = crate::lifecycle::worker::focus_worker(&settings, path, false)?;
            Ok(json!({
                "present": done.is_some(),
                "ran": done.as_ref().is_some_and(|d| d.ran),
            }))
        }
        "ide" => {
            let command = crate::infra::ide::open_command(settings.ide.as_deref(), worktree)
                .ok_or("ide is not set: put your editor command in the config's ide key")?;
            crate::infra::shell::run_shell(&command)?;
            Ok(json!({ "ran": true }))
        }
        "close" => {
            let closed = crate::lifecycle::worker::close(&settings, path, false)?.is_free();
            Ok(json!({ "closed": closed }))
        }
        other => Err(format!("no such action: {other}")),
    }
}

/// Poke this board's hub to read what is waiting for it, without leaving it a message: the
/// button on the agent board's hub entry. The wake is what `deliver_to_hub` runs, with the
/// settings as they are now; `woken` is whether it typed, and `why` is what stopped it when
/// the screen did. A refused wake is an answer, not an error.
pub(super) fn wake_hub(server: &Server) -> Result<Value, String> {
    let ctx = crate::registry::Context {
        settings: settings_now(server),
        ..server.ctx.clone()
    };
    // The oldest message still to be read names the wake, as the delivery of it would have.
    let subject = crate::mail::pending(&ctx.state, &ctx.repo.slug)
        .messages
        .into_iter()
        .find(|m| {
            !m.seen
                && crate::mail::should_wake_hub(&m.from, &ctx.repo.hub_name, &m.kind, &m.subject)
        })
        .map(|m| m.subject)
        .unwrap_or_default();
    Ok(match crate::mail::wake_hub(&ctx, &subject) {
        None => json!({ "present": false, "woken": false, "screen": false }),
        Some(Ok(done)) if done.ran => json!({ "present": true, "woken": true, "screen": false }),
        Some(Ok(done)) => json!({
            "present": true,
            "woken": false,
            "screen": done.screen,
            "why": done.description,
        }),
        Some(Err(why)) => json!({ "present": true, "woken": false, "screen": false, "why": why }),
    })
}

/// Raise the hub's tab: 「タブで話す」 on a gate the hub opened, which sits in the main
/// checkout where there is no worker to raise.
pub(super) fn focus_hub(server: &Server) -> Result<Value, String> {
    let ctx = crate::registry::Context {
        settings: settings_now(server),
        ..server.ctx.clone()
    };
    let raised = crate::lifecycle::hub::focus(&ctx, false)?;
    Ok(json!({
        "present": raised.is_some(),
        "ran": raised.as_ref().is_some_and(|r| r.done.ran),
    }))
}

/// Start, stop, close, reset or restart one of the repository's hubs from the board. `id` is the
/// `hubs[].id` the page was given, so the page can only name a hub this repository was found to
/// have.
pub(super) fn act_on_hub(
    server: &Server,
    id: &str,
    action: HubAction,
    body: &[u8],
) -> Result<Value, String> {
    let input: Value = match body.is_empty() {
        true => json!({}),
        false => serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?,
    };
    let hub = find(server, id)?;
    match action {
        HubAction::Start => {
            let how = hub_start_of(&input)?;
            Ok(match start(server, &hub, how)? {
                TabOutcome::Opened(done) => {
                    json!({ "started": true, "description": done.description })
                }
                TabOutcome::AlreadyRunning(status) => {
                    json!({ "alreadyRunning": true, "pid": status.pid })
                }
            })
        }
        HubAction::Reset => Ok(reopened("reset", reset(server, &hub)?)),
        HubAction::Restart => Ok(reopened("restarted", restart(server, &hub)?)),
        HubAction::Stop => {
            let was_running = stop(server, &hub)?;
            Ok(json!({ "stopped": true, "wasRunning": was_running }))
        }
        HubAction::Close => {
            let closed = close(server, &hub)?;
            Ok(json!({ "closed": true, "wasRunning": closed.was_running, "unread": closed.unread }))
        }
    }
}

/// A reset's or a restart's answer, `word` saying which. When a hub is up that this request
/// did not start (nothing was running, or another start won the race after the stop), it is
/// not the conversation that was asked for, and the answer must not say it is.
fn reopened(word: &str, done: Reopened) -> Value {
    match done.outcome {
        TabOutcome::Opened(opened) => json!({
            word: true,
            "wasRunning": done.was_running,
            "started": true,
            "description": opened.description,
        }),
        TabOutcome::AlreadyRunning(status) => json!({
            word: false,
            "wasRunning": done.was_running,
            "alreadyRunning": true,
            "pid": status.pid,
        }),
    }
}

/// Start the hub for the parent-task key a request names (`POST /api/hubs`), and say where it
/// is listed.
pub(super) fn start_parent_hub(server: &Server, body: &[u8]) -> Result<Value, String> {
    let input = input_of(body)?;
    let key = text(&input, "key")?.ok_or("a key is required")?;
    let how = hub_start_of(&input)?;
    let (at, outcome) = start_for_key(server, key, how)?;
    let hub = json!({ "id": at.id, "slug": at.slug });
    Ok(match outcome {
        TabOutcome::Opened(done) => json!({
            "started": true,
            "description": done.description,
            "hub": hub,
        }),
        TabOutcome::AlreadyRunning(status) => json!({
            "alreadyRunning": true,
            "pid": status.pid,
            "hub": hub,
        }),
    })
}

/// How a hub is to be started, from the `start` a request names: `auto` when it names none.
fn hub_start_of(input: &Value) -> Result<HubStart, String> {
    match input.get("start").and_then(Value::as_str).unwrap_or("auto") {
        "auto" => Ok(HubStart::Auto),
        "resume" => Ok(HubStart::Resume),
        "new" => Ok(HubStart::New),
        other => Err(format!("no such start: {other}")),
    }
}

/// The review bots' comments on a Jules task's PR, for the side sheet to choose from. Asked
/// for when a person opens the list, not on every poll: it is a round trip to GitHub.
pub(super) fn review_findings(server: &Server, id: &str) -> Result<Value, String> {
    Ok(json!({ "findings": jules_findings(&server.ctx, id)? }))
}

/// Post the chosen comments to the PR for Jules, in the name `gh` is signed in as.
pub(super) fn relay_findings(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let chosen = input
        .get("comments")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .map(crate::jules::Chosen::read)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    jules_relay(
        &server.ctx,
        id,
        &chosen,
        input.get("note").and_then(Value::as_str),
    )
}

/// The board's 「再取得」: read the task's issue again, on a click and never on a poll.
pub(super) fn fetch_issue(server: &Server, id: &str) -> Result<Value, String> {
    let task = crate::task::fetch_issue(&server.ctx, id, false)?;
    Ok(json!({ "task": task }))
}

/// The board's 「PR を確認」: the same pass as `adj task refresh`, whose answer the page shows
/// in its log before it redraws.
pub(super) fn refresh_tasks(server: &Server) -> Result<Value, String> {
    let checked = crate::task::refresh(&server.ctx)?;
    Ok(crate::transport::wording::refresh_json(&checked))
}

pub(super) fn nudge_hub(server: &Server) -> Result<Value, String> {
    let handed = crate::task::nudge(&server.ctx)?;
    Ok(json!({ "handed": crate::mail::Handed::from(&handed) }))
}

pub(super) fn answer_gate(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let decision = input
        .get("decision")
        .and_then(Value::as_str)
        .ok_or("a decision is required")?;
    if decision == "close" || decision == "dismiss" {
        let gate = crate::gate::close(
            &server.ctx,
            id,
            input.get("comment").and_then(Value::as_str),
            false,
        )?;
        return Ok(json!({ "gate": gate, "closed": true }));
    }
    let (gate, told) = crate::gate::answer(
        &server.ctx,
        id,
        decision,
        input.get("choice").and_then(Value::as_str),
        input.get("comment").and_then(Value::as_str),
    )?;
    Ok(json!({ "gate": gate, "present": told.is_present(), "woken": told.was_woken() }))
}
