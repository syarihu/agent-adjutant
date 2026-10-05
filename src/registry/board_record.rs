//! Which boards are running and where: the address book and the server record.

use super::store::{boards_dir, record_path, server_record_path};
use super::*;

pub(crate) fn served(root: &Path, repo: &crate::kernel::identity::RepoInfo) -> Option<Served> {
    let resident = live_resident(root).map(|(_, port)| port);
    if resident.is_some() {
        // Told where the repository is, so that the board this answers with can be opened.
        note_board(root, repo);
    }
    prefer(resident, dashboards_running(root, &repo.slug))
}

fn live_record(root: &Path, slug: &str) -> Option<(u32, u16)> {
    live_at(&record_path(root, slug))
}

/// The pid and port `path` records, when the process is still the one that wrote them.
pub(crate) fn live_at(path: &Path) -> Option<(u32, u16)> {
    let record: Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())?;
    let pid = record.get("pid").and_then(Value::as_u64)? as u32;
    let started = record.get("psStarted").and_then(Value::as_str);
    if ps_started(pid).as_deref() != started {
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
pub fn running(root: &Path, repo: &crate::kernel::identity::RepoInfo) -> Option<u16> {
    match served(root, repo)? {
        Served::Resident(port) | Served::Dedicated(port) => Some(port),
    }
}

/// The port of a board of its own for `slug`, one started by a hub or by `adj serve`. The
/// resident server is not asked: this is what a hub's MCP server checks before it binds one.
pub fn dashboards_running(root: &Path, slug: &str) -> Option<u16> {
    live_record(root, slug).map(|(_, port)| port)
}

/// Record this board as the one serving the hub. Returns `Ok(true)` if it wrote the record,
/// and `Ok(false)` if another live board holds it and nothing was written.
/// A second board (for instance one started in a worktree to check a UI change) must not
/// take the record from a live board, because once it stops the record would name a dead
/// process and the live board would read as absent.
pub(crate) fn record(root: &Path, slug: &str, port: u16) -> Result<bool, String> {
    if live_record(root, slug).is_some_and(|(pid, _)| pid != std::process::id()) {
        return Ok(false);
    }
    let path = record_path(root, slug);
    let pid = std::process::id();
    let record = json!({ "pid": pid, "port": port, "psStarted": ps_started(pid) });
    crate::infra::fs::write_json(&path, &record)?;
    Ok(true)
}

/// Tell the resident server where `repo` is, so that it can serve its board. Skipped when the
/// entry is already what it would write, and a failure is not one for the caller: the address
/// is only ever a convenience for a server that may not be running.
pub fn note_board(root: &Path, repo: &crate::kernel::identity::RepoInfo) {
    let entry = json!({ "main": repo.main, "nwo": repo.nwo, "hub": repo.hub });
    let path = boards_dir(root).join(format!("{}.json", repo.slug));
    if crate::infra::fs::read_json(&path).as_ref() == Some(&entry) {
        return;
    }
    let _ = crate::infra::fs::write_json(&path, &entry);
}

/// Take `slug` out of the address book, so a closed hub is not offered a board any more.
pub(crate) fn forget_board(root: &Path, slug: &str) -> Result<(), String> {
    let path = boards_dir(root).join(format!("{slug}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
    }
}

pub(crate) fn address_of(root: &Path, slug: &str) -> Option<Address> {
    let entry = crate::infra::fs::read_json(&boards_dir(root).join(format!("{slug}.json")))?;
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    let address = Address {
        slug: slug.to_string(),
        main: text("main")?,
        nwo: text("nwo")?,
        hub: text("hub").filter(|hub| !hub.is_empty()),
    };
    (Path::new(&address.main).is_dir()
        && crate::kernel::identity::slug_for(&address.nwo, address.hub.as_deref()) == slug)
        .then_some(address)
}

/// Every board the address book names, by slug.
pub(crate) fn addresses(root: &Path) -> Vec<Address> {
    let mut slugs: Vec<String> = std::fs::read_dir(boards_dir(root))
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
    slugs
        .iter()
        .filter_map(|slug| address_of(root, slug))
        .collect()
}

/// The pid and port of the resident server, when one is running. Anchored on the recorded
/// process start time like every other record here, so a killed server leaves a file that
/// reads as absent.
pub(crate) fn live_resident(root: &Path) -> Option<(u32, u16)> {
    live_at(&server_record_path(root))
}

/// Whether a resident server is running.
pub fn resident_running(root: &Path) -> bool {
    live_resident(root).is_some()
}

/// The version `server.json` names, if any.
pub(crate) fn recorded_version(root: &Path) -> Option<String> {
    crate::infra::fs::read_json(&server_record_path(root))?
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Write `server.json` for this process, which has bound `port`.
pub(crate) fn record_server(root: &Path, port: u16) -> Result<(), String> {
    let pid = std::process::id();
    let record = json!({
        "pid": pid,
        "psStarted": ps_started(pid),
        "port": port,
        "startedAt": crate::infra::clock::utc_stamp(crate::infra::clock::now_secs()),
        "version": env!("CARGO_PKG_VERSION"),
    });
    crate::infra::fs::write_json(&server_record_path(root), &record)
}

/// What `server.json` names, whether or not that process is still there.
pub(crate) fn recorded_server(root: &Path) -> Option<(u32, Option<String>)> {
    let record = crate::infra::fs::read_json(&server_record_path(root))?;
    let pid = record.get("pid").and_then(Value::as_u64)? as u32;
    let started = record
        .get("psStarted")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((pid, started))
}

/// Whether `record` still names the process `pid` started at `started`. The rule for removing
/// `server.json`: a supervisor that restarts the server may already have written the next
/// record, and that one is not ours to remove.
pub(crate) fn names_resident(
    record: Option<&(u32, Option<String>)>,
    pid: u32,
    started: Option<&str>,
) -> bool {
    record.is_some_and(|(p, s)| *p == pid && s.as_deref() == started)
}

pub(crate) fn forget_server(root: &Path, pid: u32, started: Option<&str>) {
    if names_resident(recorded_server(root).as_ref(), pid, started) {
        let _ = std::fs::remove_file(server_record_path(root));
    }
}
