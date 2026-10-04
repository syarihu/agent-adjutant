//! The lock that makes counting the slots and taking one a single step.

use super::*;

/// How long a dispatch waits for another to finish counting before it gives up.
const DISPATCH_LOCK_SECS: u64 = 10;

/// Run `f` while holding this checkout's dispatch lock.
///
/// Counting the slots and marking one taken are two steps, and two `adj work` run side by
/// side — a hub batching its shell calls, or two hubs on one repository — would both count
/// before either marked, and both start. The lock makes the pair one step.
///
/// An advisory lock on an open file, for the reason `take_over` gives: a lock made of a
/// file's existence has to guess when its holder died, and two callers guessing at once both
/// get in. This one is released by the system when its holder exits, so a killed dispatch
/// leaves nothing to clear. Waiting ends in an error rather than in taking the lock anyway.
pub fn with_dispatch_lock<T>(main: &Path, f: impl FnOnce() -> T) -> Result<T, String> {
    let path = main.join(".claude").join("adjutant-dispatch.lock");
    // Never unlinked: removing it while another process holds it open would hand the next
    // two callers two different locks.
    let lock = crate::infra::fs::open_lock(&path)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(DISPATCH_LOCK_SECS);
    loop {
        if crate::infra::fs::try_hold(&lock, &path)? {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "another dispatch has held {} for {DISPATCH_LOCK_SECS}s; try again",
                path.display()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let answer = f();
    drop(lock);
    Ok(answer)
}
