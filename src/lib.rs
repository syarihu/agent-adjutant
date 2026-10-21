//! adjutant — hands work out to workers, takes their reports back in.
//!
//! One binary, two modes. The subcommands are for shells, hooks and launchers — the places
//! that run *before* an agent exists and so cannot ask one for anything. `adjutant mcp` is
//! the same machinery served to an agent that is already running.
//!
//! Shipped as a library with two thin binaries over it (`adjutant` and its short name
//! `adj`), so the same code is one build rather than two, and the integration tests can
//! drive it directly.

mod board;
mod gate;
mod infra;
mod jules;
mod kernel;
mod lifecycle;
mod mail;
mod others;
mod registry;
mod task;
mod transport;

pub use transport::cli::run;

/// Scaffolding the tests share. Not a layer — nothing outside `#[cfg(test)]` may reach it,
/// which is why `check-layering.sh` lets any module name it.
#[cfg(test)]
pub(crate) mod testing {
    /// `ADJUTANT_CONFIG`, `ADJUTANT_STATE_DIR` and `ADJUTANT_HUB` are process-global and
    /// the harness runs tests in parallel threads, so a sandbox has to be exclusive or two
    /// tests read each other's config and each other's inboxes.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub struct Sandbox {
        dir: tempfile::TempDir,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Sandbox {
        /// Point both the config and the state directory at a tempdir. A test that reads
        /// the real ones answers about the developer's own machine, and a test that writes
        /// to them delivers its fixtures to a hub that is actually running.
        ///
        /// A config that is a JSON object without a `notification` gets `"notification":
        /// "true"` at the top level. Left unset it resolves to the built-in notifier, which on
        /// macOS posts a real banner every time a test reaches a delivery path; a string is a
        /// command template, and this one runs `true`. The top level is the lowest-priority
        /// level, so a fixture that sets `notification` in a repo entry or in `defaults` keeps
        /// its own, and one that sets it at the top level (to anything but `null`) is written as
        /// given. So is a config that is not a JSON object: some tests feed broken ones on
        /// purpose.
        pub fn new(config_json: &str) -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().unwrap();
            let config = dir.path().join("config.json");
            std::fs::write(&config, quiet_config(config_json)).unwrap();
            unsafe {
                std::env::set_var("ADJUTANT_STATE_DIR", dir.path().join("state"));
                std::env::set_var("ADJUTANT_CONFIG", &config);
                // `cargo test` run from inside a hub's own session inherits this, and every
                // test that asserts on an address would then be answering about that hub.
                std::env::remove_var("ADJUTANT_HUB");
                // The same trap one variable over, and a quieter one: a hub started with
                // `--no-dashboard` exports this, so a test run in that hub's tab would see
                // `startupDashboard` resolve to `false` no matter what its fixture said —
                // and the settings tests would fail for a reason nothing in them mentions.
                std::env::remove_var(crate::infra::env::STARTUP_DASHBOARD_ENV);
                // `cargo test` run from a git hook inherits `GIT_DIR`, and a test that sets
                // up a repository with git directly would then set up the developer's own.
                // Cleared here too so that one a test set and failed to take back ends with
                // the next sandbox.
                for name in crate::infra::git::REPOSITORY_LOCATION_ENV {
                    std::env::remove_var(name);
                }
            }
            Sandbox { dir, _guard: guard }
        }

        /// The state directory this sandbox points `ADJUTANT_STATE_DIR` at.
        pub fn state(&self) -> std::path::PathBuf {
            self.dir.path().join("state")
        }

        /// For tests that only care about where messages go. Quiet like every sandbox: see
        /// `new`.
        pub fn empty() -> Self {
            Sandbox::new("{\"repos\": {}}")
        }
    }

    /// `config_json` with a silent `notification` added when it is a JSON object that says
    /// nothing about one; anything else comes back as it was given. `Fixture::new` in
    /// `tests/common/mod.rs` keeps a copy of this rule for the integration tests, which cannot
    /// reach this module, so a change here goes there too.
    fn quiet_config(config_json: &str) -> String {
        match serde_json::from_str::<serde_json::Value>(config_json) {
            // `null` counts as unset, as it does when the config is resolved.
            Ok(serde_json::Value::Object(mut map))
                if map
                    .get("notification")
                    .is_none_or(serde_json::Value::is_null) =>
            {
                map.insert("notification".into(), "true".into());
                serde_json::Value::Object(map).to_string()
            }
            _ => config_json.to_string(),
        }
    }

    /// A repository at `dir` on `branch`, set up through `crate::infra::git::git` so that a variable a
    /// test has already exported cannot send the setup somewhere else.
    pub fn init_repo(dir: &std::path::Path, branch: &str) {
        let out = crate::infra::git::git(&["init", "-q", "-b", branch], Some(dir)).unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A variable set for as long as this lives, and removed after.
    ///
    /// Borrows the `Sandbox` for as long as it lives, so that it is only ever set while the
    /// environment lock is held: the variable is process-global, and a test running beside
    /// this one would see it.
    pub struct EnvVar<'a>(&'static str, std::marker::PhantomData<&'a Sandbox>);

    impl<'a> EnvVar<'a> {
        pub fn set(
            _sandbox: &'a Sandbox,
            name: &'static str,
            value: impl AsRef<std::ffi::OsStr>,
        ) -> Self {
            unsafe { std::env::set_var(name, value) };
            EnvVar(name, std::marker::PhantomData)
        }
    }

    impl Drop for EnvVar<'_> {
        fn drop(&mut self) {
            unsafe { std::env::remove_var(self.0) };
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::infra::terminal::Hook;

        fn notification() -> Hook {
            crate::kernel::config::resolve_config("acme/widget")
                .unwrap()
                .settings
                .notification
        }

        #[test]
        fn a_sandbox_never_resolves_to_the_built_in_notifier() {
            let _empty = Sandbox::empty();
            assert_ne!(notification(), Hook::BuiltIn);
            drop(_empty);
            let _bare = Sandbox::new("{}");
            assert_ne!(notification(), Hook::BuiltIn);
        }

        #[test]
        fn a_fixtures_own_notification_is_kept() {
            let _sandbox = Sandbox::new(r#"{"notification": false, "repos": {}}"#);
            assert_eq!(notification(), Hook::Off);
        }

        #[test]
        fn a_null_notification_is_quiet_too() {
            let _sandbox = Sandbox::new(r#"{"notification": null, "repos": {}}"#);
            assert_ne!(notification(), Hook::BuiltIn);
        }

        #[test]
        fn a_config_that_is_not_json_is_written_as_given() {
            let sandbox = Sandbox::new("{ not json");
            let written =
                std::fs::read_to_string(sandbox.state().parent().unwrap().join("config.json"))
                    .unwrap();
            assert_eq!(written, "{ not json");
        }
    }
}
