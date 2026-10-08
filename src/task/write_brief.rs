//! Writing the worker's `.claude/task-brief.md` from the task record and the settings.

use super::*;

use crate::kernel::brief::{self as text, Tracker};
use crate::registry::Context;

/// What a brief is written for, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefRequest {
    /// The worktree as given; resolved as `create` resolves one.
    pub worktree: String,
    /// What the branch was cut from, or `-`.
    pub base: String,
    /// Where to write it, as given (`~` expanded); `None` is `{worktree}/.claude/task-brief.md`.
    pub out: Option<String>,
    /// The language the person reads, from `--language`; used only when `settings.language` is not set.
    pub language: Option<String>,
    pub of: BriefOf,
}

/// A task's brief, or the brief of a session started with no task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BriefOf {
    /// `key`, `tracker` and `parent` win over what the record and the issue URL say.
    Task {
        id: String,
        key: Option<String>,
        tracker: Option<Tracker>,
        parent: Option<String>,
    },
    /// The instruction as given, already read from stdin when it was `-`.
    Session { instruction: String },
}

/// What was written: the file, and the branch it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brief {
    pub path: PathBuf,
    pub branch: String,
}

/// The Done when the brief says. The worker branches on three phrases only, and a task that
/// goes as far as handling review has opened its PR, so `Review` is written as the PR.
pub(super) fn brief_done_when(done_when: DoneWhen) -> &'static str {
    match done_when {
        DoneWhen::Review => DoneWhen::Pr.as_prose(),
        other => other.as_prose(),
    }
}

/// Write the worker's brief from the record and the settings. Every refusal comes before the
/// file is touched.
pub fn write_brief(ctx: &Context, request: &BriefRequest) -> Result<Brief, String> {
    let worktree = resolved_worktree(&request.worktree);
    let worktree = Path::new(&worktree);
    if !worktree.is_dir() {
        return Err(format!("no such worktree: {}", worktree.display()));
    }
    // Read from the worktree rather than taken as an argument: it is the branch the worker
    // will be on, and a typed one is a second answer to a question git already has.
    let branch = crate::infra::git::git(&["symbolic-ref", "-q", "--short", "HEAD"], Some(worktree))
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            format!(
                "{} is not on a branch: the brief names the branch the worker works on",
                worktree.display()
            )
        })?;
    let base = request.base.trim();
    if base.is_empty() {
        return Err("--base is empty: pass the commit-ish the worktree was cut from, or -".into());
    }

    // Written once for both kinds of brief. What the person set wins over the hub's guess at
    // it; `-` is "not known", and the worker then falls back to the language the person uses
    // with it.
    let flag = request
        .language
        .as_deref()
        .map(str::trim)
        .filter(|language| !language.is_empty());
    let language = ctx
        .settings
        .language
        .as_deref()
        .or(flag)
        .unwrap_or("-")
        .to_string();

    let rendered = match &request.of {
        BriefOf::Task {
            id,
            key,
            tracker,
            parent,
        } => {
            if let Some(parent) = parent {
                check_parent(parent)?;
            }
            // Written to a line the worker reads, and the hub puts it on a command line.
            if let Some(key) = key
                .as_deref()
                .filter(|key| *key != "-" && !text::is_key(key))
            {
                return Err(format!("not a tracker key: {key} (like ABC-123)"));
            }
            let record = get(&ctx.state, &ctx.repo.slug, id)?;
            if record.executor == Executor::Jules {
                return Err(format!(
                    "task {id} is handed to Jules: no worker is started, so no brief is written"
                ));
            }
            // An empty URL is none: `--issue-url ''` stores one.
            let nonblank = |url: &Option<String>| url.clone().filter(|url| !url.trim().is_empty());
            let url = nonblank(&record.issue_url).or_else(|| nonblank(&record.issue));
            let (key, tracker) = match (&url, key.as_deref(), *tracker) {
                (_, Some(key), Some(tracker)) => (key.to_string(), tracker),
                (None, key, tracker) => (
                    key.unwrap_or("-").to_string(),
                    tracker.unwrap_or(Tracker::None),
                ),
                (Some(url), key, tracker) => {
                    let (read_tracker, read_key) = text::tracker_and_key(
                        url,
                        &ctx.settings.task_sources,
                        &ctx.settings.issue_keys,
                    )
                    .ok_or_else(|| {
                        format!(
                            "cannot tell the tracker and key of {url}: pass --key and --tracker"
                        )
                    })?;
                    (
                        key.map_or(read_key, str::to_string),
                        tracker.unwrap_or(read_tracker),
                    )
                }
            };
            let parent = parent
                .clone()
                .or(record
                    .parent
                    .clone()
                    .filter(|parent| !parent.trim().is_empty()))
                .unwrap_or_else(|| "-".to_string());
            // The worker fetches the parent from its URL; a bare key says nothing of the tracker.
            // Checked again here because a record written before parents were checked on the way
            // in may hold anything.
            if parent != "-" {
                if text::is_key(&parent) {
                    return Err(format!(
                        "the parent task is a key ({parent}): find its URL and pass --parent '<URL>'"
                    ));
                }
                check_url(&parent, "a task")?;
            }
            text::render_task(&text::TaskBrief {
                key,
                title: record.title.clone(),
                tracker,
                url,
                request: record.body.clone(),
                branch: branch.clone(),
                base: base.to_string(),
                parent,
                record: record.id.clone(),
                done_when: brief_done_when(record.done_when).to_string(),
                stop_at: record.stop_at.as_str().to_string(),
                handover: record
                    .instruction
                    .clone()
                    .filter(|note| !note.trim().is_empty())
                    .unwrap_or_else(|| "-".to_string()),
                copilot_review: ctx.settings.copilot_review.as_str().to_string(),
                language,
                verify: ctx.settings.verify.clone(),
            })
        }
        BriefOf::Session { instruction } => {
            let instruction = match instruction.trim() {
                "" | "-" => text::NO_INSTRUCTION.to_string(),
                _ => instruction.clone(),
            };
            text::render_session(&text::SessionBrief {
                branch: branch.clone(),
                base: base.to_string(),
                language,
                instruction,
            })
        }
    };

    let path = match request.out.as_deref() {
        Some(out) => crate::infra::paths::expand_home(out),
        None => worktree.join(".claude").join("task-brief.md"),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    // Always written over: the brief is derived, and one left from an earlier start is the
    // stale answer this exists to replace.
    std::fs::write(&path, rendered).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(Brief { path, branch })
}
