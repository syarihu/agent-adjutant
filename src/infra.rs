//! The bottom layer: plumbing every module needs, naming nothing above it.
//!
//! The clock, the filesystem and shell helpers, the state and home directories, the names of
//! the environment variables that are forwarded between processes, the `git` and `gh`
//! runners, and the leaf modules (`template`, `ide`, `notify`, `http`, `ws`, `pty`) that
//! import nothing from the rest of the crate. The one exception is `notify`, which still
//! names `config::Hook` until that type moves down as well.

pub mod clock;
pub mod env;
pub mod fs;
pub mod gh;
pub mod git;
pub mod http;
pub mod ide;
pub mod notify;
pub mod paths;
pub mod pty;
pub mod shell;
pub mod template;
pub mod ws;
