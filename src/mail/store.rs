use super::*;

/// Where messages for this hub wait. One directory, whether or not the hub is running: two
/// would mean the hub has to remember to read both, and the one it forgets is the one that
/// silently swallows reports.
pub fn inbox_dir(slug: &str) -> PathBuf {
    state_dir().join("inbox").join(slug)
}

/// Where a message goes once the hub has acted on it. Kept rather than deleted so a report
/// that was mishandled can still be found.
pub fn archive_dir(slug: &str) -> PathBuf {
    inbox_dir(slug).join("read")
}

/// Where the hub leaves messages for the worker. One markdown file rather than one file per
/// message, because the worker reads it with its eyes as often as with a tool.
pub fn outbox_path(worktree: &Path) -> PathBuf {
    worktree.join(".claude").join("adjutant-outbox.md")
}

pub(super) fn hold(inbox: &Path, name: &str) -> Result<PathBuf, String> {
    for attempt in 0..CLAIM_ATTEMPTS {
        let path = inbox.join(format!("{HOLDING}{}-{attempt}-{name}", std::process::id()));
        match crate::infra::fs::create_new(&path) {
            Ok(true) => return Ok(path),
            Ok(false) => continue,
            Err(e) => return Err(format!("cannot write {}: {e}", path.display())),
        }
    }
    Err(format!(
        "cannot hold a message in {} after {CLAIM_ATTEMPTS} tries",
        inbox.display()
    ))
}

/// Put back anything an interrupted ack left holding.
///
/// Without this, a process that died between taking a message and filing it left the only
/// copy under a name nothing lists, nothing reads and nothing acks — a report that exists
/// and cannot be reached, which is worse than the overwrite this whole arrangement replaced.
/// Age is what distinguishes an abandoned hold from one in flight, and the margin is wide:
/// an ack holds a message for two syscalls.
pub(super) fn put_back_abandoned(inbox: &Path) {
    let Ok(read) = std::fs::read_dir(inbox) else {
        return;
    };
    for entry in read.flatten() {
        let held = entry.path();
        let Some(rest) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.strip_prefix(HOLDING).map(str::to_string))
        else {
            continue;
        };
        // `<pid>-<attempt>-<the name it came from>`.
        let Some(name) = rest.splitn(3, '-').nth(2).map(str::to_string) else {
            continue;
        };
        let too_old = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|at| at.elapsed().map(|d| d.as_secs()).unwrap_or(0) > HELD_STALE_SECS)
            .unwrap_or(false);
        if !too_old {
            continue;
        }
        if claim_link(&held, inbox, |seq| numbered(&name, seq)).is_ok() {
            let _ = std::fs::remove_file(&held);
        }
    }
}

/// Give `staged` a second name in `dir`, the first one `name_for` offers that is free.
///
/// Two properties have to hold at once, and one primitive gives both. `hard_link` refuses
/// an existing name instead of replacing it, so two senders racing for the same second
/// cannot both win — which `exists()` followed by a write cannot promise, because the
/// answer is already stale by the time it is acted on. And because the name being claimed
/// points at a file that is *already written in full*, nobody can read half a message.
///
/// `rename` would give the second property and lose the first: it replaces silently, which
/// is exactly the overwrite being ruled out here. So the staged file gets linked, not moved,
/// and the caller unlinks the staging name afterwards.
pub(super) fn claim_link(
    staged: &Path,
    dir: &Path,
    name_for: impl Fn(usize) -> String,
) -> Result<PathBuf, String> {
    for seq in 0..CLAIM_ATTEMPTS {
        let path = dir.join(name_for(seq));
        match std::fs::hard_link(staged, &path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write {}: {e}", path.display())),
        }
    }
    Err(format!(
        "cannot find an unused name in {} after {CLAIM_ATTEMPTS} tries",
        dir.display()
    ))
}
