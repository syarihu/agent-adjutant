//! Reading `config.json` and turning it into the one shape the prompts are allowed to see.
//!
//! Everything mechanical about the config lives here — the defaults merge, the flat
//! shorthand, the required-key checks — because a rule written as prose in three prompt
//! files is a rule with three subtly different versions.
//!
//! The resolver never fails on a bad config. It returns warnings instead: a hub that
//! refuses to start because one repo entry is malformed is worse than a hub that starts and
//! says what is wrong.

#[cfg(test)]
use serde_json::{Map, Value, json};
#[cfg(test)]
use std::path::{Path, PathBuf};

use crate::infra::env::{
    CONFIG_ENV, STARTUP_DASHBOARD_ENV, TMUX_SESSION_ENV, TMUX_SOCKET_ENV, XDG_CONFIG_HOME_ENV,
};
use crate::infra::paths::{expand_home, home_dir};
use crate::infra::terminal::{Hook, TerminalSettings, Wake};

mod defaults;
mod location;
mod resolve;
mod settings;

pub use defaults::*;
pub use location::*;
pub use resolve::*;
pub use settings::*;

#[cfg(test)]
mod tests;
