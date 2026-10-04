use serde_json::{Value, json};

use super::auth::stored_token;
use super::daemon::live_resident;

// Moved to `registry`; re-exported until #331 so the `serve` callers keep their paths.
pub(in crate::cmd) use crate::registry::forget_board;
use crate::registry::served;
pub(super) use crate::registry::{Address, Served, address_of, addresses, record};
#[cfg(test)]
pub(super) use crate::registry::{boards_dir, prefer};
pub use crate::registry::{dashboards_running, note_board, running};

pub(super) fn board_url(port: u16, token: &str) -> String {
    format!("http://127.0.0.1:{port}/?token={token}")
}

/// The same board as the resident server serves it: under a path of its own.
pub(super) fn resident_board_url(port: u16, slug: &str, token: &str) -> String {
    format!("http://127.0.0.1:{port}/b/{slug}/?token={token}")
}

/// The board serving `repo`'s hub — its URL and whether the resident server is the one — or
/// `None`. Whoever started it, the token is the one every board on this machine shares.
fn located(repo: &crate::kernel::identity::RepoInfo) -> Option<(String, bool)> {
    let token = stored_token()?;
    match served(repo)? {
        Served::Resident(port) => Some((resident_board_url(port, &repo.slug, &token), true)),
        Served::Dedicated(port) => Some((board_url(port, &token), false)),
    }
}

/// `board` as `adj config` and `adjutant_config` report it: where it is, and whether the
/// resident server serves it. `null` when nothing does.
pub fn board_json(repo: &crate::kernel::identity::RepoInfo) -> Value {
    match located(repo) {
        Some((url, resident)) => json!({ "url": url, "resident": resident }),
        None => Value::Null,
    }
}

/// Where the resident server serves `repo`'s board, when a resident is live.
pub(super) fn resident_url(repo: &crate::kernel::identity::RepoInfo) -> Option<String> {
    let (_, port) = live_resident()?;
    let token = stored_token()?;
    note_board(repo);
    Some(resident_board_url(port, &repo.slug, &token))
}
