//! The code-review hub's readers: asking for a read, starting the readers that are next in line,
//! running one, and taking its hand-in. One file per operation; the records and the queue
//! rules are `others`'s, and this module is the part that starts processes.

mod close_reader;
mod close_readers;
mod dispatch;
mod fail;
mod launch_reader;
mod request;
mod run_reader;
mod settle_reader;
mod stop_reader;
mod submit;
#[cfg(test)]
mod tests;

pub use close_reader::*;
pub use close_readers::*;
pub use dispatch::*;
pub use fail::*;
pub use launch_reader::*;
pub use request::*;
pub use run_reader::*;
pub use settle_reader::*;
pub use stop_reader::*;
pub use submit::*;
