//! What the board does with a session beyond looking at it: reopen one that ended, restart one
//! that is running on the same conversation, open one in the person's own terminal, remove the
//! worktree of one that is finished, start one with no task, and link one to a task.
//!
//! Only the resident server serves these: each reaches outside the repository's own records,
//! and a board a hub serves lives and dies with that hub. The session a request names is looked
//! up in the board's own records, and the worktree, socket and window come from there and never
//! from the request.

mod clean_up;
mod link;
mod open;
mod restart;
mod resume;
mod start;

pub use clean_up::*;
pub use link::*;
pub use open::*;
pub use restart::*;
pub use resume::*;
pub use start::*;
