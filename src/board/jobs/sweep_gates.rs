//! Closing the gates whose worker has visibly moved on, on the board's own clock rather than
//! on a read of `/api/state`.

use std::time::Duration;

use crate::gate;
use crate::registry::Context;

/// The pace of the page's poll, so a resumed gate leaves the board about as soon as it did
/// when reading the state closed it.
pub const SWEEP_EVERY: Duration = Duration::from_secs(2);

/// Sweep for as long as the process lives. `boards` is asked each round, so a board the
/// address book gained since is swept too.
pub fn run(boards: impl Fn() -> Vec<Context>) {
    loop {
        // A round that panics is a failed round, not the end of the sweep, as in `PrPoll::run`.
        // Silent otherwise, as `close_resumed` asks: in a hub's MCP process stderr is the
        // client's log.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for ctx in boards() {
                let _ = gate::close_resumed(&ctx);
            }
        }));
        std::thread::sleep(SWEEP_EVERY);
    }
}
