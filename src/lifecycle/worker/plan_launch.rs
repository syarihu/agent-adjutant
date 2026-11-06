use super::{saved_worker_session, worker_worktree};
use crate::kernel::{identity, runner};
use crate::lifecycle::{Hooks, agent_hooks_for, resume_template};
use crate::registry::{self, Context, SavedSession, agent_env, context_as, context_of};
use std::path::PathBuf;

/// What `adj worker` was asked to start.
#[derive(Debug, Clone)]
pub struct WorkerRequest {
    pub repo: Option<String>,
    pub hub: Option<String>,
    /// `None`: the worktree this command was run from.
    pub worktree: Option<String>,
    pub title: Option<String>,
    pub task: Option<String>,
    pub prompt: Option<String>,
    pub resume: bool,
    pub model: Option<String>,
}

/// What starting the worker came to.
pub enum Planned {
    Running { pid: Option<u32> },
    // Boxed because a `Launch` carries a whole `Context`.
    Launch(Box<Launch>),
}

/// The line to run, and what registering it will write down.
#[derive(Clone)]
pub struct Launch {
    pub ctx: Context,
    pub worktree: PathBuf,
    pub title: String,
    pub task: Option<String>,
    pub command: String,
    /// The session a fresh start records, when its runner has one to record.
    pub fresh_session: Option<String>,
    /// The saved session a `--resume` reopens.
    pub resumed: Option<SavedSession>,
    /// What came of the hook settings the runner may take; `Skipped` is for the caller to say.
    pub hooks: Hooks,
    pub model: Option<String>,
    pub model_note: Option<String>,
}

/// Start the worker agent in the tab `work` just opened.
///
/// The mirror image of `hub`: write down who we are, then become the agent. Running the
/// agent as a child instead would record a PID that exits the moment the agent does
/// anything, and waking a dead launcher wakes nobody.
///
/// This is the deciding half: nothing is registered. When a worker is already running here
/// it gives back the slot `adj work` marked and returns `Running`, saying nothing.
pub fn plan_launch(request: &WorkerRequest) -> Result<Planned, String> {
    // A blank model means none was chosen, so a resume falls back to the saved one.
    let requested = request.model.as_deref().filter(|m| !m.trim().is_empty());
    if let Some(m) = requested {
        runner::check_model(m)?;
    }
    let worktree = worker_worktree(request.worktree.as_deref())?;
    let resumed = match request.resume {
        true => Some(saved_worker_session(&worktree)?),
        false => None,
    };
    let task = request
        .task
        .clone()
        .or_else(|| resumed.as_ref().and_then(|saved| saved.task.clone()));
    // This tab was opened *at* the worktree, so `context` would read the record this is
    // about to replace. A worker that crashed without being closed leaves one behind, and
    // re-dispatching that task would file the new worker under the hub that ran the old.
    //
    // A resumed worker goes back under the hub that dispatched it, which the saved session
    // remembers — ahead of `ADJUTANT_HUB`, because the tab someone types `--resume` into
    // may have inherited that from a different hub entirely. Only an explicit `--hub`
    // outranks it.
    let ctx = match &resumed {
        Some(saved) => {
            let told = request
                .hub
                .as_deref()
                .map(str::trim)
                .filter(|hub| !hub.is_empty());
            context_of(identity::resolve(
                request.repo.as_deref(),
                told.or(saved.hub.as_deref()),
            )?)?
        }
        None => context_as(request.repo.as_deref(), request.hub.as_deref())?,
    };
    let status = registry::worker_status(&worktree);
    if status.present {
        // `adj work` marked this worktree on the way here, and nobody is going to register
        // over it. Left, it would hold a second slot for the grace period after the running
        // worker ends.
        let _ = registry::unmark_worker_starting(&worktree);
        return Ok(Planned::Running { pid: status.pid });
    }

    let worktree_text = worktree.to_string_lossy().to_string();
    let title = request
        .title
        .as_deref()
        .filter(|title| !title.is_empty())
        .or(resumed.as_ref().and_then(|saved| saved.title.as_deref()))
        .unwrap_or("")
        .to_string();
    let prompt = request.prompt.as_deref();
    // The hook settings are a file, not a record: written on a dry run too, and the same
    // bytes every time for one binary.
    let (configured, default, config_key) = match &resumed {
        Some(_) => (
            resume_template(
                ctx.settings.agent_resume_runner.as_deref(),
                "agentResumeRunner",
            )?,
            runner::DEFAULT_AGENT_RESUME_RUNNER,
            "agentResumeRunner",
        ),
        None => (
            ctx.settings.agent_runner.as_deref(),
            runner::DEFAULT_AGENT_RUNNER,
            "agentRunner",
        ),
    };
    let template = configured.unwrap_or(default);

    let is_explicit = requested.is_some();
    let candidate = requested
        .map(str::to_string)
        .or_else(|| resumed.as_ref().and_then(|saved| saved.model.clone()));

    let mut model_to_pass: Option<String> = None;
    let mut launch_model: Option<String> = None;
    let mut model_note: Option<String> = None;

    if let Some(m) = candidate {
        if !is_explicit && let Err(err) = runner::check_model(&m) {
            model_note = Some(format!("the saved model {m} is not passed: {err}"));
            launch_model = Some(m);
        } else {
            match runner::model_slot(template) {
                runner::ModelSlot::Placeholder | runner::ModelSlot::Appended => {
                    model_to_pass = Some(m.clone());
                    launch_model = Some(m);
                }
                runner::ModelSlot::Refused(reason) => {
                    if is_explicit {
                        return Err(format!("{config_key} {reason}"));
                    } else {
                        model_note = Some(format!(
                            "the saved model {m} is not passed: {config_key} {reason}"
                        ));
                        launch_model = Some(m);
                    }
                }
            }
        }
    }

    let hooks = agent_hooks_for(&ctx.state, template);
    let settings = hooks.path().map(|path| path.to_string_lossy());
    let settings = settings.as_deref();
    let (command, fresh_session) = match &resumed {
        Some(saved) => {
            let command = runner::worker_resume_command(
                configured,
                &agent_env(&ctx),
                &saved.session_id,
                prompt.unwrap_or(runner::WORKER_RESUME_PROMPT),
                &worktree_text,
                &title,
                settings,
                model_to_pass.as_deref(),
            );
            (command, None)
        }
        None => {
            let session = registry::new_session_id()?;
            let command = runner::worker_command(
                configured,
                &agent_env(&ctx),
                &session,
                prompt.unwrap_or(runner::WORKER_STARTUP_PROMPT),
                &worktree_text,
                &title,
                settings,
                model_to_pass.as_deref(),
            );
            let records = runner::records_session(
                ctx.settings.agent_runner.as_deref(),
                runner::DEFAULT_AGENT_RUNNER,
            );
            (command, records.then_some(session))
        }
    };
    Ok(Planned::Launch(Box::new(Launch {
        ctx,
        worktree,
        title,
        task,
        command,
        fresh_session,
        resumed,
        hooks,
        model: launch_model,
        model_note,
    })))
}
