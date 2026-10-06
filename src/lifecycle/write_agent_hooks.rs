//! The hook settings file a Claude Code session is started with.

use std::path::{Path, PathBuf};

use crate::infra::fs::{read_json, write_json};
use crate::infra::paths::exe_path_absolute;
use crate::kernel::agent_hooks::{binary_of, claude_hook_command, claude_settings};
use crate::kernel::identity::short_digest;
use crate::kernel::runner::wants_settings;

/// What a launch came to about `{settings}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hooks {
    /// The runner passes its own `--settings` or is not Claude Code: nothing to inject.
    NotWanted,
    /// The file passed with `--settings`.
    Injected(PathBuf),
    /// Wanted, but not possible; the reason, for the caller to say. The agent starts without
    /// hooks: a missing file must not stop a worker that would otherwise run.
    Skipped(String),
}

impl Hooks {
    /// The file to hand the runner, if any.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Hooks::Injected(path) => Some(path),
            _ => None,
        }
    }
}

/// The hook settings for a launch with `template`, written under `state` when the template
/// wants them. Written on a dry run too: the file is idempotent and is not a record.
pub fn agent_hooks_for(state: &Path, template: &str) -> Hooks {
    if !wants_settings(template) {
        return Hooks::NotWanted;
    }
    match exe_path_absolute().and_then(|exe| write_agent_hooks(state, &exe)) {
        Ok(path) => Hooks::Injected(path),
        Err(why) => Hooks::Skipped(why),
    }
}

/// Write `agent-hooks/claude-<digest of exe>.json` under `state`, and give its path.
///
/// Named after the binary it calls, so a development build and an installed one sharing the
/// state dir do not keep rewriting one file under each other's running sessions. Written only
/// when missing or different, through a temporary file and a rename, so a session starting
/// now never reads a half-written file and an unchanged one keeps its modification time.
pub fn write_agent_hooks(state: &Path, exe: &str) -> Result<PathBuf, String> {
    if !Path::new(exe).is_absolute() {
        return Err(format!("the binary's path is not absolute: {exe}"));
    }
    let dir = state.join("agent-hooks");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = dir.join(format!("claude-{}.json", short_digest(exe)));
    let wanted = claude_settings(&claude_hook_command(exe));
    if read_json(&path).as_ref() != Some(&wanted) {
        write_json(&path, &wanted)?;
    }
    sweep(&dir, &path);
    Ok(path)
}

/// Remove the files of binaries that are gone (a build that was deleted, an install that
/// moved), so they do not pile up. Best effort: a file that cannot be read or removed stays,
/// and so does anything not named like one of these, such as the hub's own, which is not
/// written yet.
fn sweep(dir: &Path, kept: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == kept || !is_hooks_file(&path) {
            continue;
        }
        let Some(exe) = read_json(&path).as_ref().and_then(binary_of) else {
            continue;
        };
        if !Path::new(&exe).exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// `claude-` and eight lowercase hex digits, `.json`: exactly what `write_agent_hooks` names.
fn is_hooks_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.strip_prefix("claude-")
        .and_then(|rest| rest.strip_suffix(".json"))
        .is_some_and(|digest| {
            digest.len() == 8 && digest.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
        })
}

#[cfg(test)]
mod tests;
