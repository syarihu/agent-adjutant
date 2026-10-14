//! The aide: the one session of the machine that belongs to no repository. Opening it, running
//! it, stopping it and replacing it, one file per operation; the record and the claim are
//! `registry`'s, and this module is the part that starts and stops processes.

mod home;
mod launch;
mod open;
mod reset;
mod run;
mod stop;
mod write_settings;

pub use home::*;
pub use launch::*;
pub use open::*;
pub use reset::*;
pub use run::*;
pub use stop::*;
pub use write_settings::*;
