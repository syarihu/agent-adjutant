use super::*;

/// Everything a command needs to know about where it is. Resolved once, at the top, because
/// two commands disagreeing about which repo they are in is the failure that loses reports.
#[derive(Clone)]
pub struct Context {
    pub repo: RepoInfo,
    pub settings: Settings,
    pub resolved: config::Resolved,
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
pub(super) fn context_as(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<Context, String> {
    let told = messaging::hub_id_told(hub_arg);
    let ctx = context_of(identity::resolve(repo_arg, told.as_deref())?)?;
    if told.is_some() {
        return Ok(ctx);
    }
    match messaging::hub_id_configured(&ctx.settings.agent_env) {
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
pub(super) fn agent_env(ctx: &Context) -> Vec<(String, String)> {
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
pub(super) fn context_without_hub(repo_arg: Option<&str>) -> Result<Context, String> {
    context_of(identity::resolve(repo_arg, None)?)
}

pub fn context_of(repo: RepoInfo) -> Result<Context, String> {
    // By `owner/name` and nothing else. The hub identifier moves the address; it must not
    // move the lookup, or asking for a second hub of a registered repository would answer
    // with an unregistered one — no task sources, no issue keys, no verify command.
    let resolved = config::resolve_config(&repo.nwo)?;
    Ok(Context {
        settings: resolved.settings.clone(),
        repo,
        resolved,
    })
}

/// Where we are, and which hub of it we are talking to.
///
/// Every subcommand that addresses a hub goes through here rather than calling
/// `identity::resolve` with whatever it was given: deciding between the flag, the environment
/// and the worktree is one rule, and a second copy of it is a second answer.
pub(super) fn resolve(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<RepoInfo, String> {
    identity::resolve(repo_arg, messaging::hub_id(hub_arg, None)?.as_deref())
}

// ── hub-name ─────────────────────────────────────────────────────────

pub fn hub_name(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    as_json: bool,
) -> Result<(), String> {
    let info = resolve(repo_arg, hub_arg)?;
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "main": info.main,
                "nwo": info.nwo,
                "repo": info.repo,
                // Which hub of the repository this address belongs to, as it was resolved
                // — `null` for the repository's own. Printed because it is the only way to
                // see, from outside, which of the three answers won.
                "hub": info.hub,
                "slug": info.slug,
                "hubName": info.hub_name,
                "nwoSource": info.nwo_source,
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }
    if info.nwo_source == "dirname" {
        eprintln!(
            "adjutant: origin gave no repository name, using the directory name {}",
            info.nwo
        );
    }
    println!("{}", info.hub_name);
    Ok(())
}

// ── config ───────────────────────────────────────────────────────────

pub fn show_config(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<(), String> {
    let ctx = context(repo_arg, hub_arg)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "repo": ctx.repo.nwo,
            "main": ctx.repo.main,
            "hub": ctx.repo.hub,
            "hubName": ctx.repo.hub_name,
            "board": board_json(&ctx.repo),
            "registered": ctx.resolved.registered,
            "configPath": ctx.resolved.config_path,
            "warnings": ctx.resolved.warnings,
            "settings": ctx.settings,
            "config": ctx.resolved.config,
        }))
        .unwrap_or_default()
    );
    Ok(())
}

/// This binary, for commands that have to name themselves in a command line handed to a
/// terminal. The absolute path rather than `adjutant`, so a new tab whose PATH is not yet
/// loaded still finds it.
pub(super) fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "adjutant".to_string())
}

/// Settings without insisting on a resolvable repository.
///
/// `spawn` and `notify` are useful from anywhere, including outside a checkout, and failing
/// them because `git` had nothing to say would be answering a question nobody asked.
pub(super) fn settings_for(repo_arg: Option<&str>) -> Settings {
    let nwo = match repo_arg {
        Some(arg) => arg.to_string(),
        None => identity::resolve(None, None)
            .map(|i| i.nwo)
            .unwrap_or_default(),
    };
    config::resolve_config(&nwo)
        .map(|r| r.settings)
        .unwrap_or_default()
}
