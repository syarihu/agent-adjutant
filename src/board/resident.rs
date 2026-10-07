use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use super::Server;
use crate::registry::{Address, address_of};

/// The resident server: its token and port, and the boards it has opened. A board is opened
/// on the first request for it and kept, for the same reason a dedicated one keeps its
/// `JulesWatch`: what it remembers between polls is a cache, and the records stay the answer.
pub struct Resident {
    /// The state directory this resident and every board it opens read.
    pub root: std::path::PathBuf,
    pub token: String,
    pub port: u16,
    /// Each open board with the address it was built from, so that one whose address has
    /// changed since is not served from the old context.
    pub boards: Mutex<std::collections::HashMap<String, (Address, Arc<Server>)>>,
    /// The tmux version, asked once at start: the board terminal is offered only with one.
    pub tmux: Option<(u32, u32)>,
    /// Open board terminals, over all boards.
    pub terminals: Arc<AtomicUsize>,
    /// The poll that keeps the cards' pull requests up to date, over all boards.
    pub pr_poll: Arc<crate::board::jobs::PrPoll>,
    /// What waits on a person and which terminals are open, over all boards.
    pub waits: Arc<crate::board::jobs::WaitWatch>,
}

impl Resident {
    /// The board for `slug`, or `None` when the address book has no such board any more.
    ///
    /// The address is read on every call — a small file — and the open board is used only
    /// while it is the one the file still names: a checkout that moved, or was replaced under
    /// the same slug, is served from where it is now.
    pub fn board(&self, slug: &str) -> Option<Arc<Server>> {
        // Bounded: a file rewritten again and again while this builds is not worth chasing.
        for _ in 0..3 {
            let Some(address) = address_of(&self.root, slug) else {
                self.boards.lock().ok()?.remove(slug);
                return None;
            };
            if let Some((built_from, open)) = self.boards.lock().ok()?.get(slug)
                && *built_from == address
            {
                return Some(Arc::clone(open));
            }
            // Outside the lock: resolving the checkout asks git, and every other board waits
            // on this map.
            //
            // Never `set_current_dir`: this process is threaded, and the checkout is named to
            // each call instead.
            let repo = crate::kernel::identity::resolve_in(
                Some(Path::new(&address.main)),
                Some(&address.nwo),
                address.hub.as_deref(),
            )
            .ok()
            .filter(|repo| repo.slug == slug)?;
            let ctx = crate::registry::context_at(repo, self.root.clone()).ok()?;
            let server = Arc::new(Server {
                ctx,
                token: self.token.clone(),
                port: self.port,
                resident: true,
                jules: Arc::default(),
                hub_titles: Arc::default(),
                last_lines: Arc::default(),
                tmux: self.tmux,
                terminals: Arc::clone(&self.terminals),
                pr_poll: Some(Arc::clone(&self.pr_poll)),
                waits: Arc::clone(&self.waits),
            });
            let mut boards = self.boards.lock().ok()?;
            // Asked again under the lock: what was built is only put in place while it is still
            // what the file says, so a slower build of an older address cannot replace a newer
            // one — and one that lost the race is thrown away and built again.
            if address_of(&self.root, slug).as_ref() != Some(&address) {
                continue;
            }
            match boards.get(slug) {
                Some((built_from, open)) if *built_from == address => {
                    return Some(Arc::clone(open));
                }
                _ => {
                    boards.insert(slug.to_string(), (address, Arc::clone(&server)));
                    return Some(server);
                }
            }
        }
        None
    }
}
