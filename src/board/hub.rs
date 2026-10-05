//! What the board does with a hub: find it among the repository's hubs, start it (or the hub of
//! a parent task, by its key), stop it, close it, and reset or restart it.
//!
//! Only the resident server serves these: each reaches outside the repository's own records,
//! and a board a hub serves lives and dies with that hub. None of them prints or words
//! anything; each answers a value, and the caller says what it came to.

mod close;
mod find;
mod reset;
mod restart;
mod start;
mod stop;

pub use close::*;
pub use find::*;
pub use reset::*;
pub use restart::*;
pub use start::*;
pub use stop::*;
