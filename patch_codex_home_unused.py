import sys

def modify():
    with open("src/board/check_machine.rs", "r") as f:
        content = f.read()

    old_block = """    let codex_config = Some(
        crate::infra::paths::codex_home_from(
            std::env::var(crate::infra::env::CODEX_HOME_ENV).ok().as_deref(),
            &home,
        )
        .join("config.toml"),
    );"""

    new_block = """    let codex_config = Some(crate::infra::paths::codex_home().join("config.toml"));"""

    if old_block in content:
        content = content.replace(old_block, new_block)
        with open("src/board/check_machine.rs", "w") as f:
            f.write(content)
        print("Updated check_machine.rs to use codex_home()")

modify()
