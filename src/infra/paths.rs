use crate::infra::env::STATE_DIR_ENV;
use std::path::PathBuf;

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

/// `state_dir()` made absolute against the working directory — the directory this process
/// reads and writes now.
pub fn state_dir_absolute() -> PathBuf {
    let dir = state_dir();
    std::path::absolute(&dir).unwrap_or(dir)
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

#[cfg(test)]
mod tests;
