use crate::infra::paths::exe_path;
use crate::infra::template::sh_join;
use crate::infra::terminal::{self, Performed, SpawnRequest};
use crate::lifecycle::{forwarded_env, title_command};
use crate::others::Claim;
use crate::registry::Context;

/// What opening a window answers: what was done, and the pane when one can be watched.
type Spawned = Result<(Performed, Option<String>), String>;

/// Open a window that runs the reader of `claim`, in a directory of its own under the review
/// hub's home. `Err` when the window could not be opened or closed at once, with what it printed.
pub fn launch_reader(ctx: &Context, claim: &Claim) -> Result<(), String> {
    launch_reader_with(ctx, claim, &|req| {
        terminal::spawn_held(&ctx.settings.terminal, req, false)
    })
}

/// `launch_reader`, with the way a window is opened handed in.
pub(crate) fn launch_reader_with(
    ctx: &Context,
    claim: &Claim,
    spawn: &dyn Fn(&SpawnRequest) -> Spawned,
) -> Result<(), String> {
    let scratch = &claim.dir;
    std::fs::create_dir_all(scratch)
        .map_err(|e| format!("cannot create {}: {e}", scratch.display()))?;
    // A new session starts from nothing: what an earlier reader left must not be taken for what
    // this one writes, or handed in as its.
    crate::infra::fs::remove_if_present(&scratch.join("read-through.json"))?;
    let mut parts = forwarded_env(&ctx.state);
    parts.extend([
        exe_path(),
        "review".to_string(),
        "run".to_string(),
        "--pr".to_string(),
        claim.id.clone(),
        "--session".to_string(),
        claim.session_id.clone(),
    ]);
    let repo_and_number = claim.id.rsplit('/').next().unwrap_or(&claim.id);
    let title = format!("adj-read {repo_and_number}");
    let name_it = title_command(&ctx.settings, &title);
    let cwd = scratch.to_string_lossy();
    let (_, pane) = spawn(&SpawnRequest {
        cwd: &cwd,
        title: &title,
        command: &sh_join(&parts),
        title_command: name_it.as_deref(),
    })?;
    // A reader that cannot start exits at once and its window was kept to read what it printed.
    let Some(pane) = pane else {
        return Ok(());
    };
    match terminal::tmux_watch_start(ctx.settings.terminal.tmux_socket(), &pane)? {
        terminal::StartWatch::Alive => Ok(()),
        terminal::StartWatch::Exited { status, screen } => {
            let status = status.map(|s| format!(" (status {s})")).unwrap_or_default();
            Err(match screen.as_deref() {
                Some(screen) if !screen.is_empty() => {
                    format!("the reader exited at once{status}: {screen}")
                }
                _ => format!("the reader exited at once{status} without a message"),
            })
        }
        terminal::StartWatch::Gone => {
            Err("the reader's window closed at once, before it could be read".to_string())
        }
    }
}
