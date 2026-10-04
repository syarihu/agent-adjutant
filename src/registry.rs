//! Who is running, and where: hub and worker records, saved sessions, liveness, worker slots
//! and markers, the claim and dispatch locks, addressing, `Context`, and the board address book
//! and server record.
//!
//! Names `infra` and `kernel` and nothing else. `store` is private: the paths and raw reads
//! behind the records are this module's own, and other modules ask through the typed reads.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::infra::clock::{now_secs, utc_stamp};
use crate::infra::env::HUB_ENV;
use crate::infra::fs::{
    CreateError, create_new_json, read_json, record_exists, remove_if_present, write_json,
};
use crate::infra::paths::state_dir;
use crate::kernel::identity::current_worktree;

mod address;
mod board_record;
mod claim;
mod context;
mod dispatch_lock;
mod liveness;
mod mark_removing;
mod model;
mod register;
mod relink;
mod saved_session;
mod set_phase;
mod slot;
mod status;
mod store;
mod unregister;

pub use address::*;
pub use board_record::*;
pub use claim::*;
pub use context::*;
pub use dispatch_lock::*;
pub use liveness::*;
pub use mark_removing::*;
pub use model::*;
pub use register::*;
pub use relink::*;
pub use saved_session::*;
pub use set_phase::*;
pub use slot::*;
pub use status::*;
pub use unregister::*;

// For the tests outside `registry` that write record fixtures (the inbox tests,
// `cmd/serve/tests.rs`).
#[cfg(test)]
pub(crate) use store::{boards_dir, hub_record_path, worker_record_path};

#[cfg(test)]
mod tests;
