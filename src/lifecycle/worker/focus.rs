use crate::infra::terminal;
use crate::kernel::config::Settings;
use crate::registry;

/// Bring the tab of the worker in `worktree` to the front, through `terminal.focus` like the
/// hub's. `Ok(false)` when no worker is running there — nothing to raise.
pub fn focus_worker(
    settings: &Settings,
    worktree: &std::path::Path,
    dry_run: bool,
) -> Result<Option<terminal::Performed>, String> {
    let status = registry::worker_status(worktree);
    let Some(pid) = status.pid.filter(|_| status.present) else {
        return Ok(None);
    };
    // The name the tab actually carries, as `close` hands it over: `spawn` put the record's
    // title through `sanitise_title`, and a template that finds a tab by name needs that.
    let title = terminal::sanitise_title(
        status.title.as_deref().unwrap_or_default(),
        worktree
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
    );
    terminal::focus(
        &settings.terminal,
        status.terminal.as_ref(),
        pid,
        &title,
        dry_run,
    )
    .map(Some)
}
