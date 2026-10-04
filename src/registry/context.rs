//! `Context`: where a command is, resolved once.

use super::*;
use crate::kernel::config::{self, Settings};
use crate::kernel::identity::{self, RepoInfo};

/// Everything a command needs to know about where it is. Resolved once, at the top, because
/// two commands disagreeing about which repo they are in is the failure that loses reports.
#[derive(Clone)]
pub struct Context {
    pub repo: RepoInfo,
    pub settings: Settings,
    pub resolved: config::Resolved,
    /// The state directory every record of this repository is read from: `state_root` taken
    /// against the main checkout, so a command reads the directory `adj hub` reads wherever it
    /// was typed.
    pub state: PathBuf,
}

/// Where we are, for a command addressing a hub: sending to it, listing its inbox, naming
/// it, bringing it forward.
pub fn context(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<Context, String> {
    context_of(resolve(repo_arg, hub_arg)?)
}

/// The same, for a command that *starts or registers* a hub rather than addressing one.
///
/// The difference is the worker record. A hub launched from inside a worktree, and a worker
/// registering in the worktree its tab was opened at, would both read a record that belongs
/// to somebody else — or, for the worker, the one it is a moment away from overwriting. See
/// `messaging::hub_id_told`.
///
/// Told nothing, it takes the identifier `agentEnv` names, because that is the one the agent
/// it starts will be given. Only then: a flag or `ADJUTANT_HUB` outranks the config, and
/// `agent_env` puts the answer back on the agent's line so the two still agree.
pub(crate) fn context_as(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<Context, String> {
    let told = hub_id_told(hub_arg);
    let ctx = context_of(identity::resolve(repo_arg, told.as_deref())?)?;
    if told.is_some() {
        return Ok(ctx);
    }
    match hub_id_configured(&ctx.settings.agent_env) {
        Some(hub) => Ok(Context {
            repo: ctx.repo.addressed(Some(&hub))?,
            ..ctx
        }),
        None => Ok(ctx),
    }
}

/// The environment an agent is started with: `agentEnv`, with `ADJUTANT_HUB` set to the hub
/// this invocation claimed and to nothing else.
///
/// Replaced rather than appended after the config, so that an agent started for the
/// repository's own hub is not handed an identifier the config names — the resumed worker of
/// a hub that predates the key is one. The launch also removes the variable from the
/// environment it passes on (see `hub` and `worker`), so a value inherited from whatever
/// opened the tab is never the one the agent reads.
pub(crate) fn agent_env(ctx: &Context) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = ctx
        .settings
        .agent_env
        .iter()
        .filter(|(key, _)| key != crate::infra::env::HUB_ENV)
        .cloned()
        .collect();
    if let Some(hub) = &ctx.repo.hub {
        env.push((crate::infra::env::HUB_ENV.to_string(), hub.clone()));
    }
    env
}

/// The same, for a command that needs the settings and the checkout and no hub at all.
///
/// `ide` and `worktree-path` never read a hub field, and neither takes `--hub`. Sending them
/// through the addressing path would make an unreadable worker record stop them — and an
/// unreadable record is exactly the state of the worktree somebody is trying to open an
/// editor on. Strictness belongs where a wrong answer misroutes something.
pub(crate) fn context_without_hub(repo_arg: Option<&str>) -> Result<Context, String> {
    context_of(identity::resolve(repo_arg, None)?)
}

/// The state directory, absolute. A relative `ADJUTANT_STATE_DIR` is taken against `anchor` — a
/// repository's main checkout — when given, else against the working directory. The one place
/// outside `infra` that asks the environment for it.
pub fn state_root(anchor: Option<&Path>) -> PathBuf {
    let dir = state_dir();
    let joined = match anchor {
        Some(anchor) => anchor.join(&dir),
        None => dir,
    };
    std::path::absolute(&joined).unwrap_or(joined)
}

pub fn context_of(repo: RepoInfo) -> Result<Context, String> {
    let state = state_root(Some(Path::new(&repo.main)));
    context_at(repo, state)
}

/// The context of `repo` over the state directory `state`, for a caller that already holds the
/// root it reads (the resident server opens every board over its own).
pub fn context_at(repo: RepoInfo, state: PathBuf) -> Result<Context, String> {
    // By `owner/name` and nothing else. The hub identifier moves the address; it must not
    // move the lookup, or asking for a second hub of a registered repository would answer
    // with an unregistered one — no task sources, no issue keys, no verify command.
    let resolved = config::resolve_config(&repo.nwo)?;
    Ok(Context {
        settings: resolved.settings.clone(),
        repo,
        resolved,
        state,
    })
}

/// Where we are, and which hub of it we are talking to.
///
/// Every subcommand that addresses a hub goes through here rather than calling
/// `identity::resolve` with whatever it was given: deciding between the flag, the environment
/// and the worktree is one rule, and a second copy of it is a second answer.
pub(crate) fn resolve(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<RepoInfo, String> {
    identity::resolve(repo_arg, hub_id(hub_arg, None)?.as_deref())
}
