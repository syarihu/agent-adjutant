//! Which client is open to ring a notification: the desktop app, a board page, or neither.
//!
//! More than one board server can run at once (the resident one, a hub's own, a board served
//! alone), and a notification is raised by whichever process happens to hold the event, not by
//! the one the page polls. So each server process keeps its own file under `notify-clients/`
//! and whoever decides how to ring reads them all and takes the latest of each kind.

use super::store::notify_clients_dir;
use super::*;
use std::sync::Mutex;

/// How long after its last poll a client still counts as open. A hidden tab, or a window the
/// system has throttled, polls about once a minute, so this has to outlast that. A window
/// throttled for longer than this counts as closed.
pub const PAGE_FRESH_SECS: i64 = 90;

/// A file is rewritten only when a value has moved this far from what it holds: the page polls
/// every two seconds, and the readers need the answer to the nearest few seconds only.
const WRITE_EVERY_SECS: i64 = 10;

/// A file whose latest sighting is older than this is a server that is long gone.
const KEPT_SECS: i64 = 24 * 60 * 60;

/// Who a request came from, for the requests that say they can ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    /// The desktop app, whose webview rings through the system's own notifications.
    App,
    /// A board page in a browser.
    Page,
}

/// Who rings, given what is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The desktop app polled within `PAGE_FRESH_SECS`: only it rings.
    App,
    /// A board page polled within `PAGE_FRESH_SECS`, and the app did not: only a page rings.
    Page,
    /// Neither is open.
    None,
}

/// When each kind of client was last seen, in unix seconds; `0` is never. `*_at` is a client
/// that can show notifications at all (the browser's permission is granted); `*_waits_at` is one
/// that also has 確認待ち on, which is the one that rings waits and gates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Clients {
    pub page_at: i64,
    pub app_at: i64,
    pub page_waits_at: i64,
    pub app_waits_at: i64,
}

/// What to do with a wait or a gate when `notification` is unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitRoute {
    /// A client of the target kind polls this very process, so it sees the board's state and
    /// rings the wait itself: leave it to that client.
    Leave,
    /// The target client is open elsewhere and cannot see this board: queue a notice for it.
    Queue(Client),
    /// The target kind is open but has 確認待ち off: nothing rings.
    Drop,
    /// Neither is open: `terminal-notifier`, if it is installed.
    Notifier,
}

/// Who rings a wait or gate: the app if one is open anywhere, else a page if one is, else
/// neither. `shared` is every process's stamps, `local` this process's own, which is whether the
/// target client reads this process's boards.
pub fn wait_route(shared: Clients, local: Clients, now: i64) -> WaitRoute {
    let (client, shared_at, local_at) = match shared.route(now) {
        Route::App => (Client::App, shared.app_waits_at, local.app_waits_at),
        Route::Page => (Client::Page, shared.page_waits_at, local.page_waits_at),
        Route::None => return WaitRoute::Notifier,
    };
    if !fresh(shared_at, now) {
        WaitRoute::Drop
    } else if fresh(local_at, now) {
        WaitRoute::Leave
    } else {
        WaitRoute::Queue(client)
    }
}

impl Clients {
    pub fn route(self, now: i64) -> Route {
        route(self.page_at, self.app_at, now)
    }

    pub fn app_fresh(self, now: i64) -> bool {
        fresh(self.app_at, now)
    }
}

/// Whether a client seen at `at` is still open at `now`.
pub fn fresh(at: i64, now: i64) -> bool {
    at > 0 && now - at <= PAGE_FRESH_SECS
}

/// The app first, because it rings through the system and a browser beside it would ring the
/// same thing twice; then a page; then nobody.
pub fn route(page_at: i64, app_at: i64, now: i64) -> Route {
    if fresh(app_at, now) {
        Route::App
    } else if fresh(page_at, now) {
        Route::Page
    } else {
        Route::None
    }
}

/// What this process has noted, and what it last wrote: the latest values are kept here so the
/// throttle on the file does not make a server lag behind its own client.
#[derive(Default, Clone, Copy)]
struct Noted {
    seen: Clients,
    written: Clients,
}

static NOTED: Mutex<Vec<(PathBuf, Noted)>> = Mutex::new(Vec::new());

fn own_file(root: &Path) -> PathBuf {
    notify_clients_dir(root).join(format!("{}.json", std::process::id()))
}

/// What this process alone has noted: the clients that read this process's boards.
pub fn local_notify_clients(root: &Path) -> Clients {
    let path = own_file(root);
    let noted = NOTED.lock().unwrap_or_else(|e| e.into_inner());
    noted
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, entry)| entry.seen)
        .unwrap_or_default()
}

/// Note that a client that can notify was seen at `now`, by this process, and whether it also
/// rings waits and gates (`waits`). The file is written when a value has moved by
/// `WRITE_EVERY_SECS` since it was last written.
pub fn note_notify_client(root: &Path, client: Client, waits: bool, now: i64) {
    let path = own_file(root);
    let wrote = {
        // Held across the write: two requests of this process noting at once would otherwise write
        // their snapshots in either order, and an older one (a page only) could replace a newer
        // one that has the app in it. `written` is moved only once the file is replaced, so a
        // failed write is tried again at the next sighting.
        let mut noted = NOTED.lock().unwrap_or_else(|e| e.into_inner());
        let at = match noted.iter().position(|(p, _)| *p == path) {
            Some(at) => at,
            None => {
                noted.push((path.clone(), Noted::default()));
                noted.len() - 1
            }
        };
        let entry = &mut noted[at].1;
        let (at, waits_at) = match client {
            Client::App => (&mut entry.seen.app_at, &mut entry.seen.app_waits_at),
            Client::Page => (&mut entry.seen.page_at, &mut entry.seen.page_waits_at),
        };
        *at = (*at).max(now);
        if waits {
            *waits_at = (*waits_at).max(now);
        }
        let moved = |seen: i64, written: i64| seen - written >= WRITE_EVERY_SECS;
        let due = moved(entry.seen.page_at, entry.written.page_at)
            || moved(entry.seen.app_at, entry.written.app_at)
            || moved(entry.seen.page_waits_at, entry.written.page_waits_at)
            || moved(entry.seen.app_waits_at, entry.written.app_waits_at);
        let seen = entry.seen;
        let record = json!({
            "pid": std::process::id(),
            "pageAt": seen.page_at,
            "appAt": seen.app_at,
            "pageWaitsAt": seen.page_waits_at,
            "appWaitsAt": seen.app_waits_at,
        });
        let wrote = due && write_json(&path, &record).is_ok();
        if wrote {
            entry.written = seen;
        }
        wrote
    };
    if wrote {
        prune_old_clients(root, now);
    }
}

/// When each kind of client was last seen by any server process: the latest of every file, and
/// of what this process has noted but not written yet. A file that cannot be read is skipped.
pub fn notify_clients(root: &Path) -> Clients {
    let dir = notify_clients_dir(root);
    let mut latest = Clients::default();
    let mut take = |seen: Clients| {
        latest.page_at = latest.page_at.max(seen.page_at);
        latest.app_at = latest.app_at.max(seen.app_at);
        latest.page_waits_at = latest.page_waits_at.max(seen.page_waits_at);
        latest.app_waits_at = latest.app_waits_at.max(seen.app_waits_at);
    };
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(record) = read_json(&path) else {
            continue;
        };
        // A file from before the waits stamps has none: never.
        let at = |key: &str| record.get(key).and_then(Value::as_i64).unwrap_or(0);
        take(Clients {
            page_at: at("pageAt"),
            app_at: at("appAt"),
            page_waits_at: at("pageWaitsAt"),
            app_waits_at: at("appWaitsAt"),
        });
    }
    let noted = NOTED.lock().unwrap_or_else(|e| e.into_inner());
    for (path, entry) in noted.iter() {
        if path.starts_with(&dir) {
            take(entry.seen);
        }
    }
    latest
}

/// Remove the files of servers that have not heard from a client for a day. Best effort. A file
/// is left alone while its process (the pid in its name) is alive or cannot be told apart from
/// one: its owner may have just rewritten it.
fn prune_old_clients(root: &Path, now: i64) {
    prune_old_clients_with(root, now, &process_is_gone);
}

/// Whether `ps` says there is no process `pid`. Not being able to ask is not an answer.
fn process_is_gone(pid: u32) -> bool {
    matches!(
        super::liveness::ps_answer(pid, "lstart"),
        super::liveness::Answer::NoSuchProcess
    )
}

fn prune_old_clients_with(root: &Path, now: i64, gone: &dyn Fn(u32) -> bool) {
    let own = own_file(root);
    for entry in std::fs::read_dir(notify_clients_dir(root))
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path == own || path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        // A file whose process is not known to be gone is left alone, and so is one that is
        // unreadable: it may be a newer version's.
        let owner = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<u32>().ok());
        if !owner.is_some_and(gone) {
            continue;
        }
        let Some(record) = read_json(&path) else {
            continue;
        };
        let at = |key: &str| record.get(key).and_then(Value::as_i64).unwrap_or(0);
        if now - at("pageAt").max(at("appAt")) > KEPT_SECS {
            let _ = remove_if_present(&path);
        }
    }
}

#[cfg(test)]
mod tests;
