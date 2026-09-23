//! Starting a worker agent.
//!
//! The hub's job ends at "here is a worktree and here is a brief"; which program reads that
//! brief is not its business. Keeping the start command in the config is what lets a second
//! agent take the same task without a second copy of the hub's prompt.

use crate::template::{Sub, contains_placeholder, render, sh_quote};

/// Claude Code, in the mode a worker needs: it has to run a build and a test suite without
/// a human at the tab to approve each one.
///
/// `--session-id` is what lets `adj worker --resume` find this conversation again: the id is
/// made up before the agent starts and written down next to the worker record, so the session
/// can be reopened by name rather than guessed at with "the most recent one in this directory".
pub const DEFAULT_AGENT_RUNNER: &str =
    "claude --session-id {sessionId} --permission-mode auto {prompt}";

/// The same worker, brought back. Only reached through `--resume`, with the id the start
/// command was given.
pub const DEFAULT_AGENT_RESUME_RUNNER: &str =
    "claude --resume {sessionId} --permission-mode auto {prompt}";

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
///
/// `--session-id` for the reason the worker has one: `adj hub --resume` reopens this exact
/// conversation. `claude --continue` would open whichever session last ran in the main
/// checkout, and that is as likely to be somebody's unrelated work as the hub.
pub const DEFAULT_HUB_RUNNER: &str =
    "claude -n {name} --session-id {sessionId} --permission-mode auto {prompt}";

/// The hub brought back. The name stays on the line so the session reads the same in every
/// listing after the restart as it did before it.
pub const DEFAULT_HUB_RESUME_RUNNER: &str =
    "claude -n {name} --resume {sessionId} --permission-mode auto {prompt}";

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

/// What a resumed hub is told. The conversation is all there, procedure included, so this
/// only has to cover what happened while it was gone: reports that landed with nobody to be
/// woken by them.
pub const HUB_RESUME_PROMPT: &str = "セッションを再開したのだ。止まっていた間に届いたものがあるかもしれないので、adjutant_pending で受信箱を確認して、溜まっていれば起動時と同じく振り分けてから待機に戻るのだ";

/// What a worker is told on startup, when the hub says nothing else.
pub const WORKER_STARTUP_PROMPT: &str =
    ".claude/task-brief.md を読んで、その指示に従って作業を開始してください";

/// What a resumed worker is told. The hub may have written to the outbox while nobody was
/// there to be woken, and the outbox is the one place a worker hears from it.
pub const WORKER_RESUME_PROMPT: &str = "セッションを再開しました。止まっていた間に hub から連絡が届いているかもしれないので、adjutant_outbox（使えなければ `adj outbox`）で確認してから、中断したところから作業を続けてください";

/// The placeholder a runner template puts the session id in.
pub const SESSION_PLACEHOLDER: &str = "sessionId";

/// Whether a template hands the agent a session id, which is the only way a session it starts
/// can be found again. One that does not starts fine and simply cannot be resumed.
pub fn records_session(template: Option<&str>, default: &str) -> bool {
    contains_placeholder(template.unwrap_or(default), SESSION_PLACEHOLDER)
}

/// The command line that starts a hub session called `name`.
pub fn hub_command(
    template: Option<&str>,
    env: &[(String, String)],
    name: &str,
    session: &str,
    prompt: &str,
) -> String {
    hub_line(
        template.unwrap_or(DEFAULT_HUB_RUNNER),
        env,
        name,
        session,
        prompt,
    )
}

/// The command line that reopens the hub session `session`.
pub fn hub_resume_command(
    template: Option<&str>,
    env: &[(String, String)],
    name: &str,
    session: &str,
    prompt: &str,
) -> String {
    hub_line(
        template.unwrap_or(DEFAULT_HUB_RESUME_RUNNER),
        env,
        name,
        session,
        prompt,
    )
}

fn hub_line(
    template: &str,
    env: &[(String, String)],
    name: &str,
    session: &str,
    prompt: &str,
) -> String {
    let mut command = render(
        template,
        &[
            ("name", Sub::Quoted(name)),
            (SESSION_PLACEHOLDER, Sub::Quoted(session)),
            ("prompt", Sub::Quoted(prompt)),
        ],
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
    session: &str,
    prompt: &str,
    worktree: &str,
    title: &str,
) -> String {
    worker_line(
        template.unwrap_or(DEFAULT_AGENT_RUNNER),
        env,
        session,
        prompt,
        worktree,
        title,
    )
}

/// The command line that reopens the worker session `session`.
pub fn worker_resume_command(
    template: Option<&str>,
    env: &[(String, String)],
    session: &str,
    prompt: &str,
    worktree: &str,
    title: &str,
) -> String {
    worker_line(
        template.unwrap_or(DEFAULT_AGENT_RESUME_RUNNER),
        env,
        session,
        prompt,
        worktree,
        title,
    )
}

fn worker_line(
    template: &str,
    env: &[(String, String)],
    session: &str,
    prompt: &str,
    worktree: &str,
    title: &str,
) -> String {
    let mut command = render(
        template,
        &[
            (SESSION_PLACEHOLDER, Sub::Quoted(session)),
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

    const SID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

    #[test]
    fn the_default_runner_starts_a_worker_that_can_run_its_own_build() {
        assert_eq!(
            worker_command(None, &[], SID, "go now", "/wt/wid-957", "WID-957"),
            format!("claude --session-id {SID} --permission-mode auto 'go now'")
        );
    }

    #[test]
    fn a_prompt_with_quotes_in_it_survives_the_trip_through_the_terminal() {
        let out = worker_command(
            None,
            &[],
            SID,
            "read .claude/task-brief.md, it's there",
            "/wt",
            "T",
        );
        let echoed = std::process::Command::new("sh")
            .arg("-c")
            .arg(out.replace(
                &format!("claude --session-id {SID} --permission-mode auto"),
                "printf %s",
            ))
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
            worker_command(Some("codex exec '{prompt}'"), &[], SID, "go", "/wt", "T"),
            "codex exec go"
        );
        assert_eq!(
            worker_command(
                Some("agy run {prompt} --cwd {worktree}"),
                &[],
                SID,
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
            worker_command(Some("myagent --resume"), &[], SID, "go now", "/wt", "T"),
            "myagent --resume 'go now'"
        );
    }

    #[test]
    fn the_hub_is_started_by_name_because_the_name_is_the_address() {
        let out = hub_command(None, &[], "adjutant-acme-widget", SID, HUB_STARTUP_PROMPT);
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
                SID,
                "go",
                "/wt",
                "T"
            ),
            format!(
                "env CLAUDE_CONFIG_DIR='/cfg/app one' claude --session-id {SID} --permission-mode auto go"
            )
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
            SID,
            "go",
            "/wt",
            "T",
        );
        std::process::Command::new("sh")
            .arg("-c")
            .arg(out.replace(
                &format!("claude --session-id {SID} --permission-mode auto"),
                "true",
            ))
            .output()
            .unwrap();
        // The `;` is inside the assignment `env` is handed, not a separator the shell acts
        // on — so the injected command is a variable name nobody reads, and never a
        // command. (`env` itself is lenient about odd names, so what proves it is the file
        // that was never created, not an exit status.)
        assert!(!marker.exists(), "{out}");
        assert!(out.starts_with("env ';touch "), "{out}");
    }

    #[test]
    fn both_defaults_hand_the_agent_the_id_they_are_later_resumed_by() {
        let hub = hub_command(None, &[], "adjutant-acme-widget", SID, "go");
        assert!(hub.contains(&format!("--session-id {SID}")), "{hub}");
        let worker = worker_command(None, &[], SID, "go", "/wt", "T");
        assert!(worker.contains(&format!("--session-id {SID}")), "{worker}");
        assert!(records_session(None, DEFAULT_HUB_RUNNER));
        assert!(records_session(None, DEFAULT_AGENT_RUNNER));
    }

    #[test]
    fn a_runner_without_a_session_id_is_one_nothing_can_resume() {
        assert!(!records_session(
            Some("codex exec {prompt}"),
            DEFAULT_AGENT_RUNNER
        ));
        assert!(records_session(
            Some("myagent --id {sessionId} {prompt}"),
            DEFAULT_AGENT_RUNNER
        ));
    }

    #[test]
    fn resuming_reopens_the_recorded_session_rather_than_starting_one() {
        let hub = hub_resume_command(None, &[], "adjutant-acme-widget", SID, HUB_RESUME_PROMPT);
        assert!(
            hub.starts_with(&format!("claude -n adjutant-acme-widget --resume {SID} ")),
            "{hub}"
        );
        assert!(!hub.contains("--session-id"), "{hub}");
        let worker = worker_resume_command(None, &[], SID, WORKER_RESUME_PROMPT, "/wt", "T");
        assert!(
            worker.starts_with(&format!("claude --resume {SID} --permission-mode auto ")),
            "{worker}"
        );
    }

    #[test]
    fn a_resume_template_of_ones_own_gets_the_same_placeholders() {
        assert_eq!(
            worker_resume_command(
                Some("myagent resume {sessionId} --in {worktree}"),
                &[("K".into(), "v".into())],
                SID,
                "go on",
                "/wt/x",
                "T"
            ),
            format!("env K=v myagent resume {SID} --in /wt/x 'go on'")
        );
    }
}
