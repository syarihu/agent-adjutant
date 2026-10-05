use std::path::Path;

use serde::Serialize;

use super::auth::stored_token;

use crate::registry::{Served, live_resident, note_board, served};

pub(super) fn board_url(port: u16, token: &str) -> String {
    format!("http://127.0.0.1:{port}/?token={token}")
}

/// The same board as the resident server serves it: under a path of its own.
pub(super) fn resident_board_url(port: u16, slug: &str, token: &str) -> String {
    format!("http://127.0.0.1:{port}/b/{slug}/?token={token}")
}

/// A running board as `adj config` and `adjutant_config` report it: where it is, and whether
/// the resident server serves it.
#[derive(Serialize)]
pub struct BoardAt {
    url: String,
    resident: bool,
}

/// The board serving `repo`'s hub, or `None` when nothing does (`null` in the report).
/// Whoever started it, the token is the one every board on this machine shares.
pub fn located(root: &Path, repo: &crate::kernel::identity::RepoInfo) -> Option<BoardAt> {
    let token = stored_token(root)?;
    match served(root, repo)? {
        Served::Resident(port) => Some(BoardAt {
            url: resident_board_url(port, &repo.slug, &token),
            resident: true,
        }),
        Served::Dedicated(port) => Some(BoardAt {
            url: board_url(port, &token),
            resident: false,
        }),
    }
}

/// Where the resident server serves `repo`'s board, when a resident is live.
pub(super) fn resident_url(
    root: &Path,
    repo: &crate::kernel::identity::RepoInfo,
) -> Option<String> {
    let (_, port) = live_resident(root)?;
    let token = stored_token(root)?;
    note_board(root, repo);
    Some(resident_board_url(port, &repo.slug, &token))
}
