//! A task the dashboard was handed, as a record that outlives the message announcing it.
//!
//! The message in the inbox is the *notification* — it is read once and acked, and after
//! that the hub has no way to say what became of the thing. The record is what the board
//! reads: one file per task, rewritten in place as the work moves. Both are derived from
//! this module so that only one of them is authored: `render_request` builds the message
//! body out of the same struct the file holds, rather than a second description of a task
//! kept in step by hand.
//!
//! Reads take a state root and a hub's slug, so the board can read every hub's records;
//! writes take the `Context` of the hub they are for and find `tasks/<slug>/` under its state
//! directory. Nothing outside this module builds that path or takes the record's lock.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod get;
mod github;
mod list;
mod model;
mod next;
mod note_gate_answered;
mod remove;
mod store;

pub use get::*;
pub use github::*;
pub use list::*;
pub use model::*;
pub use next::*;
pub use note_gate_answered::*;
pub use remove::*;
// For their callers in `cmd`: `claim_id` and `stamp` until #354, `save` and `lock` until #362.
pub use store::{claim_id, lock, save, stamp};
// The board's fixture in src/cmd/serve/tests.rs writes a record by hand.
#[cfg(test)]
pub(crate) use store::dir;

#[cfg(test)]
mod tests;
