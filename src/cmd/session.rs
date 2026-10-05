//! Starting a session with no task and linking one to a task live in `board::session`; the
//! old `crate::cmd::session::` paths keep working through this re-export until the callers
//! name `board` themselves.

pub(super) use crate::board::session::{link, start_request};
