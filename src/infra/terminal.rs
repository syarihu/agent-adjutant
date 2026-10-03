//! Opening a tab somewhere, bringing one to the front, and closing one when its work is
//! over.
//!
//! iTerm2 is the built-in because it needs no setup on the machine this grew up on, but it
//! is only a default: `terminal.spawn` / `terminal.focus` in the config replace it with any
//! command line, so tmux, WezTerm, Ghostty or a plain `open -a` all work without this file
//! learning about them.
//!
//! `focus`, `title` and `wake` are optional in a way `spawn` is not. Failing to open a tab
//! loses the work; failing to raise a window, name it or poke it loses nothing, so an
//! unsupported one is a quiet no-op rather than an error. `close` belongs with `spawn`
//! rather than with those: whoever asked for it is about to remove the worktree that tab is
//! sitting in, so a failure nobody was told about is a worker killed by the cleanup.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub use crate::infra::shell::run_shell;
use crate::infra::template::{Sub, contains_placeholder, render, sh_quote};

pub struct SpawnRequest<'a> {
    pub cwd: &'a str,
    pub title: &'a str,
    /// An already-assembled shell command line. Built by the caller (see `runner`) so that
    /// quoting happens once, at the point that knows what the parts mean.
    pub command: &'a str,
    /// A command that names the tab, to run inside it before the real one. Used only where
    /// `{command}` is a shell line; a terminal that titles its own tabs does that with
    /// `{title}` instead. `None` when nothing should name it.
    pub title_command: Option<&'a str>,
}

/// What a spawn or focus did, or — under `--dry-run` — would have done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Performed {
    pub description: String,
    pub script: String,
    pub ran: bool,
    /// The built-in tmux wake declined because of what the agent's screen showed, or could
    /// not confirm what it typed. Only then is the reason worth telling the person; every
    /// other failure is reported as it always was.
    pub screen: bool,
}

mod board_session;
mod capture;
mod iterm;
mod location;
mod markers;
mod proc;
mod settings;
mod title;
mod tmux;
mod verbs;

pub use self::board_session::*;
pub use self::capture::*;
pub use self::iterm::*;
pub use self::location::*;
pub(crate) use self::markers::*;
pub use self::proc::*;
pub use self::settings::*;
pub use self::title::*;
pub use self::tmux::*;
pub use self::verbs::*;

#[cfg(test)]
mod tests;
