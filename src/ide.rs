//! Opening the worktree in an editor.
//!
//! There is no list of supported editors, on purpose. Anything with a `{worktree}` in it is
//! taken as a command template, and anything else is taken as a launcher that accepts a path
//! — which is what `code`, `studio`, `idea`, `cursor`, `zed` and `nvim` all are. An editor
//! nobody here has heard of costs a config line rather than a release.

use crate::template::{Sub, render};

/// The command line that opens `worktree`, or `None` when no editor is configured. `None`
/// means "ask" — guessing puts the user in the wrong editor, which is worse than a question.
pub fn open_command(ide: Option<&str>, worktree: &str) -> Option<String> {
    let ide = ide?.trim();
    if ide.is_empty() {
        return None;
    }
    if ide.contains("{worktree}") {
        return Some(render(ide, &[("worktree", Sub::Quoted(worktree))]));
    }
    Some(render(
        &format!("{ide} {{worktree}}"),
        &[("worktree", Sub::Quoted(worktree))],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_launcher_name_is_handed_the_path() {
        assert_eq!(
            open_command(Some("studio"), "/wt/wid-957").unwrap(),
            "studio /wt/wid-957"
        );
        assert_eq!(
            open_command(Some("code"), "/wt/my app").unwrap(),
            "code '/wt/my app'"
        );
    }

    #[test]
    fn an_editor_nobody_listed_works_without_a_release() {
        assert_eq!(
            open_command(Some("emacsclient -n"), "/wt/my app").unwrap(),
            "emacsclient -n '/wt/my app'"
        );
        assert_eq!(
            open_command(Some("kitty @ launch --cwd {worktree} nvim"), "/wt/x").unwrap(),
            "kitty @ launch --cwd /wt/x nvim"
        );
    }

    #[test]
    fn no_editor_means_ask_rather_than_guess() {
        assert_eq!(open_command(None, "/wt"), None);
        assert_eq!(open_command(Some("  "), "/wt"), None);
    }
}
