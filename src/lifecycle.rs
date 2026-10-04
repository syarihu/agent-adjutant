//! Hub and worker lifecycle: starting, stopping, reopening and focusing the agents the
//! board and the subcommands hand work to, one file per operation. Names infra, kernel,
//! registry and mail, and nothing above them.

pub mod hub;
mod model;
pub mod worker;

pub use model::*;

#[cfg(test)]
mod tests;
