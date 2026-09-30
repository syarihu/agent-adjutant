//! Sessions the board asks for: one started with no task, and one linked to a task afterwards.
//!
//! A worker used to exist only as the far end of a task, and the board joins a task to its
//! worker by worktree, so a worker with no task had no card and nothing could give a running
//! one a task or another hub. Both halves live here so the HTTP layer only routes: starting
//! is a message to a hub (the hub owns creating worktrees, as it always has), and linking is
//! the one place that writes a task, a worker record and a message to the worker in step.

use std::path::Path;

use serde_json::{Value, json};

use super::same_path;
use super::serve::{Server, board_sessions, settings_now};
use super::{Context, HubStart, TabOutcome};
use crate::messaging::{self, Message};
use crate::runner;
use crate::session::{RepoHub, SessionRequest};
use crate::task::{self, Executor, Status};

/// The hub named by `hub` (a `hubs[].id`), or this board's own when none is named, with the
/// context to address it by. Refused the way `act_on_hub` refuses: a parent-task hub whose key
/// cannot be told cannot be addressed at all.
fn hub_context(
    server: &Server,
    id: Option<&str>,
    settings: crate::config::Settings,
) -> Result<(RepoHub, Context), String> {
    let repo = &server.ctx.repo;
    let hubs = messaging::all_repo_hubs(repo);
    let hub = match id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => hubs
            .into_iter()
            .find(|h| h.id == id)
            .ok_or_else(|| format!("no such hub: {id}"))?,
        None => hubs
            .into_iter()
            .find(|h| h.slug == repo.slug)
            .ok_or("the hub of this board is not known")?,
    };
    if hub.parent && hub.key.is_none() {
        return Err(
            "the key of this hub is not known; address it with adj hub --hub <key>".to_string(),
        );
    }
    let ctx = Context {
        repo: repo.clone().addressed(hub.key.as_deref())?,
        resolved: server.ctx.resolved.clone(),
        settings,
    };
    Ok((hub, ctx))
}

pub(super) fn input_of(body: &[u8]) -> Result<Value, String> {
    let input: Value = match body.is_empty() {
        true => json!({}),
        false => serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?,
    };
    match input.is_object() {
        true => Ok(input),
        false => Err("expected an object".to_string()),
    }
}

/// A string field, trimmed; blank and `null` are absent. Anything that is not a string is
/// refused rather than read as absent, as `text_field` does for a task update.
pub(super) fn text<'a>(input: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.trim()).filter(|s| !s.is_empty())),
        Some(other) => Err(format!("{key} has to be a string, not {other}")),
    }
}

/// The name a session's worktree is given when the person did not choose one: up to the first
/// four ASCII words of the instruction, else a dated one. The board proposes the same name
/// before the request is sent (`proposeName` in the page), so the two rules are kept alike.
fn derived_name(instruction: &str, epoch_secs: i64) -> String {
    let words: Vec<String> = instruction
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(4)
        .map(str::to_ascii_lowercase)
        .collect();
    // Cut as `task::slug` cuts a filename, so one long word cannot make a name a filesystem refuses.
    let mut name = words.join("-");
    name.truncate(32);
    Some(name.trim_matches('-').to_string())
        .filter(|name| !name.is_empty() && super::task::check_worktree_name(name).is_ok())
        .unwrap_or_else(|| dated_name(epoch_secs))
}

/// `session-YYYYMMDD-HHMM`, in UTC like every stamp the server writes.
fn dated_name(epoch_secs: i64) -> String {
    let stamp = messaging::utc_stamp(epoch_secs);
    format!("session-{}-{}", &stamp[..8], &stamp[9..13])
}

/// Start the hub of `ctx` when the message just left for it found nobody there and the server
/// can start one. Only once the message is in the inbox: a hub started first would find
/// nothing to do and wait, and one that failed to start would leave the person unsure whether
/// the message was sent.
fn start_if_stopped(
    server: &Server,
    ctx: &Context,
    delivered: &super::Delivered,
) -> (bool, Option<String>) {
    if delivered.delivery.present
        || !server.resident
        || !super::hub_startable(&ctx.settings.terminal)
    {
        return (false, None);
    }
    match super::start_hub(ctx, HubStart::Auto) {
        Ok(TabOutcome::Opened(_)) => (true, None),
        Ok(TabOutcome::AlreadyRunning(_)) => (false, None),
        Err(e) => (false, Some(e)),
    }
}

/// The inbox file name of a delivered message, which is what `hubs[].inbox[].name` calls it.
fn inbox_name(delivered: &super::Delivered) -> Option<String> {
    delivered
        .delivery
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
}

/// Ask a hub to start a session with no task.
///
/// Nothing is written but the message: the hub picks the final name (`worktree-path
/// --unique`), creates the worktree and starts the worker, so the checks here are the ones
/// that spare the person a request that can only fail — a name git refuses, an agent this
/// board cannot start, a machine with every worker slot taken.
pub(super) fn start_request(server: &Server, body: &[u8]) -> Result<Value, String> {
    let input = input_of(body)?;
    let settings = settings_now(server);
    // Optional: the worker greets the person and waits when there is none.
    let instruction = text(&input, "instruction")?.unwrap_or("");
    // The brief writes a missing instruction as `-`, so the hub could not tell this one apart.
    if instruction.trim() == "-" {
        return Err("an instruction of only `-` means no instruction; leave it empty".to_string());
    }
    let configured = runner::agent_from_runner(
        settings
            .agent_runner
            .as_deref()
            .unwrap_or(runner::DEFAULT_AGENT_RUNNER),
    );
    let agent = text(&input, "agent")?.unwrap_or(&configured);
    if agent != configured {
        return Err(format!(
            "only {configured} can be started: it is the agent the worker runner is set to"
        ));
    }
    let name = match text(&input, "worktreeName")? {
        Some(name) => {
            super::task::check_worktree_name(name)?;
            name.to_string()
        }
        None => derived_name(instruction, messaging::now_secs()),
    };
    // Unlike a task, a session request has no record to wait in for a free slot, so a full
    // machine is refused here rather than left for the hub to turn away. The check is advisory:
    // nothing reserves the slot, so two requests at once can both pass, and the hub's exit
    // code 3 from `adjutant work` is what finally turns the loser away.
    if let Some(max) = settings.max_workers {
        let mut candidates = crate::repo::linked_worktrees(&server.ctx.repo.main)?;
        candidates.push(server.ctx.repo.main.clone());
        let busy = messaging::busy_worktrees(&candidates, None);
        if busy.len() >= max as usize {
            return Err(format!(
                "worker limit reached: {} of maxWorkers {max} are running; \
                 start it when one finishes",
                busy.len()
            ));
        }
    }
    let (hub, ctx) = hub_context(server, text(&input, "hub")?, settings)?;
    let request = SessionRequest {
        agent: agent.to_string(),
        worktree_name: name.clone(),
        instruction: instruction.to_string(),
    };
    let message = Message {
        from: "dashboard".to_string(),
        // Deliberately none, as for a task: the sender is a person at a browser.
        worktree: None,
        kind: "session".to_string(),
        subject: format!("start a session: {name}"),
        body: request.render_request(),
    };
    let delivered = super::deliver_to_hub(&ctx, &message)?;
    let (started, start_error) = start_if_stopped(server, &ctx, &delivered);
    let mut reply = json!({
        "handed": { "present": delivered.delivery.present, "woken": delivered.woken },
        "hubStarted": started,
        "worktreeName": name,
        "hub": hub.id,
        "message": inbox_name(&delivered),
    });
    if let Some(e) = start_error {
        reply["hubStartError"] = json!(e);
    }
    Ok(reply)
}

/// Join the session `id` to a task, and to the hub that task belongs to.
///
/// The task is either an existing one (`task`) or a new one (`newTask`, the fields
/// `POST /api/tasks` takes) and is looked up or made in the task directory of the hub named
/// by `hub` — that directory is what says which hub a task belongs to, and the worker's
/// record follows it.
pub(super) fn link(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input = input_of(body)?;
    let settings = settings_now(server);
    let session = board_sessions(server, &settings)
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("no such session: {id}"))?;
    if session.kind != "worker" {
        return Err("only a worker session can be linked to a task".to_string());
    }
    let worktree = session.worktree.as_str();
    if messaging::read_json(&messaging::worker_record_path(Path::new(worktree))).is_none() {
        return Err("the session has not started yet".to_string());
    }
    // A task linked to a worker nobody is running would sit as `dispatched` in a worktree
    // that never reads the notice.
    if !messaging::worker_status(Path::new(worktree)).present {
        return Err("the session has ended; resume it first".to_string());
    }
    // Checked before anything is written: a task made or changed for a phase the record would
    // then refuse is the half-linked state the undo below exists for.
    let phase = text(&input, "phase")?;
    if let Some(phase) = phase.filter(|p| !messaging::PHASES.contains(p)) {
        return Err(format!(
            "no such phase: {phase} (one of {})",
            messaging::PHASES.join(", ")
        ));
    }
    let (hub, ctx) = hub_context(server, text(&input, "hub")?, settings)?;
    let dir = super::task::dir(&ctx);
    let held = session.task.as_deref();

    // Written first, and undone if the worker's record cannot be: a task naming a worktree
    // whose worker still reports for nothing is the half-linked state to avoid.
    let (linked, undo) = match (text(&input, "task")?, input.get("newTask")) {
        (Some(_), Some(_)) => return Err("give either task or newTask, not both".to_string()),
        (Some(task_id), None) => {
            if !task::is_plain_id(task_id) {
                return Err(format!("no such task: {task_id}"));
            }
            // An existing task already has its issue or its reason not to; only a task made by
            // this link can ask the hub to file one.
            if text(&input, "kind")? == Some("file-and-start") {
                return Err(
                    "file-and-start needs a newTask: an existing task is not filed".to_string(),
                );
            }
            if held.is_some_and(|held| held != task_id) {
                return Err(format!(
                    "this session already has a task: {}",
                    held.unwrap_or_default()
                ));
            }
            // Read to choose the status; the checks that matter are made again under the lock.
            let status = match task::load(&dir, task_id)?.status {
                Status::Pr => "pr",
                _ => "dispatched",
            };
            let mut before = Value::Null;
            let (updated, _) = super::task::update_checked(
                &ctx,
                task_id,
                &json!({ "worktree": worktree, "status": status, "note": "", "handOver": false }),
                |existing| {
                    if matches!(existing.status, Status::Done | Status::Cancelled) {
                        return Err(format!("{task_id} is finished"));
                    }
                    if existing.executor == Executor::Jules {
                        return Err(format!("{task_id} is for Jules; a session cannot take it"));
                    }
                    if let Some(other) = existing
                        .worktree
                        .as_deref()
                        .filter(|w| !same_path(w, worktree))
                        && messaging::worker_status(Path::new(other)).present
                    {
                        return Err(format!("{task_id} already has a worker running in {other}"));
                    }
                    before = json!({
                        "worktree": existing.worktree,
                        "status": existing.status.as_str(),
                        "note": existing.note,
                        "handOver": false,
                    });
                    Ok(())
                },
            )?;
            (updated, Undo::Restore(before))
        }
        (None, Some(new_task)) => {
            if let Some(held) = held {
                return Err(format!("this session already has a task: {held}"));
            }
            let mut fields = new_task
                .as_object()
                .cloned()
                .ok_or("newTask has to be an object")?;
            if fields
                .get("executor")
                .and_then(Value::as_str)
                .and_then(Executor::parse)
                == Some(Executor::Jules)
            {
                return Err("a session cannot take a task for Jules".to_string());
            }
            // What the link decides itself, whatever the caller sent.
            fields.remove("worktreeName");
            fields.insert("status".to_string(), json!("dispatched"));
            fields.insert("worktree".to_string(), json!(worktree));
            fields.insert("handOver".to_string(), json!(false));
            let (created, _) = super::task::create(&ctx, &Value::Object(fields))?;
            (created, Undo::Remove)
        }
        (None, None) => return Err("a task or a newTask is required".to_string()),
    };

    let made_here = matches!(undo, Undo::Remove);
    if let Err(e) =
        messaging::relink_worker(Path::new(worktree), hub.key.as_deref(), &linked.id, phase)
    {
        match undo {
            Undo::Remove => {
                let _ = std::fs::remove_file(task::path_of(&dir, &linked.id));
                let _ = std::fs::remove_file(dir.join(format!("{}.lock", linked.id)));
            }
            Undo::Restore(before) => {
                let _ = super::task::update(&ctx, &linked.id, &before);
            }
        }
        return Err(e);
    }
    // Only a task this link made: an existing one may carry `file-and-start` from its own request,
    // which was already handed to a hub.
    let files_issue = made_here && linked.kind == task::Kind::FileAndStart;
    let filing = if files_issue {
        "The hub files the issue for this task; its URL reaches you as `[issue <id>] <url>` \
         and is recorded on the task.\n\n"
    } else if made_here && linked.kind == task::Kind::Start && linked.issue_url.is_none() {
        // `start` reads as "an issue that already exists" in the request below.
        "This task has no issue and none will be filed for it; the Kind line below does not \
         mean one exists.\n\n"
    } else {
        ""
    };
    let body = format!(
        "From now on your Task record is {} and your hub is {}. Fetch adj-worker \
         (`adj skill adj-worker`) and follow it from where the work stands.\n\n{filing}{}",
        linked.id,
        ctx.repo.hub_name,
        task::render_request(&linked)
    );
    super::deliver_to_worker(
        &ctx,
        Path::new(worktree),
        "dashboard",
        &format!("[linked {}] this session is now a task's worker", linked.id),
        &body,
        None,
    )
    .map_err(|e| {
        format!(
            "{} is linked, but the session could not be told: {e}",
            linked.id
        )
    })?;
    let mut reply = json!({
        "task": linked,
        "session": { "id": session.id, "task": linked.id, "hub": hub.id },
    });
    if files_issue {
        // The link stands whatever happens here: the worker already has its task, and a hub that
        // could not be told is something to say, not to undo.
        let message = Message {
            from: "dashboard".to_string(),
            worktree: None,
            kind: "file-issue".to_string(),
            subject: format!("[file {}] {}", linked.id, linked.title),
            body: format!(
                "{}\n## Worker running in {worktree}\n",
                task::render_request(&linked)
            ),
        };
        match super::deliver_to_hub(&ctx, &message) {
            Ok(delivered) => {
                reply["fileIssue"] = json!({
                    "handed": { "present": delivered.delivery.present, "woken": delivered.woken },
                    "message": inbox_name(&delivered),
                });
                let (started, error) = start_if_stopped(server, &ctx, &delivered);
                reply["hubStarted"] = json!(started);
                if let Some(e) = error {
                    reply["hubStartError"] = json!(e);
                }
            }
            Err(e) => {
                reply["fileIssueError"] = json!(e);
                // The worker was told the hub would file it: say it will not, in the words it
                // already handles, so it does not wait for a URL.
                let _ = super::deliver_to_worker(
                    &ctx,
                    Path::new(worktree),
                    "dashboard",
                    &format!("[issue {}] not filed: the hub could not be told", linked.id),
                    "Nothing will file an issue for this task automatically. Carry on without one.",
                    None,
                );
            }
        }
    }
    Ok(reply)
}

/// How to take back the task write when the worker's record cannot be written.
enum Undo {
    /// The record was made for this link: remove it.
    Remove,
    /// The record existed: put these fields back.
    Restore(Value),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_the_first_four_words_of_the_instruction_or_a_dated_one() {
        let at = 1_790_000_000;
        assert_eq!(
            derived_name("Look at the flaky upload test", at),
            "look-at-the-flaky"
        );
        assert_eq!(derived_name("  Retry, the upload!", at), "retry-the-upload");
        // Nothing ASCII to make a name of.
        assert_eq!(
            derived_name("アップロードの再試行を調べる", at),
            "session-20260921-1413"
        );
        assert_eq!(derived_name("   ", at), "session-20260921-1413");
        // Cut at 32 characters, with no separator left dangling.
        let long = derived_name(&format!("{} tail", "a".repeat(40)), at);
        assert_eq!(long, "a".repeat(32));
        assert_eq!(
            derived_name("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbb", at),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }
}
