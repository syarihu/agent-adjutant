//! Claiming the right to start the aide.

use super::store::{aide_lock_path, aide_record_path};
use super::*;
use crate::infra::fs::lock;
use serde_json::Map;

/// The answer to "may an aide be started?".
#[derive(Debug, PartialEq)]
pub enum AideClaim {
    /// Nobody holds it: a new record names the session to start, and nothing runs under it yet.
    Claimed(AideRecord),
    /// An aide is running. Nothing was written.
    Running(AideRecord),
    /// One was claimed a moment ago and has not reported its process yet. Nothing was written.
    Starting(AideRecord),
}

/// Claim a new aide session unless one is already running or starting. `Err` when that
/// cannot be told (`ps` failed, or the record cannot be read): a second aide is not started
/// on a guess.
pub fn claim_aide(root: &Path, now: i64) -> Result<AideClaim, String> {
    claim_aide_with(root, now, &new_session_id, &hub_process_liveness)
}

/// `claim_aide`, with the new id and the process check handed in.
pub(crate) fn claim_aide_with(
    root: &Path,
    now: i64,
    new_id: &dyn Fn() -> Result<String, String>,
    liveness: &dyn Fn(u32, Option<&str>) -> Liveness,
) -> Result<AideClaim, String> {
    let _lock = lock(&aide_lock_path(root))?;
    let path = aide_record_path(root);
    match super::aide_record(root) {
        Recorded::Unreadable => Err(format!(
            "the aide record {} cannot be read; remove it to start a new one",
            path.display()
        )),
        Recorded::Found(record) => match aide_state_with(&record, now, liveness) {
            AideState::Running => Ok(AideClaim::Running(record)),
            AideState::Starting => Ok(AideClaim::Starting(record)),
            AideState::CannotTell => Err(format!(
                "cannot tell whether the aide (pid {}) is still running",
                record.pid.map(|p| p.to_string()).unwrap_or_default()
            )),
            AideState::Gone => fresh(&path, now, new_id),
        },
        Recorded::Absent => fresh(&path, now, new_id),
    }
}

fn fresh(
    path: &Path,
    now: i64,
    new_id: &dyn Fn() -> Result<String, String>,
) -> Result<AideClaim, String> {
    let record = AideRecord {
        session_id: new_id()?,
        launched_at: now,
        pid: None,
        ps_started: None,
        terminal: None,
        other: Map::new(),
    };
    write_json(path, &record)?;
    Ok(AideClaim::Claimed(record))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10_000;

    fn write(root: &Path, record: &AideRecord) {
        write_json(&aide_record_path(root), record).unwrap();
    }

    fn record(session: &str, launched_at: i64, pid: Option<u32>) -> AideRecord {
        AideRecord {
            session_id: session.to_string(),
            launched_at,
            pid,
            ps_started: None,
            terminal: None,
            other: Map::new(),
        }
    }

    fn id(text: &'static str) -> impl Fn() -> Result<String, String> {
        move || Ok(text.to_string())
    }

    fn alive(_: u32, _: Option<&str>) -> Liveness {
        Liveness::Alive
    }

    fn gone(_: u32, _: Option<&str>) -> Liveness {
        Liveness::Gone
    }

    #[test]
    fn no_record_is_claimed_and_written() {
        let dir = tempfile::tempdir().unwrap();
        let claim = claim_aide_with(dir.path(), NOW, &id("new"), &alive).unwrap();
        assert_eq!(claim, AideClaim::Claimed(record("new", NOW, None)));
        let Recorded::Found(written) = aide_record(dir.path()) else {
            panic!("the claim wrote nothing");
        };
        assert_eq!(written.session_id, "new");
    }

    #[test]
    fn a_live_process_is_running_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let pid = std::process::id();
        let started = ps_started(pid);
        let mut held = record("old", 1, Some(pid));
        held.ps_started = started;
        write(dir.path(), &held);
        let claim = claim_aide_with(dir.path(), NOW, &id("new"), &hub_process_liveness).unwrap();
        assert_eq!(claim, AideClaim::Running(held.clone()));
        assert_eq!(aide_record(dir.path()), Recorded::Found(held));
    }

    #[test]
    fn a_process_that_has_ended_is_replaced_by_a_new_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        write(dir.path(), &record("old", 1, Some(pid)));
        let claim = claim_aide_with(dir.path(), NOW, &id("new"), &hub_process_liveness).unwrap();
        assert_eq!(claim, AideClaim::Claimed(record("new", NOW, None)));
    }

    #[test]
    fn no_pid_is_starting_within_the_grace_and_replaced_after() {
        let dir = tempfile::tempdir().unwrap();
        let fresh_launch = record("old", NOW - 10, None);
        write(dir.path(), &fresh_launch);
        let claim = claim_aide_with(dir.path(), NOW, &id("new"), &gone).unwrap();
        assert_eq!(claim, AideClaim::Starting(fresh_launch));
        write(
            dir.path(),
            &record("old", NOW - AIDE_START_GRACE_SECS, None),
        );
        let claim = claim_aide_with(dir.path(), NOW, &id("new"), &gone).unwrap();
        assert_eq!(claim, AideClaim::Claimed(record("new", NOW, None)));
    }

    #[test]
    fn an_unreadable_record_and_a_pid_nobody_can_check_both_refuse() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(aide_record_path(dir.path()), "{").unwrap();
        let err = claim_aide_with(dir.path(), NOW, &id("new"), &alive).unwrap_err();
        assert!(err.contains("cannot be read"), "{err}");
        write(dir.path(), &record("old", 1, Some(4242)));
        let err =
            claim_aide_with(dir.path(), NOW, &id("new"), &|_, _| Liveness::CannotTell).unwrap_err();
        assert!(err.contains("cannot tell"), "{err}");
        // Neither refusal changed the record.
        assert_eq!(
            aide_record(dir.path()),
            Recorded::Found(record("old", 1, Some(4242)))
        );
    }
}
