import sys

def modify_pick_review():
    with open("src/task/pick_review_engine.rs", "r") as f:
        content = f.read()

    # Delete use std::ffi::OsStr;
    content = content.replace("use std::ffi::OsStr;\n", "")

    # Delete claude_config_dir and its doc string
    block_to_delete = """/// `${CLAUDE_CONFIG_DIR:-$HOME/.claude}`: the variable when it says something, else the default
/// account's directory. Takes both inputs as arguments so a test answers for values it chose
/// rather than for the machine it runs on. The rule mirrors the one `adj hook claude` stores a
/// row's `configDir` by, so the two name the same directory for the same account.
pub fn claude_config_dir(var: Option<&OsStr>, home: &Path) -> PathBuf {
    var.and_then(OsStr::to_str)
        .filter(|dir| !dir.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"))
}"""
    content = content.replace(block_to_delete, "")

    # Replace cache_path
    old_cache = """/// `rate-limit-cache.json` under `claude_config_dir`.
pub fn cache_path(claude_config_dir_var: Option<&OsStr>, home: &Path) -> PathBuf {
    claude_config_dir(claude_config_dir_var, home).join(CACHE_FILE)
}"""

    new_cache = """/// `rate-limit-cache.json` under the Claude config directory (`infra::paths::claude_config_dir`).
pub fn cache_path(claude_config_dir: &Path) -> PathBuf {
    claude_config_dir.join(CACHE_FILE)
}"""

    content = content.replace(old_cache, new_cache)

    with open("src/task/pick_review_engine.rs", "w") as f:
        f.write(content)

modify_pick_review()
