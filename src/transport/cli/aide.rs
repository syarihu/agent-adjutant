//! `adj aide run`: what the aide's window runs.

use super::args::AideRunArgs;
use crate::lifecycle;
use crate::registry;

/// Become the aide's agent. Returns only when that cannot be done, with what to tell the person.
pub fn aide_run(args: &AideRunArgs) -> Result<(), String> {
    let root = registry::state_root(None);
    Err(lifecycle::aide::run_aide(&root, &args.session))
}
