use crate::board::Resident;
use crate::lifecycle::aide::{AideOpened, AideRefused, open_aide};

/// Open the aide for the page, unless one is running or starting. One at a time: a second
/// request waits for the first and then finds its aide, rather than starting another.
pub fn open(resident: &Resident) -> Result<AideOpened, AideRefused> {
    let _one_at_a_time = resident.aide.lock().unwrap_or_else(|e| e.into_inner());
    open_aide(&resident.root)
}
