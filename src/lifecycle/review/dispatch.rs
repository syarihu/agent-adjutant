use std::path::Path;

use crate::infra::clock::now_secs;
use crate::lifecycle::review::launch_reader;
use crate::others::{self, Claim};
use crate::registry::{self, Context, Liveness};

/// What one pass of the queue did, by record id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dispatched {
    /// Reads failed because their reader was gone.
    pub reaped: Vec<String>,
    /// Reads whose reader was started.
    pub started: Vec<String>,
    /// Reads that were claimed and could not be started.
    pub failed: Vec<String>,
    /// Why the pass could not be made at all: the bare reason, which a transport words.
    pub error: Option<String>,
}

/// Take one step on the queue of readers: fail the ones that died, then start as many of the
/// next in line as there are free places (at most three run). Does nothing, and runs no
/// process, while no read is queued or running. Nothing is started while the code-review hub is
/// not up (a reader reports to the hub, and queued reads wait for it), but a dead reader is
/// still failed.
pub fn dispatch(root: &Path) -> Dispatched {
    // Before the config is read or a process is asked about: the job comes every few seconds, and
    // with nothing queued or running this must cost one small read per record and no more.
    if !has_work(root) {
        return Dispatched::default();
    }
    let ctx = match registry::review_context(root) {
        Ok(ctx) => ctx,
        Err(e) => {
            return Dispatched {
                error: Some(e),
                ..Dispatched::default()
            };
        }
    };
    dispatch_with(
        &ctx,
        now_secs(),
        &|| registry::hub_status(&ctx.state, &ctx.repo.slug, &ctx.repo.hub_name).present,
        &registry::hub_process_liveness,
        &registry::new_session_id,
        &launch_reader,
    )
}

/// Whether any read is queued or running.
fn has_work(root: &Path) -> bool {
    others::list(root).iter().any(|r| {
        r.ai.as_ref()
            .is_some_and(|ai| matches!(ai.status.as_str(), "queued" | "running"))
    })
}

/// `dispatch`, with what it asks the machine handed in: whether the hub is up, whether a pid is
/// alive, a new session id, and how a reader is started.
pub(crate) fn dispatch_with(
    ctx: &Context,
    now: i64,
    hub_up: &dyn Fn() -> bool,
    alive: &dyn Fn(u32, Option<&str>) -> Liveness,
    new_session: &dyn Fn() -> Result<String, String>,
    launch: &dyn Fn(&Context, &Claim) -> Result<(), String>,
) -> Dispatched {
    let root = ctx.state.as_path();
    if !has_work(root) {
        return Dispatched::default();
    }
    // A reader that died frees its place whether or not the hub is up: only starting one needs it.
    let mut done = Dispatched {
        reaped: others::reap_reads(root, now, alive),
        ..Dispatched::default()
    };
    if !hub_up() {
        return done;
    }
    let claims = match others::claim_reads(root, now, new_session) {
        Ok(claims) => claims,
        Err(e) => {
            done.error = Some(e);
            return done;
        }
    };
    // Every lock is let go of by now: starting a reader opens a window and waits to see it
    // stay, and a queue that held its lock across that would hold up everyone asking.
    for claim in claims {
        match launch(ctx, &claim) {
            Ok(()) => done.started.push(claim.id),
            Err(why) => {
                if let Some(pr) = others::pr_of(&claim.id) {
                    let _ = others::fail_read(root, &pr, Some(&claim.session_id), &why, now);
                }
                done.failed.push(claim.id);
            }
        }
    }
    done
}
