//! Reading an agent's screen moved to `mail`; re-exported until #331 so that `terminal::`
//! paths still resolve.

pub use crate::mail::{last_output_line, look_before_typing};
