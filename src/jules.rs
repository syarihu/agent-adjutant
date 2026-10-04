//! Jules, the agent that implements a task in Google's cloud: a session started from an
//! approved plan, the pull request it opens brought to the hub, and the review comments on
//! that pull request passed on to it.
//!
//! Jules keeps no records of its own: the session id and what was relayed are fields of the
//! task record, and the rest is asked of the API each time. So there is no `store`; the API
//! client, which is where its state lives, takes that place as the private `api`.
//!
//! Names `infra`, `registry`, `mail` and `task` and nothing else.

mod announce_review;
mod api;
mod findings;
mod follow;
mod model;
mod relay;

pub use announce_review::*;
// For `start` and `show` in `cmd`, and for `Watch`; #362 takes them out.
pub use api::{create, get};
pub use findings::*;
pub use follow::*;
pub use model::*;
pub use relay::*;

#[cfg(test)]
mod tests;
