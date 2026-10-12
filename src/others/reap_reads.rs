//! Failing the readers that died, so a slot is never held by one that is not there.

use std::path::Path;

use crate::infra::clock::utc_stamp;
use crate::registry::Liveness;
use crate::task::pr_ref;

use super::fail_read::fail_read;
use super::store;

/// How long a reader that has not said where it is may take to start, in seconds.
pub const READER_START_GRACE_SECS: i64 = 90;

/// Fail every `running` read whose reader is gone, and say which (by id). `alive` is asked of a
/// recorded pid and the time that process started. A reader that cannot be told apart from a
/// live one is left as it is: failing it on a `ps` that did not answer would start a second
/// reader beside the first. One that never recorded a pid is waited for until the grace is over.
pub fn reap_reads(
    root: &Path,
    now: i64,
    alive: &dyn Fn(u32, Option<&str>) -> Liveness,
) -> Vec<String> {
    // Stamps sort as text, so the cut-off is a stamp.
    let cutoff = utc_stamp(now - READER_START_GRACE_SECS);
    let mut failed = Vec::new();
    for record in store::list(root) {
        let Some(ai) = record.ai.as_ref().filter(|ai| ai.status == "running") else {
            continue;
        };
        let why = match ai.pid {
            Some(pid) => match alive(pid, ai.ps_started.as_deref()) {
                Liveness::Gone => "the reader exited before it handed in a read-through",
                Liveness::Alive | Liveness::CannotTell => continue,
            },
            None => match ai.launched_at.as_deref() {
                Some(at) if at >= cutoff.as_str() => continue,
                // Past the grace, or with no launch time to wait from.
                _ => "the reader did not start",
            },
        };
        let Some(pr) = pr_ref(&record.url, None) else {
            continue;
        };
        if matches!(
            fail_read(root, &pr, ai.session_id.as_deref(), why, now),
            Ok(true)
        ) {
            failed.push(record.id);
        }
    }
    failed
}
