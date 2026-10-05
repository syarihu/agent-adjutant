//! A worker's lifecycle: claiming a slot and opening its tab, starting a fresh one, planning,
//! registering and exec'ing its launch, reopening a saved session, bringing its tab to the
//! front, joining a running one to a task and closing it.

mod claim_slot;
mod close;
mod exec_launch;
mod focus;
mod link;
mod plan_launch;
mod register_launch;
mod resume;
mod start;
#[cfg(test)]
mod tests;

pub use close::*;
pub use exec_launch::*;
pub use focus::*;
pub use link::*;
pub use plan_launch::*;
pub use register_launch::*;
pub use resume::*;
pub use start::*;
