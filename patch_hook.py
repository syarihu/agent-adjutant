import sys

def modify_hook():
    with open("src/transport/cli/hook.rs", "r") as f:
        content = f.read()

    # Add imports
    content = content.replace(
        "use crate::infra::paths::home_dir;",
        "use crate::infra::paths::{claude_config_dir_from, codex_home_from, home_dir};"
    )

    # In claude_event
    old_claude = """    // The same rule as `pick_review_engine::cache_path`: the variable when it says something,
    // else the default account's directory.
    let config_dir = config_dir
        .filter(|dir| !dir.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| home.join(".claude").to_string_lossy().to_string());"""

    new_claude = """    let config_dir = claude_config_dir_from(config_dir, home)
        .to_string_lossy()
        .into_owned();"""
    content = content.replace(old_claude, new_claude)

    # In codex_event
    old_codex = """    let config_dir = codex_home
        .filter(|dir| !dir.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| home.join(".codex").to_string_lossy().to_string());"""

    new_codex = """    let config_dir = codex_home_from(codex_home, home)
        .to_string_lossy()
        .into_owned();"""
    content = content.replace(old_codex, new_codex)

    with open("src/transport/cli/hook.rs", "w") as f:
        f.write(content)

modify_hook()
