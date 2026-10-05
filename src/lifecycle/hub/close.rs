use std::path::Path;

use crate::kernel::identity::{self, RepoInfo};
use crate::registry;

/// Why `hub` cannot be closed, or `Ok` when it can. Only a parent-task hub none of whose
/// checkouts report to it any more is closable: the repository hub is always there, and a
/// hub with workers is still in use. Unread messages, open tasks and gates do not stop it —
/// they are kept, and starting the same key again finds them.
pub fn closable_check(repo: &RepoInfo, hub: &crate::mail::RepoHub) -> Result<(), String> {
    if !hub.parent {
        return Err("the repository hub can only be stopped, not closed; use hub-stop".to_string());
    }
    // The list is lenient about checkouts it cannot read; closing must not be.
    identity::linked_worktrees(&repo.main)
        .map_err(|e| format!("cannot tell which checkouts report to {}: {e}", hub.name))?;
    if hub.children > 0 {
        return Err(format!(
            "{} checkout(s) still report to {}; use hub-stop to stop it, or clean them up first",
            hub.children, hub.name
        ));
    }
    Ok(())
}

/// What closing a hub left behind that its owner may want to hear about.
pub struct Closed {
    /// Unread messages still in the hub's inbox: closing keeps them.
    pub unread: usize,
}

/// Close `hub`: clear its record and take it off the board's address book, so it drops out
/// of the list. Ends no process, and says nothing: the command line and the board each answer
/// in their own words.
///
/// Both ask for the same thing, so the same checks stand in front of it: `closable_check`,
/// then a refusal for a hub that is still running, then clearing only the record that was
/// looked at.
pub fn close(root: &Path, repo: &RepoInfo, hub: &crate::mail::RepoHub) -> Result<Closed, String> {
    closable_check(repo, hub)?;
    // This ends no process, so a hub that is still running would be left running with no
    // record, and the next `adj hub` would start a second one beside it. Only the hub itself
    // may clear its own record; from anywhere else it has to be stopped first.
    let recorded = registry::read_hub_record(root, &hub.slug);
    let named = match &recorded {
        registry::Recorded::Found(r) => r.pid.map(|pid| (pid as u32, r.ps_started.clone())),
        _ => None,
    };
    if let Some((pid, started)) = &named {
        let pid = *pid;
        match registry::hub_process_liveness(pid, started.as_deref()) {
            registry::Liveness::Gone => {}
            registry::Liveness::CannotTell => {
                return Err(registry::hub_cannot_tell(root, &hub.slug));
            }
            registry::Liveness::Alive => {
                if !registry::is_self_or_descendant_of(pid) {
                    return Err(format!(
                        "{} is still running (pid {pid}); close it from the board, or stop it first \
                         (`adj hub-stop` from inside it, or the board's stop) and then close it",
                        hub.name
                    ));
                }
            }
        }
    } else if matches!(recorded, registry::Recorded::Unreadable) {
        // A record that is there and cannot be read is asked the way `stop_hub` asks, and
        // nobody can be told to be the hub itself. A readable one that names no process is
        // `Gone` to `hub_liveness`, so it goes straight on to be cleared.
        match registry::hub_liveness(root, &hub.slug) {
            registry::Liveness::Gone => {}
            registry::Liveness::CannotTell => {
                return Err(registry::hub_cannot_tell(root, &hub.slug));
            }
            registry::Liveness::Alive => {
                return Err(format!(
                    "{} is still running; close it from the board, or stop it first \
                     (`adj hub-stop` from inside it, or the board's stop) and then close it",
                    hub.name
                ));
            }
        }
    }
    // Under the claim lock and only while the record is still the one that was looked at:
    // a hub that registered since is not this call's to unregister.
    let removed = match &named {
        Some((pid, started)) => {
            registry::unregister_hub_if(root, &hub.slug, *pid, started.as_deref())?
        }
        None => registry::unregister_hub_if_unnamed(root, &hub.slug)?,
    };
    if !removed {
        return Err(format!("{} changed while it was being closed", hub.name));
    }
    registry::forget_board(root, &hub.slug)?;
    Ok(Closed {
        unread: hub.inbox_count,
    })
}
