use crate::kernel::identity::{self, RepoInfo};

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
