import sys

def modify_paths_rs():
    with open("src/infra/paths.rs", "r") as f:
        content = f.read()

    new_imports = "use crate::infra::env::{CLAUDE_CONFIG_DIR_ENV, CODEX_HOME_ENV, STATE_DIR_ENV};\nuse std::path::{Path, PathBuf};"
    content = content.replace("use crate::infra::env::STATE_DIR_ENV;\nuse std::path::PathBuf;", new_imports)

    search = "pub(super) fn home_from(home: Option<&str>) -> PathBuf {"

    insert_block = """
/// A directory variable's value when it says something: set and not blank. Used as given, not
/// trimmed and not made absolute. Split out from the readers below so a test answers for the
/// value it chose rather than for the machine it runs on.
pub fn dir_said(var: Option<&str>) -> Option<PathBuf> {
    var.filter(|dir| !dir.trim().is_empty()).map(PathBuf::from)
}

/// Where the Claude Code account keeps its files: `CLAUDE_CONFIG_DIR` when it says something,
/// else `~/.claude`. The hook receiver stores a session row's `configDir` by this rule and
/// `adj review-engine` looks the row up by it, so the two name the same directory for the
/// same account.
pub fn claude_config_dir() -> PathBuf {
    claude_config_dir_from(std::env::var(CLAUDE_CONFIG_DIR_ENV).ok().as_deref(), &home_dir())
}

/// `claude_config_dir` for a value and a home handed in.
pub fn claude_config_dir_from(var: Option<&str>, home: &Path) -> PathBuf {
    dir_said(var).unwrap_or_else(|| home.join(".claude"))
}

/// Where Codex keeps its files: `CODEX_HOME` when it says something, else `~/.codex`.
pub fn codex_home() -> PathBuf {
    codex_home_from(std::env::var(CODEX_HOME_ENV).ok().as_deref(), &home_dir())
}

/// `codex_home` for a value and a home handed in.
pub fn codex_home_from(var: Option<&str>, home: &Path) -> PathBuf {
    dir_said(var).unwrap_or_else(|| home.join(".codex"))
}
"""

    # insert block directly after home_from and before expand_home according to instruction
    search2 = "pub fn expand_home(path: &str) -> PathBuf {"

    content = content.replace(search2, insert_block + "\n" + search2)

    with open("src/infra/paths.rs", "w") as f:
        f.write(content)

modify_paths_rs()
