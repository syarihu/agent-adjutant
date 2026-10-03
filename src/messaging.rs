//! Addressing and delivery between a hub and its workers, over the filesystem.
//!
//! The thing being replaced here is an agent-specific pair of primitives: "list the sessions
//! this harness knows about" and "send a message to one of them". Those exist in exactly one
//! coding agent, which is why the hub only ever worked in that one.
//!
//! What both sides actually need is smaller than a session list: an address they can both
//! derive (the hub name), a way to ask whether anyone is home, and a place to leave a note.
//! A PID file answers the second and a directory answers the third, so any agent that can
//! run a command can take part.
//!
//! Delivery never fails for want of a listener. A message written while the hub is down sits
//! in the same directory the running hub reads from, and is picked up when it next starts —
//! so `present: false` is information for the sender, not an error.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{expand_home, home_dir};
use crate::repo::current_worktree;

mod board_view;
mod hub_record;
mod inbox;
mod paths;
mod plumbing;
mod proc;
mod saved_session;
mod worker_record;

pub use board_view::*;
pub use hub_record::*;
pub use inbox::*;
pub use paths::*;
pub use plumbing::*;
pub use proc::*;
pub use saved_session::*;
pub use worker_record::*;

#[cfg(test)]
mod tests;
