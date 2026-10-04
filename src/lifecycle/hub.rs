//! The hub's lifecycle: starting it in a tab or from the board, reopening a saved session,
//! bringing its tab to the front, stopping it and deciding whether it can be closed.

mod close;
mod focus;
mod resume;
mod start;
mod stop;

pub use close::*;
pub use focus::*;
pub use resume::*;
pub use start::*;
pub use stop::*;
