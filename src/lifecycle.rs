//! Hub and worker lifecycle: starting, stopping, reopening and focusing the agents the
//! board and the subcommands hand work to, one file per operation. Names infra, kernel,
//! registry, mail and task, and nothing above them.

pub mod hub;
mod model;
pub mod worker;
mod write_agent_hooks;

pub use model::*;
pub use write_agent_hooks::*;

#[cfg(test)]
mod tests;
