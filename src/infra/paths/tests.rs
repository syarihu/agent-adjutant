use super::*;

#[test]
fn a_missing_home_still_gives_an_absolute_anchor() {
    // A relative anchor puts the state directory under whatever directory the process
    // started in, so a hub and a worker started from different places end up with
    // different inboxes and neither can see anything wrong.
    for absent in [None, Some(""), Some("relative/home")] {
        let home = home_from(absent);
        assert!(home.is_absolute(), "{absent:?} gave {}", home.display());
    }
    assert_eq!(home_from(Some("/home/x")), PathBuf::from("/home/x"));
}

#[test]
fn the_home_fallback_is_absolute_and_not_shared_with_anyone() {
    // One shared directory would put two people's configs and inboxes in one place.
    for absent in [None, Some(""), Some("relative/home")] {
        let home = home_from(absent);
        assert!(home.is_absolute(), "{absent:?} gave {}", home.display());
        assert!(
            home.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("adjutant-no-home-"),
            "{}",
            home.display()
        );
    }
}

#[test]
fn a_relative_path_is_taken_from_the_current_directory() {
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(absolute("wt/one"), cwd.join("wt/one"));
}

#[test]
fn a_home_path_is_expanded_and_then_left_alone() {
    let home = home_dir();
    assert_eq!(absolute("~/wt"), home.join("wt"));
    assert_eq!(absolute("~"), home);
}

#[test]
fn an_absolute_path_is_unchanged() {
    assert_eq!(absolute("/tmp/wt"), PathBuf::from("/tmp/wt"));
}

#[test]
fn a_config_dir_variable_that_says_nothing_is_the_default_directory() {
    let home = Path::new("/home/a");
    for v in [None, Some(""), Some("  "), Some("\t\n")] {
        assert_eq!(
            claude_config_dir_from(v, home),
            PathBuf::from("/home/a/.claude")
        );
        assert_eq!(codex_home_from(v, home), PathBuf::from("/home/a/.codex"));
        assert_eq!(dir_said(v), None);
    }
}

#[test]
fn a_config_dir_variable_that_says_something_is_used_as_given() {
    let home = Path::new("/home/a");
    assert_eq!(
        claude_config_dir_from(Some("/cfg/a"), home),
        PathBuf::from("/cfg/a")
    );
    assert_eq!(codex_home_from(Some("/cx"), home), PathBuf::from("/cx"));
    assert_eq!(
        claude_config_dir_from(Some("rel/cfg"), home),
        PathBuf::from("rel/cfg")
    );
    assert_eq!(dir_said(Some(" /cfg ")), Some(PathBuf::from(" /cfg ")));
}
