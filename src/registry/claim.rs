//! Claiming a hub's name: one hub per address, and the takeover of a dead one.

use super::store::hub_record_path;
use super::*;
use serde_json::Map;

/// Record this process as the hub for `slug`, unless another live hub already is.
///
/// Called by `adj hub`, which then `exec`s the agent — so the PID stays valid across
/// the handover and the record points at the live agent process rather than at a launcher
/// that has already exited.
///
/// "One hub per repository" is the whole point of the record, and asking `hub_status`
/// first and writing second cannot enforce it: both askers hear "nobody home" and both
/// write. The record is therefore *claimed* — created with `create_new`, which the
/// filesystem refuses for the loser — and only a claim that was refused goes on to ask
/// whose it is.
///
/// What the loser asks is deliberately not `hub_status`. A hub that has just won the claim
/// has not `exec`ed the agent yet, so for a moment its command line is still the launcher's
/// and the name is not in it — `hub_status` calls that stale, and clearing a "stale" record
/// and retrying is the same check-then-act this is here to remove, arrived at from the
/// other side. The only question that can be asked safely is the one that cannot be
/// mid-change: is the recorded process still the process that was recorded? Alive with the
/// same start time means someone holds the name, whether or not they look like a hub yet.
/// A few retries, not a thousand: each one is two calls to `ps`, and a record that keeps
/// coming back means someone else keeps winning it.
pub fn claim_hub(
    root: &Path,
    slug: &str,
    hub_name: &str,
    cwd: &str,
    name_in_command: bool,
    hub: Option<&str>,
    terminal: Option<&crate::infra::terminal::SessionTerminal>,
) -> Result<Claim, String> {
    let path = hub_record_path(root, slug);
    // `terminal` is where this hub runs, so that something outside its tab — the board's stop
    // button — can find its pane without guessing from the pid. Absent for a record written
    // before this, which readers fall back from.
    let record = HubRecord {
        pid: Some(u64::from(std::process::id())),
        hub_name: Some(hub_name.to_string()),
        cwd: Some(cwd.to_string()),
        started_at: Some(utc_stamp(now_secs())),
        ps_started: ps_started(std::process::id()),
        hub: said(hub),
        name_in_command: Some(name_in_command),
        terminal: terminal.cloned(),
        other: Map::new(),
    }
    .to_value();
    match create_new_json(&path, &record) {
        Ok(()) => return Ok(Claim::Ours),
        Err(CreateError::Taken) => {}
        Err(CreateError::Failed(message)) => return Err(message),
    }
    match holder(&path) {
        Liveness::Alive => return Ok(Claim::Taken(Box::new(hub_status(root, slug, hub_name)))),
        Liveness::CannotTell => return Err(cannot_tell(&path)),
        Liveness::Gone => {}
    }

    // The record belongs to a process that has gone, and taking it over means deleting a
    // file and creating it again — two steps, which two launchers can interleave: the
    // second delete removes the *first one's live record* and the name is handed out
    // twice. Neither `create_new` nor `rename` prevents that, because the second launcher
    // is acting on a name whose contents changed underneath it, and POSIX has no "remove
    // this file only if it is still the one I looked at".
    //
    // So the takeover — and only the takeover — is serialised. The lock is a file nobody
    // can create twice, it names who holds it, and it is held for the few syscalls between
    // "this record is dead" and "this record is mine". A lock left behind by a crash goes
    // stale on a clock, which is safe here in a way it would never be for the hub record
    // itself: this one is held for microseconds, so an old one is evidence, not a guess.
    take_over(root, &path, &record, slug, hub_name)
}

fn take_over(
    root: &Path,
    path: &Path,
    record: &Value,
    slug: &str,
    hub_name: &str,
) -> Result<Claim, String> {
    // An advisory lock held on an open file, not a file whose existence is the lock.
    //
    // A lock made of a file has to answer "what if its holder died holding it", and every
    // answer to that is a guess — a clock, a pid, another liveness check — and every guess
    // is check-then-act again, one level down: two launchers both decide a lock is stale,
    // both break it, and both are inside. This one is released by the operating system when
    // the process ends, however it ends, so there is no stale case to reason about. The
    // file itself is never removed: unlinking it while another process holds it open would
    // hand the next two callers two different locks.
    let lock_path = path.with_extension("claiming");
    // Someone else is part-way through taking this name. Whatever they end up with, it is
    // not ours — the same answer we would have been given by arriving after they finished.
    let Some(lock) = crate::infra::fs::try_lock(&lock_path)? else {
        return Ok(Claim::Taken(Box::new(hub_status(root, slug, hub_name))));
    };

    // Asked again inside the lock: the record may have been taken over while we were
    // getting in, and the answer from outside is the one that was about to go stale.
    let claimed = match holder(path) {
        Liveness::Alive => Ok(Claim::Taken(Box::new(hub_status(root, slug, hub_name)))),
        Liveness::CannotTell => Err(cannot_tell(path)),
        Liveness::Gone => {
            remove_if_present(path)?;
            match create_new_json(path, record) {
                Ok(()) => Ok(Claim::Ours),
                Err(CreateError::Taken) => {
                    Ok(Claim::Taken(Box::new(hub_status(root, slug, hub_name))))
                }
                Err(CreateError::Failed(message)) => Err(message),
            }
        }
    };
    drop(lock);
    claimed
}

fn cannot_tell(path: &Path) -> String {
    format!(
        "cannot tell whether the hub recorded in {} is still running, so its name is left alone",
        path.display()
    )
}

/// Why a hub's name was left alone, in the words `claim_hub` refuses with.
///
/// A caller that asks `hub_liveness` first is refusing on the claim's behalf, so it says
/// what the claim would have said — same record named, same thing to go and look at.
pub fn hub_cannot_tell(root: &Path, slug: &str) -> String {
    cannot_tell(&hub_record_path(root, slug))
}
