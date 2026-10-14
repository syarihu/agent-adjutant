use std::path::{Path, PathBuf};

use crate::infra::fs::{read_json, write_json};
use crate::kernel::agent_hooks::{aide_settings, claude_hook_command, claude_status_line_command};
use crate::kernel::identity::short_digest;

/// Write `agent-hooks/aide-<digest of exe>.json` under `state`, and give its path: the hooks of
/// every other session plus a status line that relays context use. It is the aide's own file and
/// not the shared `claude-<digest>.json`, because a status line there would replace the person's
/// in every hub and worker.
///
/// Written only when missing or different, through a temporary file and a rename, like
/// `write_agent_hooks`. Its sweep leaves this file alone: it removes only `claude-XXXXXXXX.json`.
pub fn write_aide_settings(state: &Path, exe: &str) -> Result<PathBuf, String> {
    if !Path::new(exe).is_absolute() {
        return Err(format!("the binary's path is not absolute: {exe}"));
    }
    let dir = state.join("agent-hooks");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = dir.join(format!("aide-{}.json", short_digest(exe)));
    let wanted = aide_settings(&claude_hook_command(exe), &claude_status_line_command(exe));
    if read_json(&path).as_ref() != Some(&wanted) {
        write_json(&path, &wanted)?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::write_agent_hooks;

    #[test]
    fn the_file_is_named_by_the_binary_and_rewritten_only_when_it_differs() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_aide_settings(dir.path(), "/bin/adj").unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let digest = name
            .strip_prefix("aide-")
            .unwrap()
            .strip_suffix(".json")
            .unwrap();
        assert_eq!(digest.len(), 8);
        assert!(digest.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        let written = read_json(&path).unwrap();
        assert!(
            written["statusLine"]["command"]
                .as_str()
                .unwrap()
                .contains("--status-line")
        );
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(write_aide_settings(dir.path(), "/bin/adj").unwrap(), path);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
        assert!(write_aide_settings(dir.path(), "adj").is_err());
    }

    #[test]
    fn the_sweep_of_the_shared_hooks_file_leaves_it_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_aide_settings(dir.path(), "/nonexistent/adj").unwrap();
        // A binary that is gone is what makes the sweep remove a `claude-` file.
        write_agent_hooks(dir.path(), "/nonexistent/other").unwrap();
        assert!(path.exists());
    }
}
