//! PRs other people asked you to review: one record per PR under `others/`, the sync that reads
//! them, the state derived from them, and the read-through a reader makes of each (the queue of
//! readers, what they hand in, the material they read). Reads and writes take the state root: these records
//! are the person's, not a hub's.
//!
//! Names `infra`, `kernel`, `registry` and `task`. `store` is private: nothing else builds the
//! paths under `others/` or takes the locks there.

mod claim_reads;
mod diff_get;
mod drafts;
mod drafts_get;
mod fail_read;
mod github;
mod list;
mod model;
mod note_reader;
mod pr_of;
mod push_pending;
mod read_context;
mod read_through;
mod read_through_get;
mod reader_dir;
mod reap_reads;
mod record_get;
mod request_read;
mod save_drafts;
mod scope;
mod store;
mod submit_read;
mod sync;
mod take_over_read;

pub use claim_reads::*;
pub use diff_get::*;
pub use drafts_get::*;
pub use fail_read::*;
pub use list::*;
pub use model::*;
pub use note_reader::*;
pub use pr_of::*;
pub use push_pending::*;
pub use read_context::*;
pub use read_through_get::*;
pub use reader_dir::*;
pub use reap_reads::*;
pub use record_get::*;
pub use request_read::*;
pub use save_drafts::*;
pub use submit_read::*;
pub use sync::*;
pub use take_over_read::*;

#[cfg(test)]
mod pending_tests;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;
