//! A gate: something an agent has prepared for a person to look at, and the ball handed
//! over with it.
//!
//! Not a question with buttons. The agent stops, puts the artefact where a person can see
//! it, and waits — so the shape of the payload is the point, and it is three frames rather
//! than one wall of prose: what to look at, what was already decided, and what the agent
//! was unsure of. A reviewer who has to read four hundred lines to find the two decisions
//! that matter is a reviewer who approves without reading.
//!
//! The answer travels back out through the outbox the hub already uses to reach a worker,
//! so a gate needs no channel of its own — and an answer to a worktree whose worker has
//! died simply waits there, which is the same promise `adjutant tell` already makes.
//!
//! Reads take a state root and a hub's slug, so the board can read every hub's gates; writes
//! take the `Context` of the hub they are for. Nothing outside this module builds
//! `gates/<slug>/…` or takes a gate's lock: `store` is private, and opening, answering and
//! closing a gate are operations of this module beside it.
//!
//! Names `infra`, `kernel`, `registry`, `mail` and `task`, all below it.

mod answer;
mod close;
mod close_resumed;
mod get;
mod list;
mod model;
mod open;
mod resume_signals;
mod store;

pub use answer::*;
pub use close::*;
pub use close_resumed::*;
pub use get::*;
pub use list::*;
pub use model::*;
pub use open::*;
pub use resume_signals::*;

#[cfg(test)]
mod tests;
