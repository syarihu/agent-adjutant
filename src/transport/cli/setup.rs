//! `adj setup <agent>`: put adjutant's hooks in the user's Claude Code settings or Codex hooks
//! file, or take them out.

use serde_json::Value;
use std::path::{Path, PathBuf};

use super::args::SetupArgs;
use crate::infra::env::{CLAUDE_CONFIG_DIR_ENV, CODEX_HOME_ENV};
use crate::infra::fs::replace_json;
use crate::infra::paths::{exe_path_absolute, home_dir};
use crate::kernel::agent_hooks::{
    GlobalHookAgent, add_global_hooks, count_global_hooks, global_hook_command, remove_global_hooks,
};

#[derive(Debug, PartialEq)]
enum Outcome {
    Added,
    Updated,
    Unchanged,
    Removed(usize),
    NothingToRemove,
}

pub fn setup(args: &SetupArgs) -> Result<(), String> {
    let agent = match args.agent.as_str() {
        "claude" => GlobalHookAgent::Claude,
        "codex" => GlobalHookAgent::Codex,
        other => return Err(format!("unknown agent {other:?}")),
    };
    let home = home_dir();
    let path = match agent {
        GlobalHookAgent::Claude => {
            settings_path(std::env::var(CLAUDE_CONFIG_DIR_ENV).ok().as_deref(), &home)
        }
        GlobalHookAgent::Codex => {
            codex_hooks_path(std::env::var(CODEX_HOME_ENV).ok().as_deref(), &home)
        }
    };
    let shown = path.display();
    let name = agent.name();
    if args.remove {
        match setup_at(&path, None, agent)? {
            Outcome::Removed(n) => println!("Removed {n} adjutant hook entries from {shown}."),
            _ => println!("No adjutant hooks in {shown}; nothing to remove."),
        }
        return Ok(());
    }
    // Before anything is read or written: without an absolute path there is nothing to put in
    // the file that a hook could run.
    let exe = exe_path_absolute()?;
    let events = agent.events().len();
    match (setup_at(&path, Some(&exe), agent)?, agent) {
        (Outcome::Added, GlobalHookAgent::Claude) => println!(
            "Added adjutant's hooks to {shown}: {events} events run {exe} hook {name} --global. \
             Claude Code sessions started from now on report to `adj agent-sessions`."
        ),
        (Outcome::Added, GlobalHookAgent::Codex) => println!(
            "Added adjutant's hooks to {shown}: {events} events run {exe} hook {name} --global. \
             Codex sessions started from now on report to `adj agent-sessions`.\n{CODEX_TRUST}"
        ),
        (Outcome::Updated, GlobalHookAgent::Codex) => {
            println!("Updated adjutant's hooks in {shown} to run {exe}.\n{CODEX_TRUST}")
        }
        (Outcome::Updated, _) => println!("Updated adjutant's hooks in {shown} to run {exe}."),
        _ => println!("adjutant's hooks in {shown} already run {exe}; nothing to change."),
    }
    Ok(())
}

/// Codex runs a hook only once the user has trusted its command, and keeps that decision in its
/// own config; adjutant never writes it, so it has to be said.
const CODEX_TRUST: &str = "Codex asks you to trust each new hook the next time it starts. \
    After moving adj, run `adj setup codex` again and trust the hooks again.";

/// The same rule as the hook receiver's: the variable when it says something, else the default
/// account's directory.
fn settings_path(config_dir: Option<&str>, home: &Path) -> PathBuf {
    match config_dir.filter(|dir| !dir.trim().is_empty()) {
        Some(dir) => Path::new(dir).join("settings.json"),
        None => home.join(".claude").join("settings.json"),
    }
}

/// Codex's global hooks file: under `CODEX_HOME` when it says something, else `~/.codex`.
fn codex_hooks_path(codex_home: Option<&str>, home: &Path) -> PathBuf {
    match codex_home.filter(|dir| !dir.trim().is_empty()) {
        Some(dir) => Path::new(dir).join("hooks.json"),
        None => home.join(".codex").join("hooks.json"),
    }
}

/// Add adjutant's hooks to the settings file at `path` (`exe` is the binary they run), or, with
/// no `exe`, take them out. Nothing is written when there is nothing to change, and nothing when
/// the file cannot be read as the settings it should be.
fn setup_at(path: &Path, exe: Option<&str>, agent: GlobalHookAgent) -> Result<Outcome, String> {
    // A settings file is often a link into a dotfiles repository; the rename must replace what
    // it points at, not the link.
    let path = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(path)
            .map_err(|e| format!("cannot follow {}: {e}", path.display()))?,
        _ => path.to_path_buf(),
    };
    let path = path.as_path();
    let old = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Value>(&text).map_err(|e| {
            format!(
                "{} is not valid JSON ({e}); nothing was written",
                path.display()
            )
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match exe {
            Some(_) => serde_json::json!({}),
            None => return Ok(Outcome::NothingToRemove),
        },
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let refused = |e: String| format!("{}: {e}; nothing was written", path.display());
    match exe {
        Some(exe) => {
            let new =
                add_global_hooks(&old, &global_hook_command(exe, agent), agent).map_err(refused)?;
            if new == old {
                return Ok(Outcome::Unchanged);
            }
            replace_json(path, &new)?;
            Ok(match count_global_hooks(&old, agent) {
                0 => Outcome::Added,
                _ => Outcome::Updated,
            })
        }
        None => {
            let (new, removed) = remove_global_hooks(&old, agent).map_err(refused)?;
            if removed == 0 {
                return Ok(Outcome::NothingToRemove);
            }
            replace_json(path, &new)?;
            Ok(Outcome::Removed(removed))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_dir_wins_when_it_says_something() {
        let home = Path::new("/home/a");
        assert_eq!(
            settings_path(Some("/cfg"), home),
            Path::new("/cfg/settings.json")
        );
        for blank in [None, Some(""), Some("  ")] {
            assert_eq!(
                settings_path(blank, home),
                Path::new("/home/a/.claude/settings.json")
            );
        }
    }

    #[test]
    fn a_missing_file_is_made_for_add_and_left_missing_for_remove() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("settings.json");
        assert_eq!(
            setup_at(&path, None, GlobalHookAgent::Claude).unwrap(),
            Outcome::NothingToRemove
        );
        assert!(!path.parent().unwrap().exists());
        assert_eq!(
            setup_at(&path, Some("/bin/adj"), GlobalHookAgent::Claude).unwrap(),
            Outcome::Added
        );
        assert_eq!(
            setup_at(&path, Some("/bin/adj"), GlobalHookAgent::Claude).unwrap(),
            Outcome::Unchanged
        );
        assert_eq!(
            setup_at(&path, Some("/b/adj"), GlobalHookAgent::Claude).unwrap(),
            Outcome::Updated
        );
        assert_eq!(
            setup_at(&path, None, GlobalHookAgent::Claude).unwrap(),
            Outcome::Removed(GlobalHookAgent::Claude.events().len())
        );
    }

    #[test]
    fn codex_hooks_live_under_codex_home_or_the_default_directory() {
        let home = Path::new("/home/a");
        assert_eq!(
            codex_hooks_path(Some("/cx"), home),
            Path::new("/cx/hooks.json")
        );
        for blank in [None, Some(""), Some("  ")] {
            assert_eq!(
                codex_hooks_path(blank, home),
                Path::new("/home/a/.codex/hooks.json")
            );
        }
    }
}
