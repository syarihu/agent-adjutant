//! The session types live in `board`; the old `crate::session::` paths keep working through
//! this re-export until the callers name `board` themselves.

pub use crate::board::{Session, SessionRequest, SessionWaiting, WaitingChoice};
