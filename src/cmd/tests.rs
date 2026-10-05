use super::*;

/// The resident is detached and stands elsewhere, so it is handed the root it was given, not
/// whatever its own working directory would make of a relative one.
#[test]
fn the_resident_is_started_with_the_root_it_was_given() {
    use std::process::Stdio;
    let sandbox = crate::testing::Sandbox::empty();
    let _state =
        crate::testing::EnvVar::set(&sandbox, crate::infra::env::STATE_DIR_ENV, "relative-state");
    let root = crate::registry::state_root(Some(std::path::Path::new("/src/widget")));
    let command = serve::resident_command(
        std::path::Path::new("adj"),
        0,
        &root,
        Stdio::null(),
        Stdio::null(),
    );
    let value = command
        .get_envs()
        .find(|(name, _)| *name == crate::infra::env::STATE_DIR_ENV)
        .and_then(|(_, value)| value);
    assert_eq!(
        value,
        Some(std::path::Path::new("/src/widget/relative-state").as_os_str())
    );
}
