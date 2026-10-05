//! The board's cache of what Jules says about each session a card follows, asked from a
//! thread of its own so that a slow API makes a badge stale rather than the board.

use serde::Serialize;

use crate::jules;
use crate::task;

// ── the board's view of the sessions ─────────────────────────────────

/// How long an answer about a session is reused. The board polls every two seconds; the
/// session's state changes over minutes. Asking once in this long keeps the badge current
/// enough to act on without sending the API a request per poll per open tab.
const FRESH_FOR: std::time::Duration = std::time::Duration::from_secs(45);

/// The last answer about each session, and whether a question is already out.
///
/// The board asks from a thread of its own, never from the request that serves the page: an
/// API that is slow to answer should make a badge a little stale, not the whole board.
#[derive(Default)]
pub struct Watch {
    seen: std::sync::Mutex<std::collections::HashMap<String, Seen>>,
}

/// What the board shows about a task's Jules session: the three objects the page reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum JulesSeen {
    /// Not answered yet. `checking` is always `true`; it is the key the page reads.
    Checking {
        session: String,
        checking: bool,
    },
    Found {
        session: String,
        state: String,
        url: Option<String>,
        pr: Option<String>,
        working: bool,
    },
    Failed {
        session: String,
        error: String,
    },
}

impl JulesSeen {
    /// The session this is about, whichever answer it is.
    pub fn session(&self) -> &str {
        match self {
            Self::Checking { session, .. }
            | Self::Found { session, .. }
            | Self::Failed { session, .. } => session,
        }
    }
}

struct Seen {
    at: std::time::Instant,
    asking: bool,
    answer: Option<Result<jules::Session, String>>,
}

impl Watch {
    /// What the board shows for a task: the last answer about its session, or `None` for a
    /// task Jules is not working on. Asks again in the background when the answer is old.
    ///
    /// Only for a task still in progress or in review: once it is done nobody reads the badge,
    /// and a session that is left alone does not change.
    pub fn look(
        self: &std::sync::Arc<Self>,
        ctx: &crate::registry::Context,
        key: &crate::infra::terminal::Hook,
        task: &task::Task,
    ) -> Option<JulesSeen> {
        let session = task.jules_session.clone()?;
        if !matches!(task.status, task::Status::Dispatched | task::Status::Pr) {
            return None;
        }
        let task_id = task.id.clone();
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let entry = seen.entry(session.clone()).or_insert(Seen {
            at: std::time::Instant::now(),
            asking: false,
            answer: None,
        });
        let stale = entry.answer.is_none() || entry.at.elapsed() >= FRESH_FOR;
        if stale && !entry.asking {
            entry.asking = true;
            let watch = std::sync::Arc::clone(self);
            let ctx = ctx.clone();
            let key = key.clone();
            let asked = session.clone();
            std::thread::spawn(move || watch.ask(&ctx, &key, &task_id, &asked));
        }
        // Nothing that changes by itself goes in here — an age in seconds, say. The page redraws
        // whenever the state it polls differs from the last, and a field that ticks would have
        // it redraw every two seconds.
        Some(match &entry.answer {
            None => JulesSeen::Checking {
                session,
                checking: true,
            },
            Some(Ok(found)) => JulesSeen::Found {
                session,
                state: found.state.clone(),
                url: found.url.clone(),
                pr: found.pr.clone(),
                working: jules::working(&found.state),
            },
            Some(Err(why)) => JulesSeen::Failed {
                session,
                error: why.clone(),
            },
        })
    }

    /// Forget the sessions no card showed on this poll: finished tasks, sessions replaced.
    /// One still being asked about is kept, so its answer has somewhere to land. Without this
    /// the map grows with every session the board has ever shown.
    pub fn keep_only(&self, shown: &std::collections::HashSet<String>) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|session, entry| entry.asking || shown.contains(session));
    }

    fn ask(
        &self,
        ctx: &crate::registry::Context,
        key: &crate::infra::terminal::Hook,
        task_id: &str,
        session: &str,
    ) {
        let answer = jules::get(key, session);
        if let Ok(found) = &answer
            && let Err(e) = jules::follow(ctx, task_id, found)
        {
            eprintln!("adj serve: could not record the pull request of {task_id}: {e}");
        }
        if let Ok(found) = &answer
            && let Err(e) = jules::announce_review(ctx, task_id, found)
        {
            eprintln!("adj serve: could not bring up the review of {task_id}: {e}");
        }
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = seen.get_mut(session) {
            entry.at = std::time::Instant::now();
            entry.asking = false;
            entry.answer = Some(answer);
        }
    }
}
