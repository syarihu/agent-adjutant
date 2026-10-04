//! A worker's lifecycle: claiming a slot and opening its tab, reopening a saved session and
//! bringing its tab to the front.

mod claim_slot;
mod focus;
mod resume;

pub use claim_slot::*;
pub use focus::*;
pub use resume::*;
