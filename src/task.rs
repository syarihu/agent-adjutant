//! A task the dashboard was handed, as a record that outlives the message announcing it.
//!
//! The message in the inbox is the *notification* — it is read once and acked, and after
//! that the hub has no way to say what became of the thing. The record is what the board
//! reads: one file per task, rewritten in place as the work moves. Both are derived from
//! this module so that only one of them is authored: `render_request` builds the message
//! body out of the same struct the file holds, rather than a second description of a task
//! kept in step by hand.
//!
//! A leaf: it is handed the directory to work in rather than deriving it, so it never has
//! to know where this machine keeps its state.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod github;
mod model;
mod next;
mod store;

pub use github::*;
pub use model::*;
pub use next::*;
// The commands, the board, `gate` and `jules` call these today; #351 narrows this.
pub use store::{claim_id, dir, list, load, path_of, save, stamp};

#[cfg(test)]
mod tests;
