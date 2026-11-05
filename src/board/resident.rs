use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use super::{MachineState, Server};
use crate::registry::{Address, address_of};

/// What `std::fs::metadata(config_path())` says about the config file: its path, modified
/// time, and length. `None` when the file cannot be stat'ed (missing).
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ConfigStamp {
    pub path: std::path::PathBuf,
    pub modified: Option<std::time::SystemTime>,
    pub len: u64,
}

impl ConfigStamp {
    fn current() -> Option<Self> {
        let path = crate::kernel::config::config_path();
        let meta = std::fs::metadata(&path).ok()?;
        Some(Self {
            path,
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// An open board with the address and config stamp it was built from.
#[derive(Clone)]
pub struct OpenBoard {
    pub address: Address,
    pub stamp: Option<ConfigStamp>,
    pub server: Arc<Server>,
}

/// The resident server: its token and port, and the boards it has opened. A board is opened
/// on the first request for it and kept, for the same reason a dedicated one keeps its
/// `JulesWatch`: what it remembers between polls is a cache, and the records stay the answer.
pub struct Resident {
    /// The state directory this resident and every board it opens read.
    pub root: std::path::PathBuf,
    pub token: String,
    pub port: u16,
    /// Each open board with the address and config stamp it was built from, so that one
    /// whose address or config has changed since is rebuilt.
    pub boards: Mutex<std::collections::HashMap<String, OpenBoard>>,
    /// What the last look at the machine found (tmux, `gh`, the MCP registration, the config),
    /// shared with every board. The board terminal is offered only while it finds a tmux.
    pub machine: Arc<MachineState>,
    /// Open board terminals, over all boards.
    pub terminals: Arc<AtomicUsize>,
    /// The poll that keeps the cards' pull requests up to date, over all boards.
    pub pr_poll: Arc<crate::board::jobs::PrPoll>,
    /// What waits on a person and which terminals are open, over all boards.
    pub waits: Arc<crate::board::jobs::WaitWatch>,
    /// The GitHub icon of each repository owner, read in the background for the work document.
    pub owner_icons: Arc<crate::board::jobs::OwnerIcons>,
    /// The serialized `/api/work` document and when it was built, reused for 1500 ms so that
    /// several open pages polling every couple of seconds read the boards once between them.
    pub work_cache: Mutex<Option<(std::time::Instant, String)>>,
}

impl Resident {
    /// The board for `slug`, or `None` when the address book has no such board any more.
    ///
    /// The address and config stamp are read on every call, and the open board is reused only
    /// while both still match what it was built from: a checkout that moved or a config file
    /// that changed rebuilds the board's context. When only the config changed, the board's
    /// caches are kept across the rebuild.
    pub fn board(&self, slug: &str) -> Option<Arc<Server>> {
        // Bounded: a file rewritten again and again while this builds is not worth chasing.
        for _ in 0..3 {
            let Some(address) = address_of(&self.root, slug) else {
                self.boards.lock().ok()?.remove(slug);
                return None;
            };
            let stamp = ConfigStamp::current();

            let cached = self.boards.lock().ok()?.get(slug).cloned();
            if let Some(ref open) = cached
                && open.address == address
                && open.stamp == stamp
            {
                return Some(Arc::clone(&open.server));
            }

            let old_server = cached
                .filter(|open| open.address == address)
                .map(|open| open.server);

            // Outside the lock: resolving the checkout asks git, and every other board waits
            // on this map.
            //
            // Never `set_current_dir`: this process is threaded, and the checkout is named to
            // each call instead.
            let ctx = match crate::registry::context_at_address(&self.root, &address) {
                Ok(ctx) => ctx,
                // A config caught half written does not parse: the board already open keeps
                // serving with the settings it has, and its stamp is left alone, so the next
                // call tries again rather than taking the board away until a restart.
                Err(_) => return old_server,
            };

            let (jules, hub_titles, last_lines, prompts, diffs) = match &old_server {
                Some(old) => (
                    Arc::clone(&old.jules),
                    Arc::clone(&old.hub_titles),
                    Arc::clone(&old.last_lines),
                    Arc::clone(&old.prompts),
                    Arc::clone(&old.diffs),
                ),
                None => (
                    Arc::default(),
                    Arc::default(),
                    Arc::default(),
                    Arc::default(),
                    Arc::default(),
                ),
            };

            let server = Arc::new(Server {
                ctx,
                token: self.token.clone(),
                port: self.port,
                resident: true,
                jules,
                hub_titles,
                last_lines,
                prompts,
                diffs,
                machine: Arc::clone(&self.machine),
                terminals: Arc::clone(&self.terminals),
                pr_poll: Some(Arc::clone(&self.pr_poll)),
                waits: Arc::clone(&self.waits),
            });

            let mut boards = self.boards.lock().ok()?;
            // Asked again under the lock: what was built is only put in place while it is still
            // what the address file and config say, so a slower build cannot replace a newer
            // one — and one that lost the race is thrown away and built again.
            if address_of(&self.root, slug).as_ref() != Some(&address)
                || ConfigStamp::current() != stamp
            {
                continue;
            }
            if let Some(open) = boards.get(slug)
                && open.address == address
                && open.stamp == stamp
            {
                return Some(Arc::clone(&open.server));
            }
            boards.insert(
                slug.to_string(),
                OpenBoard {
                    address,
                    stamp,
                    server: Arc::clone(&server),
                },
            );
            return Some(server);
        }
        None
    }
}
