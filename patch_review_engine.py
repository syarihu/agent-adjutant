import sys

def modify_review_engine():
    with open("src/transport/cli/review_engine.rs", "r") as f:
        content = f.read()

    # Drop claude_config_dir from imports
    content = content.replace("claude_config_dir, decide", "decide")

    # Replace logic in run
    old_run_logic = """    let config_var = std::env::var_os("CLAUDE_CONFIG_DIR");
    let home = crate::infra::paths::home_dir();
    let config_dir = claude_config_dir(config_var.as_deref(), &home);
    let cache = cache_path(config_var.as_deref(), &home);"""

    new_run_logic = """    let config_dir = crate::infra::paths::claude_config_dir();
    let cache = cache_path(&config_dir);"""

    content = content.replace(old_run_logic, new_run_logic)

    with open("src/transport/cli/review_engine.rs", "w") as f:
        f.write(content)

modify_review_engine()
