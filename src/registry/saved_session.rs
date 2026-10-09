//! The session a hub or a worker was started into, kept so it can be reopened.

use super::store::{hub_alive_path, hub_session_path, read_session, worker_session_path};
use super::*;
use serde_json::Map;

/// A fresh session id: a random (version 4) UUID, which is the shape Claude Code's
/// `--session-id` insists on and every other agent can take as an opaque string.
///
/// From `/dev/urandom` rather than a crate: sixteen bytes do not justify a dependency, and
/// this only runs on the Unix systems the rest of this tool already assumes.
pub fn new_session_id() -> Result<String, String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("cannot read /dev/urandom for a session id: {e}"))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// The value of `HUB_SESSION_ENV` for this hub. Neither half can contain a `/`: a slug is
/// lowercase, digits and `-`, and a session id is whatever the runner was handed, which is
/// made up here.
pub fn hub_session_env(slug: &str, session_id: &str) -> String {
    format!("{slug}/{session_id}")
}

/// Note that the hub session `session_id` is alive now.
pub fn touch_hub_session(root: &Path, slug: &str, session_id: &str) -> Result<(), String> {
    write_json(
        &hub_alive_path(root, slug),
        &json!({"sessionId": session_id, "lastAlive": now_secs()}),
    )
}

/// When the hub session saved for `slug` was last seen alive, in epoch seconds — or `None`
/// when nothing has said so about *that* session.
pub fn hub_last_alive(root: &Path, slug: &str, session_id: &str) -> Option<i64> {
    let record = read_json(&hub_alive_path(root, slug))?;
    (record.get("sessionId").and_then(Value::as_str) == Some(session_id))
        .then(|| record.get("lastAlive").and_then(Value::as_i64))
        .flatten()
}

pub fn save_hub_session(
    root: &Path,
    slug: &str,
    nwo: &str,
    hub: Option<&str>,
    hub_name: &str,
    session_id: &str,
) -> Result<PathBuf, String> {
    let path = hub_session_path(root, slug);
    let mut record = json!({
        "sessionId": session_id,
        "nwo": nwo,
        "hubName": hub_name,
        "savedAt": utc_stamp(now_secs()),
    });
    if let Some(hub) = said(hub)
        && let Some(fields) = record.as_object_mut()
    {
        fields.insert("hub".to_string(), json!(hub));
    }
    write_json(&path, &record)?;
    Ok(path)
}

pub fn save_worker_session(
    worktree: &Path,
    title: &str,
    hub: Option<&str>,
    task: Option<&str>,
    session_id: &str,
) -> Result<PathBuf, String> {
    write_worker_session(worktree, Map::new(), session_id, title, hub, task)
}

/// Save `saved` again with a new title, hub and task, keeping the keys it carries that this
/// version does not know. For a session that already exists: a fresh one starts from nothing
/// and uses `save_worker_session`.
pub fn rewrite_worker_session(
    worktree: &Path,
    saved: &SavedSession,
    title: &str,
    hub: Option<&str>,
    task: Option<&str>,
) -> Result<PathBuf, String> {
    write_worker_session(
        worktree,
        saved.other.clone(),
        &saved.session_id,
        title,
        hub,
        task,
    )
}

/// `fields` is what a newer version wrote that this one does not read, put back under the
/// keys written here.
fn write_worker_session(
    worktree: &Path,
    mut fields: Map<String, Value>,
    session_id: &str,
    title: &str,
    hub: Option<&str>,
    task: Option<&str>,
) -> Result<PathBuf, String> {
    let path = worker_session_path(worktree);
    fields.insert("sessionId".to_string(), json!(session_id));
    fields.insert("title".to_string(), json!(title));
    fields.insert("savedAt".to_string(), json!(utc_stamp(now_secs())));
    match said(hub) {
        Some(hub) => fields.insert("hub".to_string(), json!(hub)),
        None => fields.remove("hub"),
    };
    match said(task) {
        Some(task) => fields.insert("task".to_string(), json!(task)),
        None => fields.remove("task"),
    };
    write_json(&path, &Value::Object(fields))?;
    Ok(path)
}

/// Forget the hub session saved for `slug`, and when it was last alive.
///
/// For a hub started by a runner that records no session: what was saved belongs to a hub
/// before it, and leaving it would have the next `--resume` — or a plain `adj hub`, while
/// that older hub's last beat is still recent — reopen a conversation two hubs ago.
pub fn forget_hub_session(root: &Path, slug: &str) -> Result<(), String> {
    remove_if_present(&hub_session_path(root, slug))?;
    remove_if_present(&hub_alive_path(root, slug))
}

/// The same for a worktree, for a worker started by a runner that records no session.
pub fn forget_worker_session(worktree: &Path) -> Result<(), String> {
    remove_if_present(&worker_session_path(worktree))
}

pub fn hub_session(root: &Path, slug: &str) -> Option<SavedSession> {
    read_session(&hub_session_path(root, slug))
}

pub fn worker_session(worktree: &Path) -> Option<SavedSession> {
    read_session(&worker_session_path(worktree))
}

/// Every hub of `nwo` that has a session to reopen, the repository's own first.
pub fn hub_sessions_for(root: &Path, nwo: &str) -> Vec<SavedSession> {
    let Ok(entries) = std::fs::read_dir(root.join("sessions")) else {
        return Vec::new();
    };
    let mut found: Vec<SavedSession> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| read_session(&entry.path()))
        .filter(|session| session.nwo.as_deref() == Some(nwo))
        .collect();
    found.sort_by(|a, b| (a.hub.is_some(), &a.hub).cmp(&(b.hub.is_some(), &b.hub)));
    found
}
