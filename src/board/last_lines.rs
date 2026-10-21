use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How soon a pane's screen is read again. A busy agent moves its window's activity on every
/// poll, and reading its screen that often would be most of what a poll costs.
pub const LAST_LINE_MIN_AGE: Duration = Duration::from_secs(5);

/// A pane's last read value, with the window activity it was read at and when.
struct LastRead<T> {
    activity: Option<i64>,
    at: Instant,
    /// The epoch second it was read in, against a window activity that has one-second
    /// resolution: activity in the same second may have come after the read.
    secs: i64,
    value: Option<T>,
}

/// A cache of pane reads: maps pane key to last read value.
pub struct PaneCache<T> {
    read: Mutex<HashMap<String, LastRead<T>>>,
}

impl<T> Default for PaneCache<T> {
    fn default() -> Self {
        PaneCache {
            read: Mutex::new(HashMap::new()),
        }
    }
}

impl<T: Clone> PaneCache<T> {
    /// The value of `key`, which `read` produces only when the window has had activity since
    /// the last read and that was at least `LAST_LINE_MIN_AGE` ago.
    pub fn look(
        &self,
        key: &str,
        activity: Option<i64>,
        now: Instant,
        secs: i64,
        read: impl FnOnce() -> Option<T>,
    ) -> Option<T> {
        if let Some(last) = self.read.lock().ok()?.get(key) {
            let recent = now.duration_since(last.at) < LAST_LINE_MIN_AGE;
            // Unknown activity says nothing of whether the pane moved, and a read that found
            // nothing may only have been early: both are read again once the age is up.
            let unchanged = last.value.is_some()
                && activity.is_some_and(|a| last.activity == Some(a) && last.secs > a);
            if recent || unchanged {
                return last.value.clone();
            }
        }
        // Not under the lock: reading the pane runs a command.
        let value = read();
        if let Ok(mut all) = self.read.lock() {
            all.insert(
                key.to_string(),
                LastRead {
                    activity,
                    at: now,
                    secs,
                    value: value.clone(),
                },
            );
        }
        value
    }

    /// The value of `key`, read again whenever the window has had activity since the last read,
    /// however recent that was. For what has to go from the page as soon as the screen moves
    /// (a prompt answered in the terminal), and is asked of few panes, so the age that keeps
    /// `look` cheap on a busy pane is not wanted. Unknown activity, and a read that found
    /// nothing, are always read again.
    pub fn look_unless_moved(
        &self,
        key: &str,
        activity: Option<i64>,
        now: Instant,
        secs: i64,
        read: impl FnOnce() -> Option<T>,
    ) -> Option<T> {
        // A read that found nothing is not kept either: it may have been a capture that failed,
        // and a waiting pane does not move to have it read again.
        if let Some(last) = self.read.lock().ok()?.get(key)
            && last.value.is_some()
            && activity.is_some_and(|a| last.activity == Some(a) && last.secs > a)
        {
            return last.value.clone();
        }
        let value = read();
        if let Ok(mut all) = self.read.lock() {
            all.insert(
                key.to_string(),
                LastRead {
                    activity,
                    at: now,
                    secs,
                    value: value.clone(),
                },
            );
        }
        value
    }

    /// Forgets one pane, so the next poll reads it fresh.
    pub fn drop_key(&self, key: &str) {
        if let Ok(mut all) = self.read.lock() {
            all.remove(key);
        }
    }

    /// Forgets the panes that are no longer listed.
    pub fn keep_only(&self, keys: &HashSet<String>) {
        if let Ok(mut all) = self.read.lock() {
            all.retain(|key, _| keys.contains(key));
        }
    }
}

pub type LastLines = PaneCache<String>;
pub type Prompts = PaneCache<crate::board::SessionPrompt>;
