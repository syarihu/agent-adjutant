//! Whose PRs a sync looks at: the owners of the repositories adj has a board for.

use std::collections::BTreeSet;
use std::path::Path;

/// The owners to search, and every registered `owner/repo` in lower case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    /// Lower case, sorted, without repeats.
    pub owners: Vec<String>,
    pub registered: BTreeSet<String>,
}

/// The scope of the address book. Only repositories on github.com count: the search runs against
/// the default host, and an owner of another host would search the wrong one.
pub fn scope(root: &Path) -> Scope {
    let mut owners = BTreeSet::new();
    let mut registered = BTreeSet::new();
    for address in crate::registry::addresses(root) {
        let Some((owner, _)) = address.nwo.split_once('/') else {
            continue;
        };
        match crate::kernel::identity::origin_host_read(&address.main) {
            Ok(Some(host)) if host == "github.com" => {}
            Ok(_) => continue,
            // Git could not answer. An `owner/repo` came from an origin once; one owner too many
            // is harmless to search, and dropping it would lose its requests and mark its records
            // unregistered.
            Err(_) => {}
        }
        owners.insert(owner.to_ascii_lowercase());
        registered.insert(address.nwo.to_ascii_lowercase());
    }
    Scope {
        owners: owners.into_iter().collect(),
        registered,
    }
}
