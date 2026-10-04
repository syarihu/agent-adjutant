//! A worker's lifecycle: claiming a slot and opening its tab, reopening a saved session,
//! bringing its tab to the front, joining a running one to a task and closing it.

mod claim_slot;
mod close;
mod focus;
mod link;
mod resume;

pub use claim_slot::*;
pub use close::*;
pub use focus::*;
pub use link::*;
pub use resume::*;
