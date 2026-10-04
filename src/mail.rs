//! The mail between a hub and its workers: the inbox a hub reads and the outbox a worker
//! reads, delivering into either and waking whoever is on the other end, the hubs of a
//! repository with the newest messages waiting for each, and reading an agent's screen before
//! typing into it.
//!
//! Names `infra`, `kernel` and `registry` and nothing else. `store` is private: where the inbox
//! and the outbox are on disk is this module's own. The transports read the inbox through
//! `pending` and the outbox through `read_outbox`, and neither hands out a path to build on.
//!
//! Delivery never fails for want of a listener. A message written while the hub is down sits
//! in the same directory the running hub reads from, and is picked up when it next starts —
//! so `present: false` is information for the sender, not an error.

use std::path::{Path, PathBuf};

use crate::infra::clock::{now_secs, utc_stamp};
use crate::infra::fs::{CLAIM_ATTEMPTS, remove_if_present, stage};
use crate::registry::{
    Context, ProcessTable, hub_records, hub_session, hub_sessions_for, hub_status, hub_status_with,
    worker_hub_key, worker_status,
};

mod ack;
mod clear_outbox;
mod deliver;
mod list;
mod list_hubs;
mod model;
mod pending;
mod read;
mod read_outbox;
mod read_screen;
mod send;
mod store;
mod tell;

pub use ack::*;
pub use clear_outbox::*;
pub use deliver::*;
pub use list_hubs::*;
pub use model::*;
pub use pending::*;
pub use read::*;
pub use read_outbox::*;
pub use read_screen::*;
pub use send::*;
pub use tell::*;

use list::list;
use store::{archive_dir, claim_link, hold, inbox_dir, outbox_path, put_back_abandoned};

#[cfg(test)]
mod tests;
