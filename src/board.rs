//! The board: the server that answers the page, the read model it answers from, the jobs it
//! runs in the background beside answering requests, and the operations a person runs from
//! it. Names `infra`, `kernel`, `registry`, `mail`, `task`, `gate`, `jules` and `lifecycle`,
//! and nothing above them.
//!
//! The dashboard is not a second coordination system. Every button on it ends in something
//! this binary could already do: a task handed over becomes a `request` in the hub's inbox
//! and a poke on its tab, exactly as `adj send` would. What the server adds is a view of
//! state that until now could only be read one `adj` invocation at a time, and a place to
//! put the questions a worker used to have to ask into a tab nobody was watching.
//!
//! It holds one clock, and only in the resident server: the poll that keeps the cards' pull
//! requests up to date (`pr_poll`). A board served by itself, or by a hub, has none: there a
//! request arrives because a person clicked, and that is the only thing that moves. `/api/state`
//! never asks GitHub on either, since the page polls it every couple of seconds.

pub mod hub;
pub mod jobs;
pub mod session;
pub mod view;

mod daemon;
mod dedicated;
mod last_lines;
mod model;
mod resident;
mod server;
#[cfg(test)]
mod tests;
mod token;
mod url;

pub use daemon::*;
pub use dedicated::*;
pub use last_lines::*;
pub use model::*;
pub use resident::*;
pub use server::*;
pub use token::*;
pub use url::*;
