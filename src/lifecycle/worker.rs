//! A worker's lifecycle: claiming a slot and opening its tab, reopening a saved session,
//! bringing its tab to the front and joining a running one to a task.

mod claim_slot;
mod focus;
mod link;
mod resume;

pub use claim_slot::*;
pub use focus::*;
pub use link::*;
pub use resume::*;
