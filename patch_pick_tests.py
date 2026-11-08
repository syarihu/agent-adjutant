import sys

def modify_tests():
    with open("src/task/pick_review_engine/tests.rs", "r") as f:
        content = f.read()

    # Rename the test and update it
    old_test = """#[test]
fn the_cache_sits_under_claude_config_dir_or_home() {
    let home = Path::new("/home");
    assert_eq!(
        cache_path(Some(OsStr::new("/cfg/a")), home),
        PathBuf::from("/cfg/a/rate-limit-cache.json")
    );
    assert_eq!(
        cache_path(None, home),
        PathBuf::from("/home/.claude/rate-limit-cache.json")
    );
}"""
    new_test = """#[test]
fn the_cache_sits_directly_under_the_config_dir() {
    assert_eq!(
        cache_path(Path::new("/cfg/a")),
        PathBuf::from("/cfg/a/rate-limit-cache.json")
    );
}"""
    content = content.replace(old_test, new_test)

    # Delete the blank test
    blank_test = """#[test]
fn a_blank_claude_config_dir_is_the_default_account() {
    let home = Path::new("/home");
    assert_eq!(
        claude_config_dir(Some(OsStr::new("  ")), home),
        PathBuf::from("/home/.claude")
    );
    assert_eq!(
        claude_config_dir(Some(OsStr::new("")), home),
        PathBuf::from("/home/.claude")
    );
    assert_eq!(
        claude_config_dir(Some(OsStr::new("\t\n")), home),
        PathBuf::from("/home/.claude")
    );
}"""
    content = content.replace(blank_test, "")

    with open("src/task/pick_review_engine/tests.rs", "w") as f:
        f.write(content)

modify_tests()
