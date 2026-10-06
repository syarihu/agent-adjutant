//! `adj setup claude`: put adjutant's hooks in the user's Claude Code settings, or take them out.

use serde_json::Value;
use std::path::{Path, PathBuf};

use super::args::SetupArgs;
use crate::infra::env::CLAUDE_CONFIG_DIR_ENV;
use crate::infra::fs::replace_json;
use crate::infra::paths::{exe_path_absolute, home_dir};
use crate::kernel::agent_hooks::{
    CLAUDE_EVENTS, add_global_hooks, claude_global_hook_command, count_global_hooks,
    remove_global_hooks,
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
    let config_dir = std::env::var(CLAUDE_CONFIG_DIR_ENV).ok();
    let path = settings_path(config_dir.as_deref(), &home_dir());
    let shown = path.display();
    if args.remove {
        match setup_at(&path, None)? {
            Outcome::Removed(n) => println!("Removed {n} adjutant hook entries from {shown}."),
            _ => println!("No adjutant hooks in {shown}; nothing to remove."),
        }
        return Ok(());
    }
    // Before anything is read or written: without an absolute path there is nothing to put in
    // the file that a hook could run.
    let exe = exe_path_absolute()?;
    match setup_at(&path, Some(&exe))? {
        Outcome::Added => println!(
            "Added adjutant's hooks to {shown}: {} events run {exe} hook claude --global. \
             Claude Code sessions started from now on report to `adj agent-sessions`.",
            CLAUDE_EVENTS.len()
        ),
        Outcome::Updated => println!("Updated adjutant's hooks in {shown} to run {exe}."),
        _ => println!("adjutant's hooks in {shown} already run {exe}; nothing to change."),
    }
    Ok(())
}

/// The same rule as the hook receiver's: the variable when it says something, else the default
/// account's directory.
fn settings_path(config_dir: Option<&str>, home: &Path) -> PathBuf {
    match config_dir.filter(|dir| !dir.trim().is_empty()) {
        Some(dir) => Path::new(dir).join("settings.json"),
        None => home.join(".claude").join("settings.json"),
    }
}

/// Add adjutant's hooks to the settings file at `path` (`exe` is the binary they run), or, with
/// no `exe`, take them out. Nothing is written when there is nothing to change, and nothing when
/// the file cannot be read as the settings it should be.
fn setup_at(path: &Path, exe: Option<&str>) -> Result<Outcome, String> {
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
            let new = add_global_hooks(&old, &claude_global_hook_command(exe)).map_err(refused)?;
            if new == old {
                return Ok(Outcome::Unchanged);
            }
            replace_json(path, &new)?;
            Ok(match count_global_hooks(&old) {
                0 => Outcome::Added,
                _ => Outcome::Updated,
            })
        }
        None => {
            let (new, removed) = remove_global_hooks(&old).map_err(refused)?;
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
        assert_eq!(setup_at(&path, None).unwrap(), Outcome::NothingToRemove);
        assert!(!path.parent().unwrap().exists());
        assert_eq!(setup_at(&path, Some("/bin/adj")).unwrap(), Outcome::Added);
        assert_eq!(
            setup_at(&path, Some("/bin/adj")).unwrap(),
            Outcome::Unchanged
        );
        assert_eq!(setup_at(&path, Some("/b/adj")).unwrap(), Outcome::Updated);
        assert_eq!(
            setup_at(&path, None).unwrap(),
            Outcome::Removed(CLAUDE_EVENTS.len())
        );
    }
}
