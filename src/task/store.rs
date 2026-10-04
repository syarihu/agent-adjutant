//! Where the task records are kept, and the clock they are stamped with.

use super::*;

use crate::registry::Context;

/// Where this hub's tasks live. Beside the inbox rather than inside it: the inbox is a
/// queue that drains, and a task record has to still be there after its message is acked.
pub(crate) fn dir(state_dir: &Path, slug: &str) -> PathBuf {
    state_dir.join("tasks").join(slug)
}

pub(super) fn path_of(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// The directory of the hub `ctx` is for.
pub(super) fn dir_of(ctx: &Context) -> PathBuf {
    dir(&ctx.state, &ctx.repo.slug)
}

/// Take an id nobody else holds, and hold it.
///
/// The stamp has one-second resolution and a Japanese title slugs to nothing, so two tasks
/// written in the same second are not a rare case — it is what filling the form twice looks
/// like, and the second one would land on the first one's file and erase it.
///
/// Claimed with `create_new` rather than checked with `exists` first: the dashboard and the
/// command line can both be creating one, and check-then-write leaves a window where both
/// see the name free. `mail::send` names inbox files the same way, for the same reason.
pub fn claim_id(ctx: &Context, stamp: &str, title: &str) -> Result<String, String> {
    let dir = &dir_of(ctx);
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let base = new_id(stamp, title);
    for seq in 1..1000 {
        let id = if seq == 1 {
            base.clone()
        } else {
            format!("{base}-{seq}")
        };
        match crate::infra::fs::create_new(&path_of(dir, &id)) {
            Ok(true) => return Ok(id),
            Ok(false) => continue,
            Err(e) => {
                return Err(format!(
                    "cannot create a task file in {}: {e}",
                    dir.display()
                ));
            }
        }
    }
    Err(format!("no free task id for {base}"))
}

/// Write the record, creating the directory if this is the first one.
///
/// Written whole each time rather than patched: every caller already holds the struct it
/// wants on disk, and a partial write is how two writers end up with a record neither of
/// them would recognise.
///
/// Staged as a dotfile beside the record, synced and renamed over it, because the resident
/// server's PR poll writes records while `adj task show` and the board read them: a plain
/// write truncates first, and a reader in between sees an empty file. `list` skips the
/// staged name, so it never picks one up.
pub fn save(ctx: &Context, task: &Task) -> Result<PathBuf, String> {
    let path = path_of(&dir_of(ctx), &task.id);
    crate::infra::fs::write_json(&path, task)?;
    Ok(path)
}

pub(super) fn load(root: &Path, slug: &str, id: &str) -> Result<Task, String> {
    if !is_plain_id(id) {
        return Err(format!("no such task: {id}"));
    }
    let path = path_of(&dir(root, slug), id);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no such task: {id}"))?;
    serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Every task this hub knows about, queue order first.
///
/// A file that will not parse is skipped rather than fatal. The board is a view of a
/// directory somebody may have hand-edited, and one bad file must not blank the page.
pub(super) fn list(root: &Path, slug: &str) -> Vec<Task> {
    let mut tasks: Vec<Task> = match std::fs::read_dir(dir(root, slug)) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|text| serde_json::from_str::<Task>(&text).ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    tasks.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });
    tasks
}

/// Hold the write lock of one task record until the returned handle is dropped.
///
/// A change is a read of the whole record and a write of the whole record, and two of them
/// at once — the hub updating a task while a gate for it is answered on the board — would
/// each write back what they read, and the later would undo the earlier. An advisory lock
/// on an open file, like the dispatch lock: the system lets it go if its holder dies, so
/// there is nothing to clear by hand. Held only across a load and a save, so waiting on it
/// is short.
pub fn lock(ctx: &Context, id: &str) -> Result<std::fs::File, String> {
    if !is_plain_id(id) {
        return Err(format!("no such task: {id}"));
    }
    // Beside the record, and not named `.json`, so the listing never reads it as a task.
    crate::infra::fs::lock(&dir_of(ctx).join(format!("{id}.lock")))
}

pub fn stamp() -> String {
    crate::infra::clock::utc_stamp(crate::infra::clock::now_secs())
}
