//! The hub's lifecycle: starting it in a tab or from the board, planning, claiming and
//! exec'ing its launch, reopening a saved session, bringing its tab to the front, stopping
//! it and deciding whether it can be closed.

mod claim_launch;
mod close;
mod exec_launch;
mod focus;
mod plan_launch;
mod resume;
mod start;
mod stop;
#[cfg(test)]
mod tests;

pub use claim_launch::*;
pub use close::*;
pub use exec_launch::*;
pub use focus::*;
pub use plan_launch::*;
pub use resume::*;
pub use start::*;
pub use stop::*;
