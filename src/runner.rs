//! Starting a worker agent.
//!
//! The hub's job ends at "here is a worktree and here is a brief"; which program reads that
//! brief is not its business. Keeping the start command in the config is what lets a second
//! agent take the same task without a second copy of the hub's prompt.

use crate::template::{Sub, contains_placeholder, render, sh_quote};

/// Claude Code, in the mode a worker needs: it has to run a build and a test suite without
/// a human at the tab to approve each one.
pub const DEFAULT_AGENT_RUNNER: &str = "claude --permission-mode auto {prompt}";

/// The hub is a named session because the name is its address: a worker finds it by asking
/// for the hub name and sending there.
///
/// It starts unattended for the same reason a worker does, arrived at from the opposite
/// direction. A worker must not stop because it has a build to finish; a hub must not stop
/// because **a hub waiting for approval is a hub not reading its inbox** — and nobody is
/// watching that tab, which is the entire premise. The alternative is an allowlist of every
/// command the procedures reach for, which drifts the moment one of them reaches for
/// something new, and whose failure mode is the hub going quiet.
///
/// This does not remove the questions that matter. The procedure's own `AskUserQuestion`
/// checkpoints — file this issue? start work on it? — are unaffected; what goes away is
/// being asked whether `gh issue view` may run.
pub const DEFAULT_HUB_RUNNER: &str = "claude -n {name} --permission-mode auto {prompt}";

/// What the hub is told on startup.
///
/// What the hub is told on startup — naming two ways to fetch the procedure, and not a
/// slash command.
///
/// Prompt support differs between agents and between versions, so a hub that starts with an
/// unrecognised `/…` sits there doing nothing, which looks exactly like a hub that is up and
/// idle. Both a tool and a command are named because neither is universal: an MCP tool may
/// be deferred or absent, and a command needs the binary on PATH.
pub const HUB_STARTUP_PROMPT: &str = "adjutant_skill で name=adj-hub の手順書を取得（使えなければ `adj skill adj-hub`）して、その手順どおりに常駐 hub を開始するのだ";

/// The command line that starts a hub session called `name`.
pub fn hub_command(
    template: Option<&str>,
    env: &[(String, String)],
    name: &str,
    prompt: &str,
) -> String {
    let template = template.unwrap_or(DEFAULT_HUB_RUNNER);
    let mut command = render(
        template,
        &[("name", Sub::Quoted(name)), ("prompt", Sub::Quoted(prompt))],
    );
    if !contains_placeholder(template, "prompt") {
        command = format!("{command} {}", sh_quote(prompt));
    }
    with_env(env, command)
}

/// The command line that starts a worker, ready to hand to `terminal::spawn`.
///
/// `env` is prepended as `env K=V …` rather than being set on the child directly, because
/// the command does not run here — it is typed into a tab by the terminal, and only the
/// command line survives that trip.
pub fn worker_command(
    template: Option<&str>,
    env: &[(String, String)],
    prompt: &str,
    worktree: &str,
    title: &str,
) -> String {
    let template = template.unwrap_or(DEFAULT_AGENT_RUNNER);
    let mut command = render(
        template,
        &[
            ("prompt", Sub::Quoted(prompt)),
            ("worktree", Sub::Quoted(worktree)),
            ("title", Sub::Quoted(title)),
        ],
    );
    // A template with nowhere to put the prompt would start an agent with no instructions,
    // which looks like a hung worker rather than a config mistake. Appending is the reading
    // that matches every CLI agent's own argument order.
    if !contains_placeholder(template, "prompt") {
        command = format!("{command} {}", sh_quote(prompt));
    }
    with_env(env, command)
}

/// The config refuses a key that is not a variable name, which is where that belongs — but
/// this function is what turns a pair into a *shell line*, so it quotes the key as well as
/// the value. Anything that reaches here by another route is data, not syntax.
fn with_env(env: &[(String, String)], command: String) -> String {
    if env.is_empty() {
        return command;
    }
    let assignments: Vec<String> = env
        .iter()
        .map(|(k, v)| format!("{}={}", sh_quote(k), sh_quote(v)))
        .collect();
    format!("env {} {command}", assignments.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_runner_starts_a_worker_that_can_run_its_own_build() {
        assert_eq!(
            worker_command(None, &[], "go now", "/wt/wid-957", "WID-957"),
            "claude --permission-mode auto 'go now'"
        );
    }

    #[test]
    fn a_prompt_with_quotes_in_it_survives_the_trip_through_the_terminal() {
        let out = worker_command(
            None,
            &[],
            "read .claude/task-brief.md, it's there",
            "/wt",
            "T",
        );
        let echoed = std::process::Command::new("sh")
            .arg("-c")
            .arg(out.replace("claude --permission-mode auto", "printf %s"))
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&echoed.stdout),
            "read .claude/task-brief.md, it's there"
        );
    }

    #[test]
    fn another_agent_is_a_config_line_rather_than_a_code_change() {
        assert_eq!(
            worker_command(Some("codex exec '{prompt}'"), &[], "go", "/wt", "T"),
            "codex exec go"
        );
        assert_eq!(
            worker_command(
                Some("agy run {prompt} --cwd {worktree}"),
                &[],
                "go",
                "/wt/x",
                "T"
            ),
            "agy run go --cwd /wt/x"
        );
    }

    #[test]
    fn a_template_that_forgot_the_prompt_still_gets_one() {
        assert_eq!(
            worker_command(Some("myagent --resume"), &[], "go now", "/wt", "T"),
            "myagent --resume 'go now'"
        );
    }

    #[test]
    fn the_hub_is_started_by_name_because_the_name_is_the_address() {
        let out = hub_command(None, &[], "adjutant-acme-widget", HUB_STARTUP_PROMPT);
        assert!(out.starts_with("claude -n adjutant-acme-widget "), "{out}");
        assert!(out.contains("adj-hub"), "{out}");
        // Unattended by default: a hub stopped for approval is a hub not reading its inbox.
        assert!(out.contains("--permission-mode auto"), "{out}");
    }

    #[test]
    fn the_hub_startup_prompt_is_not_a_slash_command() {
        // An agent that does not recognise a slash command starts a session that sits there
        // doing nothing, which looks exactly like a hub that is up and idle.
        assert!(!HUB_STARTUP_PROMPT.trim_start().starts_with('/'));
        // Two ways in, because neither is universal.
        assert!(HUB_STARTUP_PROMPT.contains("adjutant_skill"));
        assert!(HUB_STARTUP_PROMPT.contains("adj skill"));
    }

    #[test]
    fn per_repo_env_is_prepended_without_naming_any_particular_agent() {
        assert_eq!(
            worker_command(
                None,
                &[("CLAUDE_CONFIG_DIR".into(), "/cfg/app one".into())],
                "go",
                "/wt",
                "T"
            ),
            "env CLAUDE_CONFIG_DIR='/cfg/app one' claude --permission-mode auto go"
        );
    }

    #[test]
    fn an_env_key_is_quoted_like_everything_else_on_the_line() {
        // The config rejects a key like this before it gets here; this is the second lock
        // on the same door, for a caller that did not arrive through the config.
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("pwned");
        let out = worker_command(
            None,
            &[(format!(";touch {}", marker.display()), "v".into())],
            "go",
            "/wt",
            "T",
        );
        std::process::Command::new("sh")
            .arg("-c")
            .arg(out.replace("claude --permission-mode auto", "true"))
            .output()
            .unwrap();
        // The `;` is inside the assignment `env` is handed, not a separator the shell acts
        // on — so the injected command is a variable name nobody reads, and never a
        // command. (`env` itself is lenient about odd names, so what proves it is the file
        // that was never created, not an exit status.)
        assert!(!marker.exists(), "{out}");
        assert!(out.starts_with("env ';touch "), "{out}");
    }
}
