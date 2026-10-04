use super::*;
use std::path::{Path, PathBuf};

/// `ADJUTANT_CONFIG` wins, then `$XDG_CONFIG_HOME/adjutant/config.json`, then
/// `~/.config/adjutant/config.json`. Not under a specific agent's config directory: the
/// point of this tool is that the same config serves whichever agent is driving.
pub fn config_path() -> PathBuf {
    if let Ok(path) = std::env::var(CONFIG_ENV)
        && !path.is_empty()
    {
        return expand_home(&path);
    }
    config_home().join("adjutant").join("config.json")
}

fn config_home() -> PathBuf {
    match std::env::var(XDG_CONFIG_HOME_ENV) {
        Ok(dir) if !dir.is_empty() => expand_home(&dir),
        _ => home_dir().join(".config"),
    }
}

/// `value` made absolute against `cwd` when it is a relative path, and `None` when it has to
/// be left exactly as it is: empty, already absolute, or starting with `~` the way
/// `expand_home` understands it (`~` or `~/...`), which is anchored to `HOME` rather than to
/// any directory.
pub(super) fn anchored(value: &str, cwd: &Path) -> Option<PathBuf> {
    if value.is_empty() || value == "~" || value.starts_with("~/") {
        return None;
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return None;
    }
    std::path::absolute(cwd.join(path)).ok()
}

/// Make a relative `ADJUTANT_CONFIG` and `XDG_CONFIG_HOME` absolute, once, against the
/// directory this process was started in.
///
/// A relative value is otherwise resolved against whichever directory each process stands in,
/// and every process adjutant starts stands somewhere else — the hub moves to the main
/// checkout before it execs, and tabs open at the checkout or the worktree. Rewriting the
/// variable, rather than resolving it where it is read, is what makes the answer travel: exec
/// and spawned children inherit it, and `forwarded_env` hands the same value to a tab.
/// `ADJUTANT_STATE_DIR` is deliberately not included: `registry::state_root` takes a relative
/// one against the repository's main checkout, not against where the process was started.
pub fn anchor_config_env() {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    for name in [CONFIG_ENV, XDG_CONFIG_HOME_ENV] {
        let Ok(value) = std::env::var(name) else {
            continue;
        };
        if let Some(path) = anchored(&value, &cwd) {
            // SAFETY: called first thing in `run`, before any thread is started.
            unsafe { std::env::set_var(name, path) };
        }
    }
}
