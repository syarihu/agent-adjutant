//! The gates a session waits on, read once per poll.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::gate;
use crate::session;

fn session_waiting(
    hub: &crate::mail::RepoHub,
    open: &[&gate::Gate],
) -> Option<session::SessionWaiting> {
    let first = open.first()?;
    Some(session::SessionWaiting {
        id: first.id.clone(),
        kind: first.kind.as_str().to_string(),
        hub: hub.id.clone(),
        slug: hub.slug.clone(),
        title: Some(first.title.clone()).filter(|t| !t.is_empty()),
        opened_at: first.opened_at.clone(),
        count: open.len(),
        options: if first.options.is_empty() {
            first.kind.default_options()
        } else {
            first.options.clone()
        },
        choices: first
            .choices
            .iter()
            .map(|c| session::WaitingChoice {
                id: c.id.clone(),
                label: c.label.clone(),
            })
            .collect(),
        focus: first
            .focus
            .as_deref()
            .map(|f| cut_chars(f, WAITING_FOCUS_CHARS))
            .filter(|f| !f.is_empty()),
    })
}

/// How much of a gate's focus the Sessions banner carries.
const WAITING_FOCUS_CHARS: usize = 400;

/// `text` cut to at most `max` characters, on a character boundary, with an ellipsis when cut.
pub fn cut_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text.to_string(),
    }
}

/// One hub's open gates, and what could show their workers moved on, read once per poll.
pub(super) struct HubGates {
    pub(super) open: Vec<gate::Gate>,
    signals: Vec<gate::Gate>,
}

impl HubGates {
    fn read(state_dir: &Path, slug: &str) -> Self {
        let open = gate::list(state_dir, slug, gate::Shelf::Open);
        // Left unread when nothing waits on a worker: the archives only grow.
        let signals = if open.iter().any(|g| g.wait && !g.answered_by_hub()) {
            gate::resume_signals(state_dir, slug, &open)
        } else {
            Vec::new()
        };
        HubGates { open, signals }
    }
}

/// The gates of the hubs a poll reaches, each hub's read the first time one of its sessions
/// asks.
pub(super) struct GateCache {
    pub(super) state_dir: PathBuf,
    pub(super) read: HashMap<String, HubGates>,
}

impl GateCache {
    pub(super) fn of(&mut self, slug: &str) -> &HubGates {
        self.read
            .entry(slug.to_string())
            .or_insert_with(|| HubGates::read(&self.state_dir, slug))
    }
}

/// The gate a worker is waiting to have answered: the oldest still open in its hub's gate
/// directory that it opened from `worktree` and has not moved on from. "Moved on" is judged as
/// the board's gate sweep judges it, from the same signals, but only reads: a hub's directory
/// is closed by that hub's board.
pub(super) fn waiting_worker(
    hub: &crate::mail::RepoHub,
    gates: &HubGates,
    worktree: &str,
    started: Option<&str>,
    phase_at: Option<i64>,
) -> Option<session::SessionWaiting> {
    let phase_at = phase_at.map(crate::infra::clock::utc_stamp);
    let open: Vec<&gate::Gate> = gates
        .open
        .iter()
        .filter(|g| g.worktree == worktree && g.wait && !g.answered_by_hub())
        .filter(|g| {
            let later = gates
                .signals
                .iter()
                .filter(|s| s.worktree == g.worktree && s.id != g.id && s.opened_at > g.opened_at)
                .map(|s| s.opened_at.as_str())
                .min();
            gate::resumed_at(g, started, phase_at.as_deref(), later).is_none()
        })
        .collect();
    session_waiting(hub, &open)
}

/// What a hub is waiting on: the gates it opened for a person to answer.
pub(super) fn waiting_hub(
    hub: &crate::mail::RepoHub,
    gates: &[gate::Gate],
) -> Option<session::SessionWaiting> {
    let open: Vec<&gate::Gate> = gates
        .iter()
        .filter(|g| g.wait && g.answered_by_hub())
        .collect();
    session_waiting(hub, &open)
}
