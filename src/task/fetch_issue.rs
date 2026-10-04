//! Reading a task's issue and keeping it on the record.

use super::*;

use crate::registry::Context;

/// Read the task's issue and keep its title and body on the record.
///
/// `gh` runs with no lock held, since it can take seconds; the record is read again under the
/// lock and written only if it still points at the issue that was read, the way `refresh`
/// treats a merged PR. On failure the record is left as it was, an earlier snapshot included.
pub fn fetch_issue(ctx: &Context, id: &str) -> Result<Task, String> {
    let before = get(&ctx.state, &ctx.repo.slug, id)?;
    let url = issue_to_fetch(&before)
        .ok_or("no GitHub issue to read")?
        .to_string();
    let snapshot = read_issue(&ctx.repo.main, &url)?;
    let _lock = store::lock(ctx, id)?;
    let mut now = get(&ctx.state, &ctx.repo.slug, id)?;
    if issue_to_fetch(&now) != Some(url.as_str()) {
        return Err("the task's issue changed while it was being read".to_string());
    }
    // A title made from the URL gives way to the issue's own on the first read.
    if now.title_pending && !snapshot.title.is_empty() {
        now.title = snapshot.title.clone();
        now.title_pending = false;
    }
    now.issue_snapshot = Some(snapshot);
    now.updated_at = store::stamp();
    store::save(ctx, &now)?;
    Ok(now)
}
