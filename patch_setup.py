import sys

def modify_setup():
    with open("src/transport/cli/setup.rs", "r") as f:
        content = f.read()

    # Imports
    content = content.replace(
        "use crate::infra::paths::{exe_path_absolute, home_dir};",
        "use crate::infra::paths::{claude_config_dir_from, codex_home_from, exe_path_absolute, home_dir};"
    )

    # settings_path doc and logic
    old_settings = """/// The same rule as the hook receiver's: the variable when it says something, else the default
/// account's directory.
fn settings_path(config_dir: Option<&str>, home: &Path) -> PathBuf {
    match config_dir.filter(|dir| !dir.trim().is_empty()) {
        Some(dir) => Path::new(dir).join("settings.json"),
        None => home.join(".claude").join("settings.json"),
    }
}"""

    new_settings = """/// `settings.json` in the Claude config directory (`infra::paths::claude_config_dir`).
fn settings_path(config_dir: Option<&str>, home: &Path) -> PathBuf {
    claude_config_dir_from(config_dir, home).join("settings.json")
}"""
    content = content.replace(old_settings, new_settings)

    # codex_hooks_path doc and logic
    old_codex = """/// Codex's global hooks file: under `CODEX_HOME` when it says something, else `~/.codex`.
fn codex_hooks_path(codex_home: Option<&str>, home: &Path) -> PathBuf {
    match codex_home.filter(|dir| !dir.trim().is_empty()) {
        Some(dir) => Path::new(dir).join("hooks.json"),
        None => home.join(".codex").join("hooks.json"),
    }
}"""

    new_codex = """/// Codex's global hooks file, `hooks.json` in `infra::paths::codex_home`.
fn codex_hooks_path(codex_home: Option<&str>, home: &Path) -> PathBuf {
    codex_home_from(codex_home, home).join("hooks.json")
}"""
    content = content.replace(old_codex, new_codex)

    with open("src/transport/cli/setup.rs", "w") as f:
        f.write(content)

modify_setup()
