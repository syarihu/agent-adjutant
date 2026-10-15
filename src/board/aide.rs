//! The aide as the page sees it: whether there is one, opening it, replacing it, and which tmux
//! window its terminal attaches to. The record and the launch are `registry`'s and `lifecycle`'s;
//! this is the part the resident's routes call.

mod display;
mod find;
mod open;
mod reset;
mod show;
mod window;

pub use display::*;
pub use find::*;
pub use open::*;
pub use reset::*;
pub use show::*;
pub use window::*;
