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
mod registry;
mod session;
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
        pub fn new(config_json: &str) -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().unwrap();
            let config = dir.path().join("config.json");
            std::fs::write(&config, config_json).unwrap();
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

        /// For tests that only care about where messages go.
        pub fn empty() -> Self {
            Sandbox::new("{\"repos\": {}}")
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
}
