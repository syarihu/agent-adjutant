//! The bottom layer: plumbing every module needs, naming nothing above it.
//!
//! The clock, the filesystem and shell helpers, the state and home directories, the names of
//! the environment variables that are forwarded between processes, the `git` and `gh`
//! runners, and the leaf modules (`template`, `ide`, `notify`, `http`, `ws`, `pty`) that
//! import nothing from the rest of the crate. Also `terminal` (opening, raising, naming,
//! closing and waking tabs, and the settings types that drive it) and `agent` (the kind of
//! agent).

pub mod agent;
pub mod clock;
pub mod env;
pub mod fs;
pub mod gh;
pub mod git;
pub mod http;
pub mod ide;
pub mod notify;
pub mod paths;
#[cfg(unix)]
pub mod pty;
pub mod shell;
pub mod template;
pub mod terminal;
pub mod ws;
