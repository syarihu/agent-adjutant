use super::*;

/// Where messages for this hub wait. One directory, whether or not the hub is running: two
/// would mean the hub has to remember to read both, and the one it forgets is the one that
/// silently swallows reports.
pub fn inbox_dir(slug: &str) -> PathBuf {
    state_dir().join("inbox").join(slug)
}

/// Where a message goes once the hub has acted on it. Kept rather than deleted so a report
/// that was mishandled can still be found.
pub fn archive_dir(slug: &str) -> PathBuf {
    inbox_dir(slug).join("read")
}
