use super::claim_slot::{claim_worker_slot, open_worker_tab};
use super::resume::Started;
use crate::infra::paths::exe_path;
use crate::infra::terminal::SpawnRequest;
use crate::kernel::runner;
use crate::lifecycle::{forwarded_env, title_command};
use crate::registry::Context;

/// What a fresh worker is dispatched with.
pub struct StartRequest {
    /// The directory the worker works in. `~` is expanded.
    pub worktree: String,
    /// The title typed on the command line; empty when none was, and the title is then taken
    /// from the task's record.
    pub title: String,
    /// The task the worker is dispatched for.
    pub task: Option<String>,
    /// What the agent is first told; `None` for the standard startup prompt.
    pub prompt: Option<String>,
    /// The `--repo` the tab's command line carries, so the worker reads the same repository.
    pub repo: Option<String>,
}

/// Open a tab and start a worker agent in it. One command rather than two so the runner
/// template is read in exactly one place.
///
/// Says nothing: the command line prints what came of it.
///
/// `ctx` is the dispatching side's own (see `work`): the hub being handed to the new worker
/// is the caller's, never one read out of the worktree.
pub fn start(ctx: &Context, request: &StartRequest, dry_run: bool) -> Result<Started, String> {
    let prompt = request
        .prompt
        .as_deref()
        .unwrap_or(runner::WORKER_STARTUP_PROMPT);
    let task_id = request.task.as_deref();
    // Named after the task's record rather than a title typed on the command line. The title
    // comes from an issue or a report, and quoted into the hub's shell it could close the
    // quote; the record's id is one this tool generated.
    let from_record;
    let title = match task_id {
        Some(id) if request.title.is_empty() => {
            from_record = crate::task::get(&ctx.state, &ctx.repo.slug, id)?.title;
            from_record.as_str()
        }
        _ => request.title.as_str(),
    };
    let worktree = crate::infra::paths::expand_home(&request.worktree)
        .to_string_lossy()
        .to_string();
    // Asked here and not left to the spawn, because marking the slot writes into the
    // worktree and would create the very directory the spawn checks for — a mistyped path
    // would then open a tab in an empty directory outside any repository.
    if !std::path::Path::new(&worktree).is_dir() {
        return Err(format!("no such directory: {worktree}"));
    }
    if let Some(refusal) = claim_worker_slot(ctx, std::path::Path::new(&worktree), dry_run)? {
        return Ok(Started::Full(refusal));
    }
    // The tab runs `adjutant worker`, not the agent directly. The agent is started by a
    // process that has already written down its own PID and then `exec`s itself away, which
    // is the only way anyone later gets to ask "is that worker still there".
    let mut parts = forwarded_env(&ctx.state);
    parts.extend([
        exe_path(),
        "worker".to_string(),
        "--worktree".to_string(),
        worktree.clone(),
        // `=` rather than a separate word, as in `title_command`: a title from an issue that
        // starts with `--` would otherwise be parsed as an option and the worker never start.
        format!("--title={title}"),
        format!("--prompt={prompt}"),
    ]);
    if let Some(repo) = &request.repo {
        parts.push("--repo".to_string());
        parts.push(repo.clone());
    }
    // The *resolved* identifier rather than the flag, because a hub dispatching work runs
    // this as its own child and so usually passes no flag at all — it is carrying the
    // answer in its environment. That environment does not survive the trip: the tab is
    // opened by the terminal, which is handed a command line and nothing else. So the
    // answer goes onto the command line, or the worker registers under the wrong hub and
    // reports to an inbox nobody reads.
    // One argument rather than two: an identifier that starts with a dash reaches here from
    // `ADJUTANT_HUB`, where no flag parser has seen it, and as a separate word clap reads it
    // as the next option instead of as this one's value.
    if let Some(hub) = &ctx.repo.hub {
        parts.push(format!("--hub={hub}"));
    }
    if let Some(task) = task_id {
        parts.push(format!("--task={task}"));
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
