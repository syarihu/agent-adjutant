//! Asking how a session is doing.

use super::*;

use crate::infra::terminal::Hook;

/// Ask how a session is doing.
pub fn get(key: &Hook, id: &str) -> Result<Session, String> {
    check_id(id)?;
    parse_session(&api::call(key, "GET", &format!("sessions/{id}"), None)?)
}
