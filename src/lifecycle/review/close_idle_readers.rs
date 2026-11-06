use std::path::Path;

use crate::infra::clock::now_secs;
use crate::lifecycle::review::{CloseError, close_reader_of_with, stop_reader};
use crate::others::{self, HungReader, Record};
use crate::registry::{self, AgentSession, AgentStatus, Liveness, Recorded};
use crate::task::pr_ref;

/// The reason `close_reader_of_with` asks for. Only a `running` read is failed with it, and this
/// closes `ready` ones alone, so it is never written: it is here because the close needs one.
const IDLE: &str = "nobody talked to the reader for a while";

/// Close the readers that have handed in their read-through and that nobody has talked to for
/// `readerIdleMinutes`:
/// a reader that sits in its tmux window after handing in is there so the person can ask about the
/// diff. If nobody talks to it for a while the session is closed so tabs do not pile up; the
/// read-through, the drafts and the record stay.
///
/// Returns the ones that could not be closed, by id, and why. A reader that cannot be told alive
/// or dead, or cannot be stopped, is left as it is and tried again at the next pass; one failing
/// does not stop the others. No pass on the queue: a ready read holds no place in the queue.
pub fn close_idle_readers(root: &Path) -> Vec<(String, String)> {
    if !has_ready_reader(root) {
        return Vec::new();
    }
    let ctx = match registry::review_context(root) {
        Ok(ctx) => ctx,
        Err(why) => return vec![(String::new(), why)],
    };
    let minutes = ctx.settings.reader_idle_minutes;
    if minutes <= 0.0 {
        return Vec::new();
    }
    let idle_secs = ((minutes * 60.0).round() as i64).max(1);
    close_idle_readers_with(
        root,
        now_secs(),
        idle_secs,
        &|id| registry::agent_session(root, id),
        &registry::hub_process_liveness,
        &|reader| stop_reader(&ctx, reader),
    )
}

/// Whether any PR has a reader open on a read that has handed in.
pub(super) fn has_ready_reader(root: &Path) -> bool {
    others::list(root).iter().any(|r| {
        r.ai.as_ref()
            .is_some_and(|ai| ai.status == "ready" && ai.reader_open())
    })
}

/// `close_idle_readers`, with the time, the idle threshold, the row lookup, the way a process is
/// asked after and the way it is stopped handed in.
pub(crate) fn close_idle_readers_with(
    root: &Path,
    now: i64,
    idle_secs: i64,
    row: &dyn Fn(&str) -> Recorded<AgentSession>,
    alive: &dyn Fn(u32, Option<&str>) -> Liveness,
    stop: &dyn Fn(&HungReader) -> Result<(), String>,
) -> Vec<(String, String)> {
    let mut failures = Vec::new();
    for listed in others::list(root) {
        let Some(session) = idle_session(&listed, now, idle_secs, row) else {
            continue;
        };
        let Some(pr) = pr_ref(&listed.url, None) else {
            continue;
        };
        // The record and the row are read again just before closing: the list was taken before
        // the readers ahead of this one were stopped, and meanwhile the read may have been asked
        // for again (a new session, running) or a person may have started talking. Only the
        // session judged idle is closed. A prompt typed after this look and before the pane is
        // closed is not seen: closing that window needs a lock the agent's hooks do not take, and
        // the read-through stays either way.
        let record = match others::record(root, &pr) {
            Ok(Some(record)) => record,
            Ok(None) => continue,
            Err(why) => {
                failures.push((listed.id, why));
                continue;
            }
        };
        if idle_session(&record, now, idle_secs, row).as_deref() != Some(session.as_str()) {
            continue;
        }
        match close_reader_of_with(root, &pr, Some(&session), now, IDLE, alive, stop) {
            // Nothing open to close, or the record changed under it: nothing to say.
            Ok(_) | Err(CloseError::NoReader | CloseError::NoRecord) => {}
            Err(CloseError::Failed(why)) => failures.push((record.id, why)),
        }
    }
    failures
}

/// The session of a record's reader when it has handed in and has sat idle for `idle_secs` or
/// more; `None` when it is to be left open.
fn idle_session(
    record: &Record,
    now: i64,
    idle_secs: i64,
    row: &dyn Fn(&str) -> Recorded<AgentSession>,
) -> Option<String> {
    let ai = record.ai.as_ref()?;
    if ai.status != "ready" || !ai.reader_open() {
        return None;
    }
    let session_id = ai.session_id.as_deref()?;
    match row(session_id) {
        Recorded::Found(session) if session_is_idle(&session, now, idle_secs) => {
            Some(session_id.to_string())
        }
        _ => None,
    }
}

/// Whether an agent session row says nobody has talked to it for at least `idle_secs`.
///
/// The last talk is `updatedAt`, which moves only when the status does (a permission prompt
/// moves it too, and keeps the reader while it is up): a prompt starts a turn
/// (`running`) and its end makes it `done`, so it is the later of the last prompt and the end of
/// the answer to it. `lastPromptAt` alone would count a long answer as idle time, and
/// `lastEventAt` is a heartbeat that moves while nobody talks. Only a turn that has ended counts
/// (`idle`, `done`, `failed`): `running`, `waiting`, no status and a status this binary does not
/// know all keep the reader.
fn session_is_idle(session: &AgentSession, now: i64, idle_secs: i64) -> bool {
    if !matches!(
        session.status,
        Some(AgentStatus::Idle | AgentStatus::Done | AgentStatus::Failed)
    ) {
        return false;
    }
    if !session.subagents.is_empty() {
        return false;
    }
    let Some(last_activity) = session.updated_at else {
        return false;
    };
    now.saturating_sub(last_activity) >= idle_secs
}
