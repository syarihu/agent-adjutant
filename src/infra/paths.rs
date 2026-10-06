use crate::infra::env::STATE_DIR_ENV;
use std::path::PathBuf;

/// `ADJUTANT_STATE_DIR` wins, then `$XDG_STATE_HOME/adjutant`, then `~/.local/state/adjutant`.
pub fn state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var(STATE_DIR_ENV)
        && !dir.is_empty()
    {
        return expand_home(&dir);
    }
    default_state_dir()
}

/// What `state_dir` answers when `ADJUTANT_STATE_DIR` is not set. Relative only when
/// `XDG_STATE_HOME` is, so a caller that needs an absolute path makes it one
/// (`registry::hook_state_root` does, with `std::path::absolute`).
pub fn default_state_dir() -> PathBuf {
    match std::env::var("XDG_STATE_HOME") {
        Ok(dir) if !dir.is_empty() => expand_home(&dir).join("adjutant"),
        _ => home_dir().join(".local").join("state").join("adjutant"),
    }
}

/// Where `~` points, and the anchor under which every path this program uses is derived.
///
/// An unset `HOME` used to make that anchor the empty string, which left the state
/// directory *relative*: it landed under whatever directory the process happened to start
/// in, so a hub and a worker started from different places read different inboxes — and
/// neither is wrong about anything it can see, which is why nobody would find it. An
/// absolute fallback keeps the two agreeing, and saying so on stderr is the only way the
/// person running them learns that `HOME` is missing.
pub fn home_dir() -> PathBuf {
    home_from(std::env::var("HOME").ok().as_deref())
}

/// Split out from `home_dir` so the answer can be checked without a test reaching into the
/// environment every other test is reading.
pub(super) fn home_from(home: Option<&str>) -> PathBuf {
    match home {
        Some(home) if home.starts_with('/') => PathBuf::from(home),
        _ => {
            // Per person, and absolute whatever `TMPDIR` says. `temp_dir()` is one shared
            // directory on most Linux systems and follows a `TMPDIR` that may itself be
            // relative — so a bare name under it would put two people's configs and
            // inboxes in one place, and a relative `TMPDIR` would undo the very thing this
            // fallback is for.
            let temp = std::env::temp_dir();
            let temp = match temp.is_absolute() {
                true => temp,
                false => PathBuf::from("/tmp"),
            };
            let whose = ["USER", "LOGNAME"]
                .iter()
                .find_map(|key| std::env::var(key).ok())
                .filter(|name| !name.is_empty() && !name.contains('/'))
                .unwrap_or_else(|| "unknown".to_string());
            let fallback = temp.join(format!("adjutant-no-home-{whose}"));
            static SAID: std::sync::Once = std::sync::Once::new();
            SAID.call_once(|| {
                eprintln!(
                    "adjutant: HOME is not set to an absolute path, falling back to {} — set HOME so every session agrees on one location",
                    fallback.display()
                );
            });
            fallback
        }
    }
}

pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home_dir().join(rest),
        None if path == "~" => home_dir(),
        None => PathBuf::from(path),
    }
}

/// Whether two paths name one directory, for a task's stored worktree against the one git lists.
pub fn same_path(a: &str, b: &str) -> bool {
    let resolved = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| p.into());
    resolved(a) == resolved(b)
}

/// This binary, for commands that have to name themselves in a command line handed to a
/// terminal. The absolute path rather than `adjutant`, so a new tab whose PATH is not yet
/// loaded still finds it.
pub fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "adjutant".to_string())
}

/// `exe_path` for a caller that writes the path into a file a later process runs, where the
/// fallback `adjutant` (found through a `PATH` the hook may not have) would be a silent
/// no-op. The reason is in the error because it is what the person is told.
pub fn exe_path_absolute() -> Result<String, String> {
    let path = std::env::current_exe().map_err(|e| format!("cannot find this binary: {e}"))?;
    if !path.is_absolute() {
        return Err(format!(
            "this binary's path is not absolute: {}",
            path.display()
        ));
    }
    path.into_os_string()
        .into_string()
        .map_err(|path| format!("this binary's path is not valid UTF-8: {path:?}"))
}

#[cfg(test)]
mod tests;
