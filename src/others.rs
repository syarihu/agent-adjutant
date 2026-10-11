//! PRs other people asked you to review: one record per PR under `others/`, the sync that reads
//! them, the state derived from them, and the read-through a reader makes of each (the queue of
//! readers, what they hand in, the material they read). Reads and writes take the state root: these records
//! are the person's, not a hub's.
//!
//! Names `infra`, `kernel`, `registry` and `task`. `store` is private: nothing else builds the
//! paths under `others/` or takes the locks there.

mod claim_reads;
mod fail_read;
mod github;
mod list;
mod model;
mod note_reader;
mod pr_of;
mod read_context;
mod read_through;
mod read_through_get;
mod reader_dir;
mod reap_reads;
mod request_read;
mod scope;
mod store;
mod submit_read;
mod sync;

pub use claim_reads::*;
pub use fail_read::*;
pub use list::*;
pub use model::*;
pub use note_reader::*;
pub use pr_of::*;
pub use read_context::*;
pub use read_through_get::*;
pub use reader_dir::*;
pub use reap_reads::*;
pub use request_read::*;
pub use submit_read::*;
pub use sync::*;

#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;
