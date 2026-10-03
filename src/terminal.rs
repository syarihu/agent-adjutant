//! Reading an agent's screen before typing into it. It stays here until it moves to `mail`.

use crate::infra::agent::Agent;
pub use crate::infra::terminal::*;

mod agent_screen;

pub use self::agent_screen::*;

#[cfg(test)]
mod tests;
