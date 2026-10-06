//! Where the records live on disk, and the raw reads and writes behind the typed ones.

use super::*;

pub fn hub_record_path(root: &Path, slug: &str) -> PathBuf {
    root.join("hubs").join(format!("{slug}.json"))
}

// ── the other direction: a worker in a worktree ──────────────────────

/// A worker's record and its messages both live in the worktree, not in the state
/// directory. The hub already knows the worktree path — it created it — so there is no key
/// to derive and no way for the two sides to disagree about one.
pub fn worker_record_path(worktree: &Path) -> PathBuf {
    worktree.join(".claude").join("adjutant-worker.json")
}

/// Written by `adj work` just before it opens the tab, removed when the worker registers.
pub fn starting_marker_path(worktree: &Path) -> PathBuf {
    worktree
        .join(".claude")
        .join("adjutant-worker-starting.json")
}

/// Where the marker saying `worktree` is being removed is kept: in the main checkout, outside
/// the worktree that is about to go.
///
/// The worktree is resolved through its parent directory, so the answer is the same while the
/// directory exists and after it is gone; resolving the path itself would give a symlinked
/// `/tmp` or `/var` one spelling before and another after.
pub(super) fn removing_marker_path(main: &Path, worktree: &Path) -> PathBuf {
    let resolved = match (worktree.parent(), worktree.file_name()) {
        (Some(parent), Some(leaf)) => parent
            .canonicalize()
            .map(|parent| parent.join(leaf))
            .unwrap_or_else(|_| worktree.to_path_buf()),
        _ => worktree.to_path_buf(),
    };
    let name: String = resolved
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    main.join(".claude")
        .join("adjutant-removing")
        .join(format!("{name}.json"))
}

pub fn hub_session_path(root: &Path, slug: &str) -> PathBuf {
    root.join("sessions").join(format!("{slug}.json"))
}

/// When a hub session was last known to be running, kept in a file of its own.
///
/// Not a field in the session file, because the two have different writers: the session is
/// written by `adj hub` as it starts a hub, this by the MCP server under the agent, every
/// minute and once more as it ends. Written into one file, the old hub's server finishing
/// its last write could put the old session back over the one a new hub has just saved. Here
/// the worst it can do is record that the old session was alive, which the reader discards
/// because the ids do not match.
pub fn hub_alive_path(root: &Path, slug: &str) -> PathBuf {
    root.join("sessions").join(format!("{slug}.alive"))
}

/// Beside the worker record, for the reason the record is there: the worktree is the one key
/// both sides already have.
pub fn worker_session_path(worktree: &Path) -> PathBuf {
    worktree.join(".claude").join("adjutant-session.json")
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

// ── is anybody serving? ──────────────────────────────────────────────

pub(super) fn record_path(root: &Path, slug: &str) -> PathBuf {
    root.join("dashboards").join(format!("{slug}.json"))
}

pub(crate) fn boards_dir(root: &Path) -> PathBuf {
    root.join("boards")
}

pub(super) fn server_record_path(root: &Path) -> PathBuf {
    root.join("server.json")
}

/// Read first, and only when that fails ask whether anything is there, so a record removed
/// in between reads as absent.
pub(super) fn read_record<T>(path: &Path, parse: impl FnOnce(Value) -> Option<T>) -> Recorded<T> {
    if let Some(value) = read_json(path) {
        return parse(value).map_or(Recorded::Unreadable, Recorded::Found);
    }
    match record_exists(path) {
        Ok(false) => Recorded::Absent,
        Ok(true) | Err(_) => Recorded::Unreadable,
    }
}

pub(super) fn write_worker_record(worktree: &Path, record: &WorkerRecord) -> Result<(), String> {
    write_json(&worker_record_path(worktree), &record.to_value())
}

/// A record that is there and cannot be rewritten, which is not the same news as no record:
/// the worker is registered and its file is damaged.
pub(super) fn unreadable_worker_record(worktree: &Path) -> String {
    format!(
        "cannot read the worker record at {}: it is not the JSON this tool writes",
        worker_record_path(worktree).display()
    )
}

// ── what each agent session is doing ─────────────────────────────────

pub(super) fn agent_sessions_dir(root: &Path) -> PathBuf {
    root.join("agent-sessions")
}

pub(super) fn agent_session_path(root: &Path, id: &str) -> PathBuf {
    agent_sessions_dir(root).join(format!("{id}.json"))
}

pub(super) fn agent_session_lock_path(root: &Path, id: &str) -> PathBuf {
    agent_sessions_dir(root).join(format!("{id}.lock"))
}

/// Where a row that cannot be read is moved to, so a new one can be started.
pub(super) fn agent_session_broken_path(root: &Path, id: &str) -> PathBuf {
    agent_sessions_dir(root).join(format!("{id}.json.broken"))
}

/// A row, with the session id filled in from the file name when the file does not say.
pub(super) fn read_agent_session(path: &Path) -> Recorded<AgentSession> {
    match read_record(path, |value| {
        serde_json::from_value::<AgentSession>(value).ok()
    }) {
        Recorded::Found(mut row) => {
            if row.session_id.is_empty() {
                row.session_id = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or_default();
            }
            Recorded::Found(row)
        }
        Recorded::Absent => Recorded::Absent,
        Recorded::Unreadable => Recorded::Unreadable,
    }
}

pub(super) fn write_agent_session(root: &Path, row: &AgentSession) -> Result<(), String> {
    if !valid_session_id(&row.session_id) {
        return Err(format!("not a usable session id: {:?}", row.session_id));
    }
    write_json_private(&agent_session_path(root, &row.session_id), row)
}
