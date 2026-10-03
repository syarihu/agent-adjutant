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
