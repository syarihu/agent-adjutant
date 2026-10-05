//! A worker's lifecycle: claiming a slot and opening its tab, starting a fresh one, reopening a
//! saved session, bringing its tab to the front, joining a running one to a task and closing it.

mod claim_slot;
mod close;
mod focus;
mod link;
mod resume;
mod start;

pub use close::*;
pub use focus::*;
pub use link::*;
pub use resume::*;
pub use start::*;
