use super::claim_slot::{claim_worker_slot, open_worker_tab};
use crate::infra::paths::exe_path;
use crate::infra::terminal::{self, SpawnRequest};
use crate::kernel::identity;
use crate::lifecycle::{forwarded_env, resume_template, title_command};
use crate::registry::{self, Context};

/// What starting or reopening a worker came to.
pub enum Started {
    /// The tab was opened (or, on a dry run, would be).
    Opened(terminal::Performed),
    /// `maxWorkers` is reached; the refusal says so. Kept apart from `Err` because the command
    /// line gives it its own exit code.
    Full(String),
}

/// Reopen the worker session saved in `worktree` in a new tab, without saying anything: the
/// command line prints what came of it and the board puts it in a reply.
///
/// `ctx` is the caller's to build. It has to address no hub of its own (see `work_resumed`),
/// and this does not look at the process's directory: the resident server runs nowhere near
/// the repository.
pub fn resume_worker(
    ctx: &Context,
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    worktree: &str,
    title: &str,
    prompt: Option<&str>,
    dry_run: bool,
) -> Result<Started, String> {
    let worktree = worker_worktree(Some(worktree))?;
    // Refused here rather than in the tab, so the caller — often a hub — hears about it.
    let saved = saved_worker_session(&worktree)?;
    resume_template(
        ctx.settings.agent_resume_runner.as_deref(),
        "agentResumeRunner",
    )?;
    // A reopened worker is as much a process as a fresh one.
    if let Some(refusal) = claim_worker_slot(ctx, &worktree, dry_run)? {
        return Ok(Started::Full(refusal));
    }
    let worktree = worktree.to_string_lossy().to_string();
    let title = match title {
        "" => saved.title.as_deref().unwrap_or(""),
        given => given,
    };
    let mut parts = forwarded_env(&ctx.state);
    parts.extend([
        exe_path(),
        "worker".to_string(),
        "--resume".to_string(),
        "--worktree".to_string(),
        worktree.clone(),
    ]);
    if !title.is_empty() {
        parts.push(format!("--title={title}"));
    }
    if let Some(prompt) = prompt {
        parts.push(format!("--prompt={prompt}"));
    }
    if let Some(repo) = repo_arg {
        parts.push("--repo".to_string());
        parts.push(repo.to_string());
    }
    if let Some(hub) = hub_arg.map(str::trim).filter(|hub| !hub.is_empty()) {
        parts.push(format!("--hub={hub}"));
    }
    let name_it = title_command(&ctx.settings, title);
    open_worker_tab(
        ctx,
        &worktree,
        &SpawnRequest {
            cwd: &worktree,
            title,
            command: &crate::infra::template::sh_join(&parts),
            title_command: name_it.as_deref(),
        },
        dry_run,
    )
    .map(Started::Opened)
}

/// The worktree a worker runs in: the one named, or — for `--resume`, typed by a person
/// standing in it — the one this command was run from.
pub fn worker_worktree(worktree: Option<&str>) -> Result<std::path::PathBuf, String> {
    let worktree = match worktree {
        Some(path) => crate::infra::paths::expand_home(path),
        None => identity::current_worktree(None)
            .map(std::path::PathBuf::from)
            .ok_or("not inside a git worktree; pass --worktree")?,
    };
    if !worktree.is_dir() {
        return Err(format!("no such worktree: {}", worktree.display()));
    }
    Ok(worktree)
}

/// The session `--resume` reopens in `worktree`, or a refusal that says why there is none.
pub fn saved_worker_session(worktree: &std::path::Path) -> Result<registry::SavedSession, String> {
    registry::worker_session(worktree).ok_or_else(|| {
        format!(
            "no saved worker session in {}: a session is saved when a worker is started by \
             `adj work` with a runner that takes {{sessionId}} (the built-in one does)",
            worktree.display()
        )
    })
}
