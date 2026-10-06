use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::LastLines;
use crate::board::jobs::Watch as JulesWatch;
use crate::board::jobs::{HubTitles, PrPoll};

/// Everything a connection needs. Shared across threads, read-only after startup — the
/// state that changes lives on disk, where the hub and its workers can also reach it.
pub(crate) struct Server {
    pub ctx: crate::registry::Context,
    pub token: String,
    pub port: u16,
    /// Whether the resident server is the one answering, which serves this board at a path
    /// of its own. The page reads it from the state.
    pub resident: bool,
    /// What Jules last said about each session a card follows. The one thing here that
    /// changes after startup, and it is a cache: the record on disk stays the answer.
    pub jules: Arc<JulesWatch>,
    /// The titles of the parent tasks the hubs are named after: a cache of the tracker's, kept
    /// on disk, and read from a thread of its own.
    pub hub_titles: Arc<HubTitles>,
    /// The last line each session's pane showed, for the pages that ask for it (`?lines=1`, or `?lines=hub` for the hubs').
    /// A cache: the pane is the answer.
    pub last_lines: Arc<LastLines>,
    /// What tmux this machine has, when the board may open terminals on it: only the resident
    /// server serves one, and only where `tmux -V` answered when it started.
    pub tmux: Option<(u32, u32)>,
    /// How many board terminals are open across every board, which is what is capped.
    pub terminals: Arc<AtomicUsize>,
    /// The resident server's PR poll, whose health the page shows. `None` on a board that is
    /// served by itself: nothing polls there, and the page says nothing about it.
    pub pr_poll: Option<Arc<PrPoll>>,
}

/// The settings as `adj work` would read them now. Resolved on every poll rather than taken
/// from the ones the server started with, because `adj work` reads the config each time it
/// runs, and a limit changed under a running board would otherwise show one number while
/// dispatches are refused by another.
pub fn settings_now(server: &Server) -> crate::kernel::config::Settings {
    crate::kernel::config::resolve_config(&server.ctx.repo.nwo)
        .map(|resolved| resolved.settings)
        .unwrap_or_else(|_| server.ctx.settings.clone())
}

/// How long a hub's restart keeps refusing another one after it has answered. The answer comes
/// when the tmux window is open, a moment before the new `adj hub` has registered, and a second
/// restart in between would stop the new hub or open a stray window beside it. The page's
/// `RESTART_MS` is the same length: it shows 「再起動しています…」 for as long.
pub(super) const RESTART_HOLD: Duration = Duration::from_secs(45);

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
pub struct Restarting {
    key: String,
    held: bool,
}

impl Restarting {
    pub fn claim(key: &str, what: &str) -> Result<Restarting, String> {
        Self::claim_at(key, what, Instant::now())
    }

    /// `claim`, with the time it is made at handed in so that the hold can be tested.
    pub(super) fn claim_at(key: &str, what: &str, now: Instant) -> Result<Restarting, String> {
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
    pub fn hold(self) {
        self.hold_at(Instant::now());
    }

    pub(super) fn hold_at(mut self, now: Instant) {
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

/// Numbers the sessions this process makes to open a terminal on, so that two requests never
/// share a name.
pub(crate) static NEXT: AtomicUsize = AtomicUsize::new(1);
