//! The parent task's title, for the hub that works on it.
//!
//! A parent-task hub is named by a key (`ALPHA-233`), which says little at a glance. The title
//! is the tracker's, so it is read once through `gh` and kept on disk, with how far the issue's
//! sub-issues have got; the board never waits for it, and a poll never reaches the network. A
//! person can also ask for the one issue to be read now (`read_now`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::infra::clock::{now_secs, utc_stamp};
use crate::mail::{HubTitleState, HubTitleStatus, MissingWhy, RepoHub};
use crate::registry::Context;
use crate::task::{self, IssueSummaryRead};

/// How long an issue that could not be read is left alone before it is asked about again, after
/// the first, second and every later failure. Only in memory: a restarted server tries once more.
const RETRY_AFTER: [Duration; 3] = [
    Duration::from_secs(30),
    Duration::from_secs(2 * 60),
    Duration::from_secs(10 * 60),
];

fn retry_after(failures: u32) -> Duration {
    RETRY_AFTER[(failures.saturating_sub(1) as usize).min(RETRY_AFTER.len() - 1)]
}

/// What each parent-task hub's title was last found to be, and whether a read is out.
#[derive(Default)]
pub struct HubTitles {
    seen: Mutex<HashMap<String, Seen>>,
}

#[derive(Default)]
struct Seen {
    /// The issue the rest of this is about; another issue starts it over.
    url: Option<String>,
    asking: bool,
    /// How many reads of that issue failed in a row.
    failures: u32,
    /// Why the last one failed, and when (epoch seconds) it is tried again.
    failed: Option<(String, i64)>,
    /// GitHub has no such issue: asking again will not change that.
    missing: bool,
    /// What was last read of that issue, held so that a file that could not be written does not
    /// make every poll read the issue again.
    held: Option<Kept>,
}

impl Seen {
    fn follow(&mut self, url: &str) {
        if self.url.as_deref() != Some(url) {
            self.url = Some(url.to_string());
            self.failures = 0;
            self.failed = None;
            self.missing = false;
            self.held = None;
        }
    }
}

/// What the file under `hub-titles/` keeps: the issue, its title and, when the issue was read
/// on its own, its sub-issue counts and when. A file from before the counts has only the first
/// two.
#[derive(Clone)]
struct Kept {
    url: String,
    title: String,
    total: Option<u32>,
    completed: Option<u32>,
    read_at: Option<String>,
}

fn kept_of(v: &Value) -> Option<Kept> {
    let count = |key: &str| v.get(key).and_then(Value::as_u64).map(|n| n as u32);
    Some(Kept {
        url: v.get("url")?.as_str()?.to_string(),
        title: v.get("title")?.as_str()?.to_string(),
        total: count("total"),
        completed: count("completed"),
        read_at: v.get("readAt").and_then(Value::as_str).map(str::to_string),
    })
}

/// Write down what was read of `url`. The file's other keys stay (a newer binary's). Counts
/// kept from another issue are dropped, and those of this one stay when it was read without
/// them.
fn keep(
    path: &std::path::Path,
    url: &str,
    title: &str,
    counts: Option<(u32, u32)>,
    read_at: &str,
) -> Result<(), String> {
    // Not locked: two boards writing at once can lose one read's counts, and the next read
    // rewrites them, so the lock the other records take is not worth its cost here.
    let mut kept = match crate::infra::fs::read_json(path) {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    if kept.get("url").and_then(Value::as_str) != Some(url) {
        for key in ["total", "completed", "readAt"] {
            kept.remove(key);
        }
    }
    kept.insert("url".into(), url.into());
    kept.insert("title".into(), title.into());
    if let Some((total, completed)) = counts {
        kept.insert("total".into(), total.into());
        kept.insert("completed".into(), completed.into());
        kept.insert("readAt".into(), read_at.into());
    }
    crate::infra::fs::write_json(path, &Value::Object(kept))
}

fn file_of(root: &std::path::Path, slug: &str) -> std::path::PathBuf {
    root.join("hub-titles").join(format!("{slug}.json"))
}

/// The title kept for the hub's parent task, if one has been read. Only the file is looked at,
/// so the board list can use it on every poll.
pub fn cached_title(root: &std::path::Path, slug: &str) -> Option<String> {
    crate::infra::fs::read_json(&file_of(root, slug))?
        .get("title")?
        .as_str()
        .map(str::to_string)
}

/// The issue the hub's parent task is: the URL a task under the hub names as its parent, else
/// the one its key stands for in `issueKeys`. None for a key that matches neither, which is
/// never asked about.
fn issue_of(ctx: &Context, slug: &str, key: &str) -> Option<String> {
    let named = task::list(&ctx.state, slug)
        .into_iter()
        .filter_map(|t| t.parent)
        .map(|p| p.trim().to_string())
        .find(|p| task::fetchable_issue(p));
    named.or_else(|| task::parent_issue_url(key, &ctx.settings.issue_keys))
}

/// What a read of the issue came to.
enum Outcome {
    /// The title, and the sub-issue counts when the read gave them.
    Found {
        title: String,
        counts: Option<(u32, u32)>,
    },
    NotFound,
    Failed(String),
}

/// Read the issue through `gh`.
fn read_over_the_network(ctx: &Context, url: &str) -> Outcome {
    let Some(issue) = task::issue_ref_of(url) else {
        // A URL the summary read will not take still has a title `gh issue view` can give.
        return match task::read_issue(&ctx.repo.main, url) {
            Ok(snapshot) => Outcome::Found {
                title: snapshot.title,
                counts: None,
            },
            Err(why) => Outcome::Failed(why),
        };
    };
    match task::read_issue_summary(&issue, Instant::now() + super::pr_poll::ROUND_TIMEOUT) {
        IssueSummaryRead::Found(read) => Outcome::Found {
            title: read.title,
            counts: Some((read.total, read.completed)),
        },
        IssueSummaryRead::NotFound => Outcome::NotFound,
        IssueSummaryRead::Failed(why) => Outcome::Failed(why),
    }
}

/// What `read_now` came to.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadNow {
    Read {
        title: String,
        total: Option<u32>,
        completed: Option<u32>,
        read_at: String,
    },
    /// A read of this issue is already out.
    Busy,
    /// GitHub has no such issue.
    NotFound,
    /// There is no issue to read.
    Missing(MissingWhy),
}

/// What `look_at` decided: what to show, and the issue to read in the background, if one is to be.
struct Looked {
    title: Option<String>,
    state: Option<HubTitleState>,
    ask: Option<String>,
}

fn state_of(status: HubTitleStatus, reading: bool, url: Option<&str>) -> HubTitleState {
    HubTitleState {
        status,
        reading,
        why: None,
        missing: None,
        retry_at: None,
        total: None,
        completed: None,
        read_at: None,
        parent_key: url.map(task::parent_key),
    }
}

fn missing(why: MissingWhy, reading: bool, url: Option<&str>) -> HubTitleState {
    HubTitleState {
        missing: Some(why),
        ..state_of(HubTitleStatus::Missing, reading, url)
    }
}

impl HubTitles {
    /// The title of the hub's parent task, or None while it is not known, with where it stands.
    /// `slugs` are the hubs of this repository, whose task records may already hold the issue.
    /// Neither is there for a hub that is not a parent-task hub.
    ///
    /// Asks in the background when there is an issue and no title for it, never from the poll.
    pub fn look(
        self: &Arc<Self>,
        ctx: &Context,
        hub: &RepoHub,
        slugs: &[String],
    ) -> (Option<String>, Option<HubTitleState>) {
        let looked = self.look_at(ctx, hub, now_secs());
        if let Some(url) = looked.ask {
            let titles = Arc::clone(self);
            let ctx = ctx.clone();
            let slug = hub.slug.clone();
            let slugs = slugs.to_vec();
            std::thread::spawn(move || titles.ask(&ctx, &slug, &slugs, &url));
        }
        (looked.title, looked.state)
    }

    fn look_at(&self, ctx: &Context, hub: &RepoHub, now: i64) -> Looked {
        let shown = |title, state| Looked {
            title,
            state: Some(state),
            ask: None,
        };
        if !hub.parent {
            return Looked {
                title: None,
                state: None,
                ask: None,
            };
        }
        let Some(key) = hub.key.as_deref().map(str::trim).filter(|k| !k.is_empty()) else {
            return shown(None, missing(MissingWhy::NoKey, false, None));
        };
        let Some(url) = issue_of(ctx, &hub.slug, key) else {
            return shown(None, missing(MissingWhy::NoIssue, false, None));
        };
        // Looked at on every poll, so a title another board kept shows here too.
        let kept = crate::infra::fs::read_json(&file_of(&ctx.state, &hub.slug))
            .as_ref()
            .and_then(kept_of)
            .filter(|k| k.url == url);
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let entry = seen.entry(hub.slug.clone()).or_default();
        entry.follow(&url);
        let reading = entry.asking;
        let kept = kept.or_else(|| entry.held.clone());
        if let Some(kept) = kept {
            let state = HubTitleState {
                total: kept.total,
                completed: kept.completed,
                read_at: kept.read_at,
                ..state_of(HubTitleStatus::Read, reading, Some(&url))
            };
            return shown(Some(kept.title), state);
        }
        if entry.missing {
            return shown(None, missing(MissingWhy::NotFound, reading, Some(&url)));
        }
        if let Some((why, retry_at)) = &entry.failed
            && now < *retry_at
        {
            let state = HubTitleState {
                why: Some(why.clone()),
                retry_at: Some(*retry_at),
                ..state_of(HubTitleStatus::Failed, reading, Some(&url))
            };
            return shown(None, state);
        }
        let ask = (!entry.asking).then(|| url.clone());
        entry.asking = true;
        Looked {
            ask,
            ..shown(None, state_of(HubTitleStatus::Pending, true, Some(&url)))
        }
    }

    /// Forget the hubs the board no longer lists. One being asked about is kept, so its answer
    /// has somewhere to land. The files stay: the hub may come back.
    pub fn keep_only(&self, listed: &HashSet<String>) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|slug, entry| entry.asking || listed.contains(slug));
    }

    fn ask(&self, ctx: &Context, slug: &str, slugs: &[String], url: &str) {
        self.ask_with(ctx, slug, slugs, url, &read_over_the_network);
    }

    /// The background read: a title some task record already holds is taken without asking `gh`
    /// (it has no counts), so an issue the board has read is not read again.
    fn ask_with(
        &self,
        ctx: &Context,
        slug: &str,
        slugs: &[String],
        url: &str,
        read: &dyn Fn(&Context, &str) -> Outcome,
    ) {
        let outcome = match task::known_title(&ctx.state, slugs, url) {
            Some(title) => Outcome::Found {
                title,
                counts: None,
            },
            None => read(ctx, url),
        };
        self.finish(ctx, slug, url, outcome, now_secs());
    }

    /// Read the hub's parent issue now, whatever the background read has done or is doing, and
    /// keep what it says. Always asks `gh`: a person who pressed the button wants GitHub's word.
    pub fn read_now(&self, ctx: &Context, slug: &str, key: &str) -> Result<ReadNow, String> {
        self.read_now_with(ctx, slug, key, &read_over_the_network)
    }

    fn read_now_with(
        &self,
        ctx: &Context,
        slug: &str,
        key: &str,
        read: &dyn Fn(&Context, &str) -> Outcome,
    ) -> Result<ReadNow, String> {
        let key = key.trim();
        if key.is_empty() {
            return Ok(ReadNow::Missing(MissingWhy::NoKey));
        }
        let Some(url) = issue_of(ctx, slug, key) else {
            return Ok(ReadNow::Missing(MissingWhy::NoIssue));
        };
        {
            let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            let entry = seen.entry(slug.to_string()).or_default();
            if entry.asking {
                return Ok(ReadNow::Busy);
            }
            entry.follow(&url);
            entry.asking = true;
        }
        let outcome = read(ctx, &url);
        let now = now_secs();
        let answer = match &outcome {
            Outcome::Found { title, counts } => Ok(ReadNow::Read {
                title: title.clone(),
                total: counts.map(|c| c.0),
                completed: counts.map(|c| c.1),
                read_at: utc_stamp(now),
            }),
            Outcome::NotFound => Ok(ReadNow::NotFound),
            Outcome::Failed(why) => Err(why.clone()),
        };
        self.finish(ctx, slug, &url, outcome, now);
        answer
    }

    /// What either read leaves behind: the title kept, or what is known of why there is none.
    fn finish(&self, ctx: &Context, slug: &str, url: &str, outcome: Outcome, now: i64) {
        // The title is good whether or not it could be written down: `held` keeps it for this
        // run, and only a restart loses it.
        if let Outcome::Found { title, counts } = &outcome
            && let Err(e) = keep(
                &file_of(&ctx.state, slug),
                url,
                title,
                *counts,
                &utc_stamp(now),
            )
        {
            eprintln!("adj serve: could not keep the title of {url}: {e}");
        }
        if let Outcome::Failed(why) = &outcome {
            eprintln!("adj serve: could not read the title of {url}: {why}");
        }
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let entry = seen.entry(slug.to_string()).or_default();
        entry.asking = false;
        // The hub may have moved to another issue while this was out: that one's own reads say.
        if entry.url.as_deref() != Some(url) {
            return;
        }
        match outcome {
            Outcome::Found { title, counts } => {
                entry.held = Some(Kept {
                    url: url.to_string(),
                    title,
                    total: counts.map(|c| c.0),
                    completed: counts.map(|c| c.1),
                    read_at: Some(utc_stamp(now)),
                });
                entry.failures = 0;
                entry.failed = None;
                entry.missing = false;
            }
            Outcome::NotFound => entry.missing = true,
            Outcome::Failed(why) => {
                entry.failures += 1;
                let retry_at = now + retry_after(entry.failures).as_secs() as i64;
                entry.failed = Some((why, retry_at));
            }
        }
    }
}

#[cfg(test)]
mod tests;
