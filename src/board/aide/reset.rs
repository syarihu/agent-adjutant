use std::path::Path;

use crate::board::Resident;
use crate::kernel::config::resolve_config;
use crate::kernel::identity::AIDE_CONFIG_KEY;
use crate::lifecycle::aide::{AideReset, ResetRefused, reset_aide};
use crate::lifecycle::hub::hub_startable;
use crate::registry::{self, ProcessTable, Recorded};

use super::{aide_row, is_busy};

/// What a reset answers: what the lifecycle did, and whether a turn was in progress when it
/// began, which the reset cut short.
#[derive(Debug, PartialEq)]
pub struct AideResetDone {
    pub reset: AideReset,
    pub was_busy: bool,
}

/// Replace the aide for the page. One at a time, as `open`.
pub fn reset(resident: &Resident) -> Result<AideResetDone, ResetRefused> {
    let _one_at_a_time = resident.aide.lock().unwrap_or_else(|e| e.into_inner());
    // Read before the stop: after it the row of the old agent is gone. Not when the reset is
    // going to be refused for lack of tmux, which asks nothing of the process table.
    let startable = resolve_config(AIDE_CONFIG_KEY)
        .map(|resolved| hub_startable(&resolved.settings.terminal))
        .unwrap_or(false);
    let was_busy = startable && busy(&resident.root);
    reset_aide(&resident.root).map(|reset| AideResetDone { reset, was_busy })
}

fn busy(root: &Path) -> bool {
    let Recorded::Found(record) = registry::aide_record(root) else {
        return false;
    };
    aide_row(root, &ProcessTable::each(), &record)
        .is_some_and(|row| is_busy(&row, crate::infra::clock::now_secs()))
}
