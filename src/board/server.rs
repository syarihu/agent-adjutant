use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

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
    /// The last line each session's pane showed, for the pages that ask for it (`?lines=1`).
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
