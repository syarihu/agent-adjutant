//! The HTML report a gate can carry: read from the worker's worktree when the gate opens,
//! and kept beside the gates so the board can show it after the worktree is gone.

use std::io::Read;
use std::path::{Path, PathBuf};

use super::store::dir;
use super::*;

use crate::registry::Context;

/// The largest report taken. A report is one self-contained page, images inlined, and the
/// board holds it in memory to hand to a frame; this is room for a long analysis with a few
/// pictures, and not for a file somebody pointed at by mistake.
pub const MAX_REPORT_BYTES: u64 = 5 * 1024 * 1024;

/// Whether `id` can name a report copy. The server builds a path from the id in a URL, and
/// nothing but letters, digits and `-` (what a gate id is made of) keeps `..` and `/` out of it.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Where the copy of gate `id`'s report lives. Under the open shelf's directory whatever
/// shelf the gate is on now, so that answering or archiving the gate does not have to move it.
fn path_of(root: &Path, slug: &str, id: &str) -> PathBuf {
    dir(root, slug, Shelf::Open)
        .join("reports")
        .join(format!("{id}.html"))
}

/// Read the file a request names, resolved against the worktree, refusing anything the board
/// could not show. Every refusal names the path, so a worker can see which file it was.
pub(super) fn read(worktree: &str, given: &str) -> Result<String, String> {
    let path = Path::new(worktree).join(given);
    let shown = path.display();
    let meta = std::fs::metadata(&path).map_err(|e| format!("cannot read report {shown}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("report {shown} is not a regular file"));
    }
    // Refused from the metadata, before reading a byte: a file of any size can be named.
    if meta.len() > MAX_REPORT_BYTES {
        return Err(format!(
            "report {shown} is {} bytes, over the {MAX_REPORT_BYTES} byte limit",
            meta.len()
        ));
    }
    let file =
        std::fs::File::open(&path).map_err(|e| format!("cannot read report {shown}: {e}"))?;
    // The file can grow between the check and the read, so the read is bounded as well.
    let mut bytes = Vec::new();
    file.take(MAX_REPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read report {shown}: {e}"))?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
        return Err(format!(
            "report {shown} is over the {MAX_REPORT_BYTES} byte limit"
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("report {shown} is not UTF-8 text"))
}

/// Keep `html` as gate `id`'s report, in one step so the board never reads half of it.
pub(super) fn write(ctx: &Context, id: &str, html: &str) -> Result<(), String> {
    let path = path_of(&ctx.state, &ctx.repo.slug, id);
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    let staged = crate::infra::fs::stage(parent, html)?;
    std::fs::rename(&staged, &path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("cannot write {}: {e}", path.display())
    })
}

/// Drop gate `id`'s copy, for a gate that could not be written after it. Nothing to say if it
/// is already gone.
pub(super) fn remove(ctx: &Context, id: &str) {
    let _ = std::fs::remove_file(path_of(&ctx.state, &ctx.repo.slug, id));
}

/// The report copy of gate `id`. `Ok(None)` when the gate has none; `Err` when there is a
/// copy that cannot be read, which is not the same as there being none.
pub fn load(root: &Path, slug: &str, id: &str) -> Result<Option<String>, String> {
    if !valid_id(id) {
        return Ok(None);
    }
    let path = path_of(root, slug, id);
    match std::fs::read_to_string(&path) {
        Ok(html) => Ok(Some(html)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}
