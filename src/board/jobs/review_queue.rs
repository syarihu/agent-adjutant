//! Starting the readers that are next in line, and failing the ones that died, on the board's
//! own clock: a read waits for the hub to come up, and a reader that crashed holds its place
//! until something notices. Each pass then closes the readers of the tasks that are finished,
//! and the place they held is filled by the next pass.

use std::path::PathBuf;
use std::time::Duration;

use crate::lifecycle::review::{close_task_readers, dispatch};

/// How often the queue is looked at. A pass with nothing queued or running reads the records
/// and no more.
pub const REVIEW_QUEUE_EVERY: Duration = Duration::from_secs(3);

/// Work the queue for as long as the process lives. What a pass could not do is said on stderr,
/// which is the resident's log, once for each different thing: the same failure every three
/// seconds would bury the rest.
pub fn run(root: PathBuf) {
    let mut said: Option<String> = None;
    let mut said_close: Option<String> = None;
    loop {
        // A round that panics is a failed round, not the end of the queue, as in `sweep_gates`.
        // Two rounds, so a panic in closing readers never keeps the queue from being worked.
        let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(&root)));
        let error = pass.ok().and_then(|done| done.error);
        if error != said {
            if let Some(why) = &error {
                eprintln!("adj server: the queue of readers could not be worked: {why}");
            }
            said = error;
        }
        // After the queue, so a reader that is slow to stop never delays starting the reads; a
        // place this frees is filled by the next pass.
        let closing =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| close_task_readers(&root)))
                .unwrap_or_default();
        // A reader that cannot be told alive is tried again every pass: it is said when the set
        // of failures changes, not on each of them.
        let closing_said = (!closing.is_empty()).then(|| {
            closing
                .iter()
                .map(|(id, why)| format!("{id}: {why}"))
                .collect::<Vec<_>>()
                .join("\n")
        });
        if closing_said != said_close {
            for (id, why) in &closing {
                if id.is_empty() {
                    eprintln!("adj server: the readers could not be closed: {why}");
                } else {
                    eprintln!("adj server: the reader of {id} could not be closed: {why}");
                }
            }
            said_close = closing_said;
        }
        std::thread::sleep(REVIEW_QUEUE_EVERY);
    }
}
