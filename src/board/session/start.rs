use crate::board::{Server, SessionRequest, settings_now};
use crate::kernel::runner;
use crate::lifecycle::hub::{HubStart, TabOutcome, hub_startable, start_hub};
use crate::mail::{self, DeliveryOutcome, Message, RepoHub};
use crate::registry::{self, Context};
use crate::task;

/// The hub named by `hub` (a `hubs[].id`), or this board's own when none is named, with the
/// context to address it by. Refused the way a hub's start from the board
/// (`hub::start_context`) refuses: a parent-task hub whose key cannot be told cannot be
/// addressed at all.
pub(super) fn hub_context(
    server: &Server,
    id: Option<&str>,
    settings: crate::kernel::config::Settings,
) -> Result<(RepoHub, Context), String> {
    let repo = &server.ctx.repo;
    let hubs = mail::all_repo_hubs(&server.ctx.state, repo);
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
        state: server.ctx.state.clone(),
        settings,
    };
    Ok((hub, ctx))
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
        .filter(|name| !name.is_empty() && task::check_worktree_name(name).is_ok())
        .unwrap_or_else(|| dated_name(epoch_secs))
}

/// `session-YYYYMMDD-HHMM`, in UTC like every stamp the server writes.
fn dated_name(epoch_secs: i64) -> String {
    let stamp = crate::infra::clock::utc_stamp(epoch_secs);
    format!("session-{}-{}", &stamp[..8], &stamp[9..13])
}

/// Start the hub of `ctx` when the message just left for it found nobody there and the server
/// can start one. Only once the message is in the inbox: a hub started first would find
/// nothing to do and wait, and one that failed to start would leave the person unsure whether
/// the message was sent.
pub(super) fn start_if_stopped(
    server: &Server,
    ctx: &Context,
    delivered: &DeliveryOutcome,
) -> Result<bool, String> {
    if delivered.is_present() || !server.resident || !hub_startable(&ctx.settings.terminal) {
        return Ok(false);
    }
    match start_hub(ctx, HubStart::Auto) {
        Ok(TabOutcome::Opened(_)) => Ok(true),
        Ok(TabOutcome::AlreadyRunning(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// The inbox file name of a delivered message, which is what `hubs[].inbox[].name` calls it.
pub fn inbox_name(delivered: &DeliveryOutcome) -> Option<String> {
    delivered
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
}

/// What the person asked for when starting a session with no task. Strings are trimmed and a
/// blank one is absent; an absent instruction is "".
pub struct StartSession {
    pub instruction: String,
    pub agent: Option<String>,
    pub worktree_name: Option<String>,
    pub hub: Option<String>,
}

/// What asking a hub to start a session came to.
pub struct SessionAsked {
    pub delivered: DeliveryOutcome,
    pub worktree_name: String,
    /// The `hubs[].id` of the hub that was asked.
    pub hub: String,
    /// Whether a stopped hub was started for the message, or why it could not be.
    pub hub_started: Result<bool, String>,
}

/// Ask a hub to start a session with no task.
///
/// Nothing is written but the message: the hub picks the final name (`worktree-path
/// --unique`), creates the worktree and starts the worker, so the checks here are the ones
/// that spare the person a request that can only fail — a name git refuses, an agent this
/// board cannot start, a machine with every worker slot taken.
pub fn start(server: &Server, request: StartSession) -> Result<SessionAsked, String> {
    let settings = settings_now(server);
    let configured = runner::agent_from_runner(
        settings
            .agent_runner
            .as_deref()
            .unwrap_or(runner::DEFAULT_AGENT_RUNNER),
    );
    let agent = request.agent.as_deref().unwrap_or(&configured);
    if agent != configured {
        return Err(format!(
            "only {configured} can be started: it is the agent the worker runner is set to"
        ));
    }
    let name = match request.worktree_name {
        Some(name) => {
            task::check_worktree_name(&name)?;
            name
        }
        None => derived_name(&request.instruction, crate::infra::clock::now_secs()),
    };
    // Unlike a task, a session request has no record to wait in for a free slot, so a full
    // machine is refused here rather than left for the hub to turn away. The check is advisory:
    // nothing reserves the slot, so two requests at once can both pass, and the hub's exit
    // code 3 from `adjutant work` is what finally turns the loser away.
    if let Some(max) = settings.max_workers {
        let mut candidates = crate::kernel::identity::linked_worktrees(&server.ctx.repo.main)?;
        candidates.push(server.ctx.repo.main.clone());
        let busy = registry::busy_worktrees(&candidates, None);
        if busy.len() >= max as usize {
            return Err(format!(
                "worker limit reached: {} of maxWorkers {max} are running; \
                 start it when one finishes",
                busy.len()
            ));
        }
    }
    let (hub, ctx) = hub_context(server, request.hub.as_deref(), settings)?;
    let session = SessionRequest {
        agent: agent.to_string(),
        worktree_name: name.clone(),
        instruction: request.instruction,
    };
    let message = Message {
        from: "dashboard".to_string(),
        // Deliberately none, as for a task: the sender is a person at a browser.
        worktree: None,
        kind: "session".to_string(),
        subject: format!("start a session: {name}"),
        body: session.render_request(),
    };
    let delivered = mail::deliver_to_hub(&ctx, &message)?;
    let hub_started = start_if_stopped(server, &ctx, &delivered);
    Ok(SessionAsked {
        delivered,
        worktree_name: name,
        hub: hub.id,
        hub_started,
    })
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
