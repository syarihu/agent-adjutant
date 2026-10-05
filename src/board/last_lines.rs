use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How soon a pane's screen is read again. A busy agent moves its window's activity on every
/// poll, and reading its screen that often would be most of what a poll costs.
pub const LAST_LINE_MIN_AGE: Duration = Duration::from_secs(5);

/// A pane's last line, with the window activity it was read at and when.
struct LastRead {
    activity: Option<i64>,
    at: Instant,
    /// The epoch second it was read in, against a window activity that has one-second
    /// resolution: activity in the same second may have come after the read.
    secs: i64,
    line: Option<String>,
}

/// The last line of output of each pane a page has asked for, by pane.
#[derive(Default)]
pub struct LastLines {
    read: Mutex<HashMap<String, LastRead>>,
}

impl LastLines {
    /// The line of `key`, which `read` produces only when the window has had activity since
    /// the last read and that was at least `LAST_LINE_MIN_AGE` ago.
    pub fn look(
        &self,
        key: &str,
        activity: Option<i64>,
        now: Instant,
        secs: i64,
        read: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        if let Some(last) = self.read.lock().ok()?.get(key) {
            let recent = now.duration_since(last.at) < LAST_LINE_MIN_AGE;
            // Unknown activity says nothing of whether the pane moved, and a read that found
            // nothing may only have been early: both are read again once the age is up.
            let unchanged = last.line.is_some()
                && activity.is_some_and(|a| last.activity == Some(a) && last.secs > a);
            if recent || unchanged {
                return last.line.clone();
            }
        }
        // Not under the lock: reading the pane runs a command.
        let line = read();
        if let Ok(mut all) = self.read.lock() {
            all.insert(
                key.to_string(),
                LastRead {
                    activity,
                    at: now,
                    secs,
                    line: line.clone(),
                },
            );
        }
        line
    }

    /// Forgets the panes that are no longer listed.
    pub fn keep_only(&self, keys: &HashSet<String>) {
        if let Ok(mut all) = self.read.lock() {
            all.retain(|key, _| keys.contains(key));
        }
    }
}
