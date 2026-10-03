use super::*;

/// Where the inbox, the records and the boards live. A constant for the same reason
/// `config::CONFIG_ENV` is one: it has to be forwarded by name into the tabs this opens.
pub const STATE_DIR_ENV: &str = "ADJUTANT_STATE_DIR";

/// `ADJUTANT_STATE_DIR` wins, then `$XDG_STATE_HOME/adjutant`, then `~/.local/state/adjutant`.
pub fn state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var(STATE_DIR_ENV)
        && !dir.is_empty()
    {
        return expand_home(&dir);
    }
    match std::env::var("XDG_STATE_HOME") {
        Ok(dir) if !dir.is_empty() => expand_home(&dir).join("adjutant"),
        _ => home_dir().join(".local").join("state").join("adjutant"),
    }
}

pub fn hub_record_path(slug: &str) -> PathBuf {
    state_dir().join("hubs").join(format!("{slug}.json"))
}

/// Where messages for this hub wait. One directory, whether or not the hub is running: two
/// would mean the hub has to remember to read both, and the one it forgets is the one that
/// silently swallows reports.
pub fn inbox_dir(slug: &str) -> PathBuf {
    state_dir().join("inbox").join(slug)
}

/// Where a message goes once the hub has acted on it. Kept rather than deleted so a report
/// that was mishandled can still be found.
pub fn archive_dir(slug: &str) -> PathBuf {
    inbox_dir(slug).join("read")
}
