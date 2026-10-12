//! Starting the readers that are next in line, and failing the ones that died, on the board's
//! own clock: a read waits for the hub to come up, and a reader that crashed holds its place
//! until something notices.

use std::path::PathBuf;
use std::time::Duration;

use crate::lifecycle::review::dispatch;

/// How often the queue is looked at. A pass with nothing queued or running reads the records
/// and no more.
pub const REVIEW_QUEUE_EVERY: Duration = Duration::from_secs(3);

/// Work the queue for as long as the process lives. What a pass could not do is said on stderr,
/// which is the resident's log, once for each different thing: the same failure every three
/// seconds would bury the rest.
pub fn run(root: PathBuf) {
    let mut said: Option<String> = None;
    loop {
        // A round that panics is a failed round, not the end of the queue, as in `sweep_gates`.
        let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(&root)));
        let error = pass.ok().and_then(|done| done.error);
        if error != said {
            if let Some(why) = &error {
                eprintln!("adj server: the queue of readers could not be worked: {why}");
            }
            said = error;
        }
        std::thread::sleep(REVIEW_QUEUE_EVERY);
    }
}
