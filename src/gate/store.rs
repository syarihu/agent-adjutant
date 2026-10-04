//! Where the gates are kept, the raw reads and writes behind the typed ones, and the clock they are stamped with.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::*;

use crate::registry::Context;

pub(super) fn dir(root: &Path, slug: &str, shelf: Shelf) -> PathBuf {
    let open = root.join("gates").join(slug);
    match shelf {
        Shelf::Open => open,
        Shelf::Record => open.join("records"),
        Shelf::Answered => open.join("answered"),
    }
}

pub(super) fn dir_of(ctx: &Context, shelf: Shelf) -> PathBuf {
    dir(&ctx.state, &ctx.repo.slug, shelf)
}

pub(super) fn path_of(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// Whether a gate file is on `shelf`, without reading it. Lets a caller tell a missing gate
/// from one that is there and broken, which `load` reports the same way.
pub(super) fn exists(root: &Path, slug: &str, shelf: Shelf, id: &str) -> bool {
    path_of(&dir(root, slug, shelf), id).exists()
}

/// `20260922T041233Z-diff`, and a suffix if that name is taken.
///
/// Claimed rather than checked, like a task id and an inbox filename: a worker finishing two
/// pieces of work in the same second is ordinary, and the loser of a check-then-write would
/// overwrite a gate somebody is in the middle of reading.
///
/// A record's id ends in `-record` (`20260922T041233Z-diff-record`). Named apart from an open
/// gate's id rather than claimed on both shelves: an answer finds its gate by id alone, and a
/// record and a gate opened in the same second must not be mistaken for each other.
pub(super) fn claim_id(
    ctx: &Context,
    shelf: Shelf,
    stamp: &str,
    kind: Kind,
) -> Result<String, String> {
    let base = if shelf == Shelf::Record {
        format!("{stamp}-{}-record", kind.as_str())
    } else {
        format!("{stamp}-{}", kind.as_str())
    };
    claim(&dir_of(ctx, shelf), base)
}

fn claim(dir: &Path, base: String) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
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
                    "cannot create a gate file in {}: {e}",
                    dir.display()
                ));
            }
        }
    }
    Err(format!("no free gate id for {base}"))
}

/// Write a gate, replacing whatever is at its name in one step.
///
/// A record is written again each time it is answered, while the board reads it every few
/// seconds and nothing it reads through takes the writer's lock. Written in place, a reader
/// could catch it half-written and drop it from the listing, and a write cut short would
/// leave it unreadable for good. Staged as a dotfile beside it, synced and renamed over it,
/// the name never points at a partial file.
pub(super) fn save(ctx: &Context, shelf: Shelf, gate: &Gate) -> Result<PathBuf, String> {
    let path = path_of(&dir_of(ctx, shelf), &gate.id);
    crate::infra::fs::write_json(&path, gate)?;
    Ok(path)
}

pub(super) fn load(root: &Path, slug: &str, shelf: Shelf, id: &str) -> Result<Gate, String> {
    let path = path_of(&dir(root, slug, shelf), id);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no open gate: {id}"))?;
    serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// The gates in `dir` whose file was written at or after `since`, oldest first. For a caller
/// that can only care about gates opened after some moment: a gate's file is written after it
/// is opened, so an older file cannot be one, and the directory's history is not parsed.
/// Only prunes: a caller still checks what it needs of each gate it gets.
pub(super) fn list_modified_since(dir: &Path, since: SystemTime) -> Vec<Gate> {
    let mut gates: Vec<Gate> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter(|e| {
                e.metadata()
                    .and_then(|m| m.modified())
                    .is_ok_and(|modified| modified >= since)
            })
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|text| serde_json::from_str::<Gate>(&text).ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    gates.sort_by(|a, b| a.opened_at.cmp(&b.opened_at));
    gates
}

pub(super) fn list_where(dir: &Path, wanted: impl Fn(&str) -> bool) -> Vec<Gate> {
    let mut gates: Vec<Gate> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter(|e| {
                e.path()
                    .file_stem()
                    .is_some_and(|stem| wanted(&stem.to_string_lossy()))
            })
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|text| serde_json::from_str::<Gate>(&text).ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    gates.sort_by(|a, b| a.opened_at.cmp(&b.opened_at));
    gates
}

/// Move an answered gate out of the way, so "open" means what it says.
///
/// Written through `write_json` like `save`: the board reads `answered/` on every poll, so it
/// must never see a half-written file there.
pub(super) fn archive(ctx: &Context, gate: &Gate) -> Result<PathBuf, String> {
    let path = path_of(&dir_of(ctx, Shelf::Answered), &gate.id);
    crate::infra::fs::write_json(&path, gate)?;
    let _ = std::fs::remove_file(path_of(&dir_of(ctx, Shelf::Open), &gate.id));
    Ok(path)
}

/// Hold the write lock of one gate or record until the returned handle is dropped. The same
/// advisory lock `task::lock` takes, for the same reason: appending an answer is a read
/// and a write of the whole file, and two at once would each write back what they read. On
/// an open gate it is what lets only one of the board's answer, the worker's close and the
/// board's own sweep decide it.
pub(super) fn lock(ctx: &Context, shelf: Shelf, id: &str) -> Result<std::fs::File, String> {
    crate::infra::fs::lock(&lock_path(&dir_of(ctx, shelf), id))
}

/// `lock` without waiting: `None` when somebody else holds it.
pub(super) fn try_lock(
    ctx: &Context,
    shelf: Shelf,
    id: &str,
) -> Result<Option<std::fs::File>, String> {
    crate::infra::fs::try_lock(&lock_path(&dir_of(ctx, shelf), id))
}

/// Not named `.json`, so the listing never reads it as a gate. Never removed, for the reason
/// given at `with_dispatch_lock`: a lock file that is unlinked can be locked twice.
fn lock_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.lock"))
}

pub(super) fn stamp() -> String {
    crate::infra::clock::utc_stamp(crate::infra::clock::now_secs())
}
