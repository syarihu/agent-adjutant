//! A reader saying it is there: its process and its window, written down.

use std::path::Path;

use crate::infra::terminal::SessionTerminal;
use crate::task::PrRef;

use super::store;

/// Record the reader of `pr` that runs as `session_id`: its pid, when that process started, and
/// where its window is. Refused unless the record is `running` under that very session, so a
/// reader that was superseded (the read was asked for again, or it was failed) exits without
/// starting an agent.
pub fn note_reader(
    root: &Path,
    pr: &PrRef,
    session_id: &str,
    pid: u32,
    ps_started: Option<String>,
    terminal: Option<SessionTerminal>,
) -> Result<(), String> {
    let _lock = store::lock(root, &store::stem(pr))?;
    let mut record = store::load(root, pr)?.ok_or("no record of this pull request")?;
    let ai = record
        .ai
        .as_mut()
        .filter(|ai| ai.status == "running" && ai.session_id.as_deref() == Some(session_id))
        .ok_or("this reader is no longer the one the read-through waits for")?;
    ai.pid = Some(pid);
    ai.ps_started = ps_started;
    ai.terminal = terminal;
    store::save(root, &mut record)
}
