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

mod create;
mod edit;
mod fetch_issue;
mod find_branch_prs;
mod get;
mod github;
mod hand_over;
mod known_title;
mod list;
mod model;
mod next;
mod note_gate_answered;
mod nudge;
mod pick_review_engine;
mod read_issue_parents;
mod refresh;
mod remove;
mod store;
mod update;
mod write_brief;

pub use create::*;
pub use edit::*;
pub use fetch_issue::*;
pub use find_branch_prs::*;
pub use get::*;
pub use github::*;
pub use hand_over::*;
pub use known_title::*;
pub use list::*;
pub use model::*;
pub use next::*;
pub use note_gate_answered::*;
pub use nudge::*;
pub use pick_review_engine::*;
pub use read_issue_parents::*;
pub use refresh::*;
pub use remove::*;
pub use update::*;
pub use write_brief::*;
// The board's fixture in src/board/tests.rs writes a record by hand.
#[cfg(test)]
pub(crate) use store::dir;

#[cfg(test)]
mod tests;
