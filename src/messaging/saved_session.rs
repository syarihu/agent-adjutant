use super::*;

// ── sessions: what `--resume` reopens ────────────────────────────────

/// The conversation a hub or a worker was started into, written down so it can be reopened
/// after the agent itself has gone — an update, a crash, a closed tab.
///
/// Kept apart from the presence records on purpose. Those say who is running *now*, and are
/// removed the moment nobody is: `hub-stop` clears the hub's, `close` the worker's, and a
/// takeover rewrites either. The session is wanted precisely after that has happened, so it
/// lives in a file nothing clears — a later start in the same place overwrites it, which is
/// the one thing that should.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSession {
    pub session_id: String,
    /// The hub identifier: which hub this was (for a hub), or which hub dispatched it (for a
    /// worker). `None` is the repository's own hub.
    pub hub: Option<String>,
    /// `owner/name`, for a hub. What lets `--resume` list a repository's resumable hubs when
    /// asked for one that has nothing saved.
    pub nwo: Option<String>,
    pub hub_name: Option<String>,
    /// The worker's tab title, so a resumed worker is named what it was named before.
    pub title: Option<String>,
    pub task: Option<String>,
    pub saved_at: Option<String>,
}

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

pub fn hub_session_path(slug: &str) -> PathBuf {
    state_dir().join("sessions").join(format!("{slug}.json"))
}

/// Carried on the hub's command line and inherited by the MCP server the agent starts:
/// `{slug}/{session id}`. It is what tells that server it belongs to a hub — the same server
/// runs under every session on the machine that has it registered — and which one.
pub const HUB_SESSION_ENV: &str = "ADJUTANT_HUB_SESSION";

/// Carried on the hub's command line like `HUB_SESSION_ENV`, and holding the hub's slug:
/// it is what tells the MCP server under the agent to serve that hub's board. A variable of
/// its own rather than the session one, because that one is left off for a runner that
/// records no session, and such a hub still wants its board. Left off when `hubServe` is
/// `false`.
pub const HUB_SERVE_ENV: &str = "ADJUTANT_HUB_SERVE";

/// The value of `HUB_SESSION_ENV` for this hub. Neither half can contain a `/`: a slug is
/// lowercase, digits and `-`, and a session id is whatever the runner was handed, which is
/// made up here.
pub fn hub_session_env(slug: &str, session_id: &str) -> String {
    format!("{slug}/{session_id}")
}

/// When a hub session was last known to be running, kept in a file of its own.
///
/// Not a field in the session file, because the two have different writers: the session is
/// written by `adj hub` as it starts a hub, this by the MCP server under the agent, every
/// minute and once more as it ends. Written into one file, the old hub's server finishing
/// its last write could put the old session back over the one a new hub has just saved. Here
/// the worst it can do is record that the old session was alive, which the reader discards
/// because the ids do not match.
pub fn hub_alive_path(slug: &str) -> PathBuf {
    state_dir().join("sessions").join(format!("{slug}.alive"))
}

/// Note that the hub session `session_id` is alive now.
pub fn touch_hub_session(slug: &str, session_id: &str) -> Result<(), String> {
    write_json(
        &hub_alive_path(slug),
        &json!({"sessionId": session_id, "lastAlive": now_secs()}),
    )
}

/// When the hub session saved for `slug` was last seen alive, in epoch seconds — or `None`
/// when nothing has said so about *that* session.
pub fn hub_last_alive(slug: &str, session_id: &str) -> Option<i64> {
    let record = read_json(&hub_alive_path(slug))?;
    (record.get("sessionId").and_then(Value::as_str) == Some(session_id))
        .then(|| record.get("lastAlive").and_then(Value::as_i64))
        .flatten()
}

/// Beside the worker record, for the reason the record is there: the worktree is the one key
/// both sides already have.
pub fn worker_session_path(worktree: &Path) -> PathBuf {
    worktree.join(".claude").join("adjutant-session.json")
}

pub fn save_hub_session(
    slug: &str,
    nwo: &str,
    hub: Option<&str>,
    hub_name: &str,
    session_id: &str,
) -> Result<PathBuf, String> {
    let path = hub_session_path(slug);
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
    let path = worker_session_path(worktree);
    let mut record = json!({
        "sessionId": session_id,
        "title": title,
        "savedAt": utc_stamp(now_secs()),
    });
    if let Some(hub) = said(hub)
        && let Some(fields) = record.as_object_mut()
    {
        fields.insert("hub".to_string(), json!(hub));
    }
    if let Some(task) = said(task)
        && let Some(fields) = record.as_object_mut()
    {
        fields.insert("task".to_string(), json!(task));
    }
    write_json(&path, &record)?;
    Ok(path)
}

/// Forget the hub session saved for `slug`, and when it was last alive.
///
/// For a hub started by a runner that records no session: what was saved belongs to a hub
/// before it, and leaving it would have the next `--resume` — or a plain `adj hub`, while
/// that older hub's last beat is still recent — reopen a conversation two hubs ago.
pub fn forget_hub_session(slug: &str) -> Result<(), String> {
    remove_if_present(&hub_session_path(slug))?;
    remove_if_present(&hub_alive_path(slug))
}

/// The same for a worktree, for a worker started by a runner that records no session.
pub fn forget_worker_session(worktree: &Path) -> Result<(), String> {
    remove_if_present(&worker_session_path(worktree))
}

pub fn hub_session(slug: &str) -> Option<SavedSession> {
    read_session(&hub_session_path(slug))
}

pub fn worker_session(worktree: &Path) -> Option<SavedSession> {
    read_session(&worker_session_path(worktree))
}

/// Every hub of `nwo` that has a session to reopen, the repository's own first.
pub fn hub_sessions_for(nwo: &str) -> Vec<SavedSession> {
    let Ok(entries) = std::fs::read_dir(state_dir().join("sessions")) else {
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

/// A saved session, or `None` for anything that does not name one. There is nothing to be
/// careful of here the way there is with a presence record: the worst a bad file can do is
/// leave `--resume` with nothing to reopen, and that is said in so many words.
pub(super) fn read_session(path: &Path) -> Option<SavedSession> {
    let record = read_json(path)?;
    let text = |key: &str| {
        record
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    Some(SavedSession {
        session_id: text("sessionId")?,
        hub: text("hub"),
        nwo: text("nwo"),
        hub_name: text("hubName"),
        title: text("title"),
        task: text("task"),
        saved_at: text("savedAt"),
    })
}
