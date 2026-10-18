//! The GitHub icon of each repository owner, for the organisation switch.
//!
//! One `gh api users/<login>` per owner (that endpoint answers for organisations too, and says
//! which kind it is), read in the background the first time an owner is seen and read again
//! after a day. The work document never waits for it: until an owner is read, it is listed by
//! name and the page draws an initial. Kept in memory only, so a restart costs one call per owner.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::infra::gh::GhRun;

/// How long a read stays good. The old icon is served while the next read is out.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

/// How long an owner that could not be read is left alone before it is asked about again, after
/// the first, second and every later failure.
const RETRY_AFTER: [Duration; 3] = [
    Duration::from_secs(30),
    Duration::from_secs(2 * 60),
    Duration::from_secs(10 * 60),
];

/// How `gh` is run, handed in so that a test does not reach GitHub.
type Gh<'a> = &'a dyn Fn(&[&str], Instant) -> Result<GhRun, String>;

const READ_TIMEOUT: Duration = Duration::from_secs(10);

fn retry_after(failures: u32) -> Duration {
    RETRY_AFTER[(failures.saturating_sub(1) as usize).min(RETRY_AFTER.len() - 1)]
}

/// An owner, with what GitHub said of it once that has been read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerIcon {
    pub login: String,
    /// An `https://` URL of the avatar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// "organization" or "user", as GitHub says; absent until read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// What is known of each owner, and which reads are out.
#[derive(Default)]
pub struct OwnerIcons {
    /// By lowercase login.
    seen: Mutex<HashMap<String, Seen>>,
}

#[derive(Default)]
struct Seen {
    asking: bool,
    read_at: Option<Instant>,
    /// How many reads in a row failed.
    failures: u32,
    retry_at: Option<Instant>,
    icon: Option<OwnerIcon>,
}

impl OwnerIcons {
    /// What is known of each of `logins`, never waiting: one not read yet has only its name.
    pub fn look(&self, logins: &[String]) -> Vec<OwnerIcon> {
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        logins
            .iter()
            .map(|login| {
                seen.get(&login.to_ascii_lowercase())
                    .and_then(|s| s.icon.clone())
                    .unwrap_or_else(|| OwnerIcon {
                        login: login.clone(),
                        avatar_url: None,
                        kind: None,
                    })
            })
            .collect()
    }

    /// Read the owners that are due, in one background thread. None of them is asked about while
    /// a read of it is out, so a poll that comes before the answer starts nothing.
    pub fn want(self: &Arc<Self>, logins: &[String]) {
        let ask = {
            let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            let ask = due(&seen, logins, Instant::now());
            for login in &ask {
                seen.entry(login.clone()).or_default().asking = true;
            }
            ask
        };
        if ask.is_empty() {
            return;
        }
        let icons = Arc::clone(self);
        std::thread::spawn(move || {
            for login in ask {
                // Not under the lock: `gh` can take its whole deadline.
                let read = read_owner(&login);
                icons.record(&login, read, Instant::now());
            }
        });
    }

    fn record(&self, login: &str, read: Result<OwnerIcon, String>, now: Instant) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let entry = seen.entry(login.to_ascii_lowercase()).or_default();
        entry.asking = false;
        match read {
            Ok(icon) => {
                entry.icon = Some(icon);
                entry.read_at = Some(now);
                entry.failures = 0;
                entry.retry_at = None;
            }
            // The icon read before, if any, stays: an owner is not unknown because GitHub was
            // not reachable for a minute.
            Err(_) => {
                entry.failures += 1;
                entry.retry_at = Some(now + retry_after(entry.failures));
            }
        }
    }
}

/// The lowercase logins of `logins` that are to be read now: never read, or read longer ago than
/// `KEEP`, and neither being read nor waiting out a failure.
fn due(seen: &HashMap<String, Seen>, logins: &[String], now: Instant) -> Vec<String> {
    let mut due: Vec<String> = Vec::new();
    for login in logins {
        let login = login.to_ascii_lowercase();
        let ready = match seen.get(&login) {
            None => true,
            Some(s) if s.asking => false,
            Some(s) if s.retry_at.is_some_and(|at| now < at) => false,
            Some(s) => s
                .read_at
                .is_none_or(|at| now.saturating_duration_since(at) >= KEEP),
        };
        if ready && !due.contains(&login) {
            due.push(login);
        }
    }
    due
}

/// Whether `login` is a GitHub login: it goes into a path of the API, so nothing else is asked.
pub(crate) fn valid_login(login: &str) -> bool {
    let bytes = login.as_bytes();
    (1..=39).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
}

/// The owner out of `gh`'s answer. Only an `https://` avatar is kept: the page puts it in an
/// `<img src>`.
fn parse_owner(stdout: &str) -> Result<OwnerIcon, String> {
    let v: serde_json::Value =
        serde_json::from_str(stdout).map_err(|e| format!("unreadable answer: {e}"))?;
    let text = |key: &str| v.get(key).and_then(|x| x.as_str());
    let login = text("login")
        .filter(|l| valid_login(l))
        .ok_or("the answer has no login")?;
    Ok(OwnerIcon {
        login: login.to_string(),
        avatar_url: text("avatar")
            .filter(|u| u.starts_with("https://"))
            .map(str::to_string),
        kind: text("type").map(str::to_ascii_lowercase),
    })
}

fn read_owner(login: &str) -> Result<OwnerIcon, String> {
    read_owner_with(login, &|args, deadline| {
        crate::infra::gh::run(None, args, deadline)
    })
}

/// Ask GitHub about `login`. A failed run is an `Err`, never "no avatar".
fn read_owner_with(login: &str, gh: Gh) -> Result<OwnerIcon, String> {
    if !valid_login(login) {
        return Err(format!("not a GitHub login: {login}"));
    }
    let path = format!("users/{login}");
    let args = [
        "api",
        "--hostname",
        super::HOST,
        path.as_str(),
        "--jq",
        "{login: .login, type: .type, avatar: .avatar_url}",
    ];
    let run = gh(&args, Instant::now() + READ_TIMEOUT)?;
    if !run.ok {
        return Err(run.stderr.trim().to_string());
    }
    parse_owner(&run.stdout)
}

#[cfg(test)]
mod tests;
