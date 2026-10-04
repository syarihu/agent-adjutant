//! The title of an issue some task record already holds.

use super::*;

/// The title of the issue at `url` when a task record of one of these hubs already holds it:
/// the snapshot read for a task whose issue it is, else the title of the task made from it.
/// Looked at before `gh` is asked, so an issue the board has already read is not read again.
pub fn known_title(root: &Path, slugs: &[String], url: &str) -> Option<String> {
    let tasks = slugs.iter().flat_map(|slug| list(root, slug));
    let mut fallback = None;
    for t in tasks {
        if let Some(snapshot) = t.issue_snapshot.as_ref().filter(|s| s.url == url) {
            return Some(snapshot.title.clone()).filter(|t| !t.is_empty());
        }
        if fallback.is_none()
            && !t.title_pending
            && t.issue_url.as_deref() == Some(url)
            && !t.title.is_empty()
        {
            fallback = Some(t.title.clone());
        }
    }
    fallback
}
