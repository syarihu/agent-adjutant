import sys

def modify_check_machine():
    with open("src/board/check_machine.rs", "r") as f:
        content = f.read()

    # Drop dir_from closure and rewrite dir setup
    old_code = """    let dir_from = |key: &str| {
        std::env::var(key)
            .ok()
            .as_deref()
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
    };
    let claude_config = Some(match dir_from(crate::infra::env::CLAUDE_CONFIG_DIR_ENV) {
        Some(dir) => dir.join(".claude.json"),
        None => home.join(".claude.json"),
    });
    let codex_config = Some(match dir_from(crate::infra::env::CODEX_HOME_ENV) {
        Some(dir) => dir.join("config.toml"),
        None => home.join(".codex").join("config.toml"),
    });"""

    new_code = """    // Claude Code keeps `.claude.json` in its config directory when one is named, and beside the
    // default one (`~/.claude.json`, not `~/.claude/.claude.json`) when none is.
    let claude_config = Some(
        crate::infra::paths::dir_said(
            std::env::var(crate::infra::env::CLAUDE_CONFIG_DIR_ENV).ok().as_deref(),
        )
        .map(|dir| dir.join(".claude.json"))
        .unwrap_or_else(|| home.join(".claude.json")),
    );
    let codex_config = Some(
        crate::infra::paths::codex_home_from(
            std::env::var(crate::infra::env::CODEX_HOME_ENV).ok().as_deref(),
            &home,
        )
        .join("config.toml"),
    );"""

    content = content.replace(old_code, new_code)

    with open("src/board/check_machine.rs", "w") as f:
        f.write(content)

modify_check_machine()
