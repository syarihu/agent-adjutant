//! The reader of a read that is open, which closing it starts from.

use std::path::Path;

use crate::task::PrRef;

use super::store;
use super::take_over_read::HungReader;

/// The reader of `pr`'s read when one is open: the read is `running` or `ready`, has a session,
/// and its reader was not closed. `None` for anything else.
pub fn open_reader(root: &Path, pr: &PrRef) -> Result<Option<HungReader>, String> {
    let Some(record) = store::load(root, pr)? else {
        return Ok(None);
    };
    Ok(record.ai.filter(|ai| ai.reader_open()).and_then(|ai| {
        Some(HungReader {
            session_id: ai.session_id?,
            pid: ai.pid,
            ps_started: ai.ps_started,
            terminal: ai.terminal,
        })
    }))
}
