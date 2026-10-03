//! Which boards are running and where.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::messaging;

use super::auth::stored_token;
use super::daemon::live_resident;

pub(super) fn board_url(port: u16, token: &str) -> String {
    format!("http://127.0.0.1:{port}/?token={token}")
}

/// The same board as the resident server serves it: under a path of its own.
pub(super) fn resident_board_url(port: u16, slug: &str, token: &str) -> String {
    format!("http://127.0.0.1:{port}/b/{slug}/?token={token}")
}

/// Where a board for a hub is being served from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Served {
    Resident(u16),
    Dedicated(u16),
}

/// The resident server when there is one, the hub's own board otherwise. The resident wins
/// because it is the one that outlives the hub, and a URL that named the hub's board would
/// stop working when the hub did.
pub(super) fn prefer(resident: Option<u16>, dedicated: Option<u16>) -> Option<Served> {
    resident
        .map(Served::Resident)
        .or(dedicated.map(Served::Dedicated))
}

fn served(repo: &crate::repo::RepoInfo) -> Option<Served> {
    let resident = live_resident().map(|(_, port)| port);
    if resident.is_some() {
        // Told where the repository is, so that the board this answers with can be opened.
        note_board(repo);
    }
    prefer(resident, dashboards_running(&repo.slug))
}

/// The board serving `repo`'s hub — its URL and whether the resident server is the one — or
/// `None`. Whoever started it, the token is the one every board on this machine shares.
fn located(repo: &crate::repo::RepoInfo) -> Option<(String, bool)> {
    let token = stored_token()?;
    match served(repo)? {
        Served::Resident(port) => Some((resident_board_url(port, &repo.slug, &token), true)),
        Served::Dedicated(port) => Some((board_url(port, &token), false)),
    }
}

/// `board` as `adj config` and `adjutant_config` report it: where it is, and whether the
/// resident server serves it. `null` when nothing does.
pub fn board_json(repo: &crate::repo::RepoInfo) -> Value {
    match located(repo) {
        Some((url, resident)) => json!({ "url": url, "resident": resident }),
        None => Value::Null,
    }
}

/// Where the resident server serves `repo`'s board, when a resident is live.
pub(super) fn resident_url(repo: &crate::repo::RepoInfo) -> Option<String> {
    let (_, port) = live_resident()?;
    let token = stored_token()?;
    note_board(repo);
    Some(resident_board_url(port, &repo.slug, &token))
}

// ── is anybody serving? ──────────────────────────────────────────────

fn record_path(slug: &str) -> PathBuf {
    messaging::state_dir()
        .join("dashboards")
        .join(format!("{slug}.json"))
}

fn live_record(slug: &str) -> Option<(u32, u16)> {
    live_at(&record_path(slug))
}

/// The pid and port `path` records, when the process is still the one that wrote them.
pub(super) fn live_at(path: &Path) -> Option<(u32, u16)> {
    let record: Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())?;
    let pid = record.get("pid").and_then(Value::as_u64)? as u32;
    let started = record.get("psStarted").and_then(Value::as_str);
    if messaging::ps_started(pid).as_deref() != started {
        return None;
    }
    let port = record
        .get("port")
        .and_then(Value::as_u64)
        .map(|p| p as u16)?;
    Some((pid, port))
}

/// The port a live dashboard is on for `repo`'s hub, the resident server's or the hub's own,
/// or `None`.
///
/// This is what `adj gate open` asks before it hands the ball over: a gate written with
/// nobody serving is a message into a directory no one opens, and an agent that waited on
/// one would wait for ever. Anchored on the recorded process start time like every other
/// record here, so a crashed server leaves a file that reads as absent rather than as a
/// dashboard that is about to answer.
pub fn running(repo: &crate::repo::RepoInfo) -> Option<u16> {
    match served(repo)? {
        Served::Resident(port) | Served::Dedicated(port) => Some(port),
    }
}

/// The port of a board of its own for `slug`, one started by a hub or by `adj serve`. The
/// resident server is not asked: this is what a hub's MCP server checks before it binds one.
pub fn dashboards_running(slug: &str) -> Option<u16> {
    live_record(slug).map(|(_, port)| port)
}

/// Record this board as the one serving the hub. Returns `Ok(true)` if it wrote the record,
/// and `Ok(false)` if another live board holds it and nothing was written.
/// A second board (for instance one started in a worktree to check a UI change) must not
/// take the record from a live board, because once it stops the record would name a dead
/// process and the live board would read as absent.
pub(super) fn record(slug: &str, port: u16) -> Result<bool, String> {
    if live_record(slug).is_some_and(|(pid, _)| pid != std::process::id()) {
        return Ok(false);
    }
    let path = record_path(slug);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let pid = std::process::id();
    let record = json!({ "pid": pid, "port": port, "psStarted": messaging::ps_started(pid) });
    std::fs::write(&path, format!("{record:#}\n"))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(true)
}

pub(super) fn boards_dir() -> PathBuf {
    messaging::state_dir().join("boards")
}

/// Tell the resident server where `repo` is, so that it can serve its board. Skipped when the
/// entry is already what it would write, and a failure is not one for the caller: the address
/// is only ever a convenience for a server that may not be running.
pub fn note_board(repo: &crate::repo::RepoInfo) {
    let entry = json!({ "main": repo.main, "nwo": repo.nwo, "hub": repo.hub });
    let path = boards_dir().join(format!("{}.json", repo.slug));
    if messaging::read_json(&path).as_ref() == Some(&entry) {
        return;
    }
    let _ = crate::infra::fs::write_json(&path, &entry);
}

/// Take `slug` out of the address book, so a closed hub is not offered a board any more.
pub(in crate::cmd) fn forget_board(slug: &str) -> Result<(), String> {
    let path = boards_dir().join(format!("{slug}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
    }
}

/// The pieces of the address book entry `slug` names, when they still describe that board:
/// the checkout is there and the repository and hub still come to the same slug.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Address {
    pub(super) slug: String,
    pub(super) main: String,
    pub(super) nwo: String,
    pub(super) hub: Option<String>,
}

pub(super) fn address_of(slug: &str) -> Option<Address> {
    let entry = messaging::read_json(&boards_dir().join(format!("{slug}.json")))?;
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    let address = Address {
        slug: slug.to_string(),
        main: text("main")?,
        nwo: text("nwo")?,
        hub: text("hub").filter(|hub| !hub.is_empty()),
    };
    (Path::new(&address.main).is_dir()
        && crate::repo::slug_for(&address.nwo, address.hub.as_deref()) == slug)
        .then_some(address)
}

/// Every board the address book names, by slug.
pub(super) fn addresses() -> Vec<Address> {
    let mut slugs: Vec<String> = std::fs::read_dir(boards_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .collect();
    slugs.sort();
    slugs.iter().filter_map(|slug| address_of(slug)).collect()
}
