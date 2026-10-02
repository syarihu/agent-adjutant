//! The parent task's title, for the hub that works on it.
//!
//! A parent-task hub is named by a key (`ALPHA-233`), which says little at a glance. The title
//! is the tracker's, so it is read once through `gh` and kept on disk; the board never waits for
//! it, and a poll never reaches the network.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use super::Context;
use crate::messaging;
use crate::session::RepoHub;
use crate::task;

/// How long an issue that could not be read is left alone before it is asked about again.
/// Only in memory: a restarted server tries once more.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

/// What each parent-task hub's title was last found to be, and whether a read is out.
#[derive(Default)]
pub struct HubTitles {
    seen: Mutex<HashMap<String, Seen>>,
}

#[derive(Default)]
struct Seen {
    /// The file under `hub-titles/` has been looked at.
    loaded: bool,
    /// The issue read and its title.
    found: Option<(String, String)>,
    asking: bool,
    failed: Option<(String, Instant)>,
}

fn file_of(slug: &str) -> std::path::PathBuf {
    messaging::state_dir()
        .join("hub-titles")
        .join(format!("{slug}.json"))
}

/// The title kept for the hub's parent task, if one has been read. Only the file is looked at,
/// so the board list can use it on every poll.
pub(super) fn cached_title(slug: &str) -> Option<String> {
    messaging::read_json(&file_of(slug))?
        .get("title")?
        .as_str()
        .map(str::to_string)
}

/// The issue the hub's parent task is: the URL a task under the hub names as its parent, else
/// the one its key stands for in `issueKeys`. None for a key that matches neither, which is
/// never asked about.
fn issue_of(ctx: &Context, hub: &RepoHub, key: &str) -> Option<String> {
    let dir = task::dir(&messaging::state_dir(), &hub.slug);
    let named = task::list(&dir)
        .into_iter()
        .filter_map(|t| t.parent)
        .map(|p| p.trim().to_string())
        .find(|p| task::fetchable_issue(p));
    named.or_else(|| {
        let keys = ctx
            .resolved
            .config
            .as_ref()?
            .get("issueKeys")?
            .as_object()?;
        task::parent_issue_url(key, keys)
    })
}

impl HubTitles {
    /// The title of the hub's parent task, or None while it is not known. `slugs` are the
    /// hubs of this repository, whose task records may already hold the issue.
    ///
    /// Asks in the background when there is an issue and no title for it, never from the poll.
    pub fn look(
        self: &Arc<Self>,
        ctx: &Context,
        hub: &RepoHub,
        slugs: &[String],
    ) -> Option<String> {
        let key = hub
            .key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())?;
        if !hub.parent {
            return None;
        }
        let url = issue_of(ctx, hub, key)?;
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let entry = seen.entry(hub.slug.clone()).or_default();
        if !entry.loaded {
            entry.loaded = true;
            entry.found = messaging::read_json(&file_of(&hub.slug)).and_then(|v| {
                Some((
                    v.get("url")?.as_str()?.to_string(),
                    v.get("title")?.as_str()?.to_string(),
                ))
            });
        }
        if let Some((found, title)) = &entry.found
            && *found == url
        {
            return Some(title.clone());
        }
        let refused = entry
            .failed
            .as_ref()
            .is_some_and(|(failed, at)| *failed == url && at.elapsed() < RETRY_AFTER);
        if !entry.asking && !refused {
            entry.asking = true;
            let titles = Arc::clone(self);
            let ctx = ctx.clone();
            let slug = hub.slug.clone();
            let slugs = slugs.to_vec();
            std::thread::spawn(move || titles.ask(&ctx, &slug, &slugs, &url));
        }
        None
    }

    /// Forget the hubs the board no longer lists. One being asked about is kept, so its answer
    /// has somewhere to land. The files stay: the hub may come back.
    pub fn keep_only(&self, listed: &HashSet<String>) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|slug, entry| entry.asking || listed.contains(slug));
    }

    fn ask(&self, ctx: &Context, slug: &str, slugs: &[String], url: &str) {
        let answer = super::task::known_title(slugs, url)
            .ok_or(())
            .or_else(|()| {
                super::task::read_issue(&ctx.repo.main, url).map(|snapshot| snapshot.title)
            });
        // The title is good whether or not it could be written down: only a restart loses it.
        if let Ok(title) = &answer
            && let Err(e) =
                messaging::write_json(&file_of(slug), &json!({ "url": url, "title": title }))
        {
            eprintln!("adj serve: could not keep the title of {url}: {e}");
        }
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let entry = seen.entry(slug.to_string()).or_default();
        entry.asking = false;
        match answer {
            Ok(title) => {
                entry.failed = None;
                entry.found = Some((url.to_string(), title));
            }
            Err(why) => {
                eprintln!("adj serve: could not read the title of {url}: {why}");
                entry.failed = Some((url.to_string(), Instant::now()));
            }
        }
    }
}
