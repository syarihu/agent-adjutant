use std::os::unix::process::CommandExt;
use std::path::Path;

use crate::infra::agent::Agent;
use crate::infra::clock::now_secs;
use crate::infra::env::REVIEW_SESSION_ENV;
use crate::infra::terminal;
use crate::kernel::runner;
use crate::lifecycle::review::start_watcher;
use crate::lifecycle::{agent_command, agent_hooks_for};
use crate::mail::wake_agent;
use crate::others::{self, ReadMode};
use crate::registry::{self, agent_env};
use crate::task::PrRef;

/// Become the reader of `pr` that runs as `session`: write down this process and its window,
/// then replace this process with the agent. Returns only when it cannot, with what to tell the
/// person.
///
/// Just before the agent takes over, a detached watcher (`settle_reader`) is started for it when
/// the reader is in a tmux pane and runs Claude Code or agy: those ask whether to trust the
/// directory they start in, which nobody is there to answer. A `reviewerRunner` of any other
/// agent, or a reader outside tmux, has no watcher and runs as it always did.
///
/// The reader runs with the `agentEnv` of the repository `pr` belongs to, as that repository's
/// workers do, and not the code-review hub's own.
///
/// Refused when the record no longer names this session (the read was asked for again, or it
/// was failed), so a superseded launch starts no agent.
pub fn run_reader(root: &Path, pr: &PrRef, session: &str) -> String {
    let ctx = match registry::review_context_for(root, &pr.nwo()) {
        Ok(ctx) => ctx,
        Err(e) => return e,
    };
    let id = others::pr_id(pr);
    let stem = others::reader_stem(pr);
    let pid = std::process::id();
    let location = terminal::own_location(&ctx.settings.terminal);
    if let Err(e) = others::note_reader(
        root,
        pr,
        session,
        pid,
        registry::ps_started(pid),
        Some(location.clone()),
    ) {
        return e;
    }
    // The reader works in its own directory, whatever this was started from.
    let scratch = others::reader_dir(root, pr);
    if let Err(e) =
        std::fs::create_dir_all(&scratch).and_then(|()| std::env::set_current_dir(&scratch))
    {
        let why = format!("cannot work in {}: {e}", scratch.display());
        let _ = others::fail_read(root, pr, Some(session), &why, now_secs());
        return why;
    }
    let template = ctx.settings.reviewer_runner.as_deref();
    let runner_line = template.unwrap_or(runner::DEFAULT_REVIEWER_RUNNER);
    let agent = wake_agent(Some(runner_line));
    let env = launch_env(&ctx, agent, session);
    let hooks = agent_hooks_for(&ctx.state, runner_line);
    let settings = hooks.path().map(|path| path.to_string_lossy());
    // The mode is the record's, as the request left it: what asked is not the reader's to say.
    let mode = match others::record(root, pr) {
        Ok(record) => ReadMode::of(record.as_ref().and_then(|r| r.ai.as_ref())),
        Err(why) => {
            let _ = others::fail_read(root, pr, Some(session), &why, now_secs());
            return why;
        }
    };
    let prompt = reader_prompt(&id, mode);
    let command = runner::reviewer_command(
        template,
        &env,
        &format!("adjutant-read-{stem}"),
        session,
        &prompt,
        settings.as_deref(),
    );
    if let (Some(pane), true) = (location.pane.as_deref(), agent != Agent::Generic) {
        start_watcher(pr, session, pane, location.socket.as_deref());
    }
    let error = agent_command(&command).exec();
    let why = format!("cannot start the reader: {error}");
    let _ = others::fail_read(root, pr, Some(session), &why, now_secs());
    why
}

/// The whole environment of the reader's agent: `reader_env` over what the context's `agentEnv`
/// gives, and the session the record names.
fn launch_env(ctx: &registry::Context, agent: Agent, session: &str) -> Vec<(String, String)> {
    let mut env = reader_env(agent_env(ctx), agent);
    env.push((REVIEW_SESSION_ENV.to_string(), session.to_string()));
    env
}

/// Claude Code's setting for its fullscreen terminal UI.
const FULLSCREEN_ENV: &str = "CLAUDE_CODE_NO_FLICKER";

/// The environment the reader's agent gets: `env` (the person's `agentEnv`), and for Claude Code
/// the fullscreen UI too unless `agentEnv` already says. The reader's settings sources are
/// dropped, and with them a person's `"tui": "fullscreen"`, which would leave the inline UI;
/// that asks for no mouse, so the board's terminal turns the wheel into arrow keys that scroll
/// the input history and not the conversation. A second `--settings` would replace the hooks
/// file, hence the variable.
fn reader_env(mut env: Vec<(String, String)>, agent: Agent) -> Vec<(String, String)> {
    if agent == Agent::Claude && !env.iter().any(|(k, _)| k == FULLSCREEN_ENV) {
        env.push((FULLSCREEN_ENV.to_string(), "1".to_string()));
    }
    env
}

/// What the reader's agent is told to do: the procedure is fetched, and the mode picks the part
/// of it that applies.
fn reader_prompt(id: &str, mode: ReadMode) -> String {
    match mode {
        ReadMode::Read => format!(
            "Fetch the adj-reviewer procedure with `adj skill adj-reviewer` (no MCP tools are available to you), and read the pull request {id} as it says."
        ),
        ReadMode::Split => format!(
            "Fetch the adj-reviewer procedure with `adj skill adj-reviewer` (no MCP tools are available to you), and draft a split proposal for the pull request {id} as its section \"Split proposal\" says."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn claude_gets_the_fullscreen_ui_once_unless_agent_env_says() {
        let default = wake_agent(Some(runner::DEFAULT_REVIEWER_RUNNER));
        assert_eq!(
            reader_env(env_of(&[("A", "b")]), default),
            env_of(&[("A", "b"), ("CLAUDE_CODE_NO_FLICKER", "1")])
        );
        let own = env_of(&[("CLAUDE_CODE_NO_FLICKER", "0")]);
        assert_eq!(reader_env(own.clone(), Agent::Claude), own);
    }

    #[test]
    fn a_claude_reader_is_given_its_repositorys_agent_env_and_its_session() {
        let sandbox = crate::testing::Sandbox::new(
            r#"{"repos": {"acme/widget": {"agentEnv": {"CLAUDE_CONFIG_DIR": "/cfg/widget"}}}}"#,
        );
        let root = sandbox.state();
        let ctx = registry::review_context_for(&root, "acme/widget").unwrap();
        let env = launch_env(&ctx, Agent::Claude, "s1");
        for pair in [
            ("CLAUDE_CONFIG_DIR", "/cfg/widget"),
            ("CLAUDE_CODE_NO_FLICKER", "1"),
            (REVIEW_SESSION_ENV, "s1"),
            ("ADJUTANT_HUB", "@review"),
        ] {
            assert!(env.contains(&env_of(&[pair])[0]), "{pair:?} in {env:?}");
        }
    }

    #[test]
    fn another_agent_is_given_no_claude_setting() {
        let agy = wake_agent(Some("agy -i {prompt}"));
        let other = wake_agent(Some("codex exec {prompt}"));
        assert!(reader_env(Vec::new(), agy).is_empty());
        assert!(reader_env(Vec::new(), other).is_empty());
    }

    #[test]
    fn the_prompt_names_the_split_section_only_for_a_split() {
        let read = reader_prompt("o/r#1", ReadMode::Read);
        assert!(read.contains("read the pull request o/r#1 as it says"));
        assert!(!read.contains("Split proposal"));
        let split = reader_prompt("o/r#1", ReadMode::Split);
        assert!(split.contains("draft a split proposal for the pull request o/r#1"));
        assert!(split.contains("\"Split proposal\""));
    }
}
