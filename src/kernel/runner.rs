//! Starting a worker agent.
//!
//! The hub's job ends at "here is a worktree and here is a brief"; which program reads that
//! brief is not its business. Keeping the start command in the config is what lets a second
//! agent take the same task without a second copy of the hub's prompt.

use std::ops::Range;
use std::path::Path;

use crate::infra::template::{Sub, contains_placeholder, render, sh_quote};
use crate::kernel::config;
use crate::kernel::identity;

/// Claude Code, in the mode a worker needs: it has to run a build and a test suite without
/// a human at the tab to approve each one.
///
/// `--session-id` is what lets `adj worker --resume` find this conversation again: the id is
/// made up before the agent starts and written down next to the worker record, so the session
/// can be reopened by name rather than guessed at with "the most recent one in this directory".
pub const DEFAULT_AGENT_RUNNER: &str =
    "claude --session-id {sessionId} --permission-mode auto {settings} {prompt}";

/// The same worker, brought back. Only reached through `--resume`, with the id the start
/// command was given.
pub const DEFAULT_AGENT_RESUME_RUNNER: &str =
    "claude --resume {sessionId} --permission-mode auto {settings} {prompt}";

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
    "claude -n {name} --session-id {sessionId} --permission-mode auto {settings} {prompt}";

/// The hub brought back. The name stays on the line so the session reads the same in every
/// listing after the restart as it did before it.
pub const DEFAULT_HUB_RESUME_RUNNER: &str =
    "claude -n {name} --resume {sessionId} --permission-mode auto {settings} {prompt}";

/// What the hub is told on startup.
///
/// What the hub is told on startup — naming two ways to fetch the procedure, and not a
/// slash command.
///
/// Prompt support differs between agents and between versions, so a hub that starts with an
/// unrecognised `/…` sits there doing nothing, which looks exactly like a hub that is up and
/// idle. Both a tool and a command are named because neither is universal: an MCP tool may
/// be deferred or absent, and a command needs the binary on PATH.
pub const HUB_STARTUP_PROMPT: &str = "Fetch the adj-hub procedure with adjutant_skill name=adj-hub (or `adj skill adj-hub` if that is not available), and start the resident hub as it says.";

/// What a resumed hub is told. The conversation is all there, procedure included, so this
/// only has to cover what happened while it was gone: reports that landed with nobody to be
/// woken by them.
pub const HUB_RESUME_PROMPT: &str = "The session has been resumed. Something may have arrived while it was down, so check the inbox with adjutant_pending, sort out anything waiting as on startup, and then go back to waiting.";

/// What a worker is told on startup, when the hub says nothing else.
pub const WORKER_STARTUP_PROMPT: &str = "Read .claude/task-brief.md and start working as it says.";

/// What a resumed worker is told. The hub may have written to the outbox while nobody was
/// there to be woken, and the outbox is the one place a worker hears from it.
pub const WORKER_RESUME_PROMPT: &str = "The session has been resumed. The hub may have sent something while it was down, so check with adjutant_outbox (or `adj outbox` if that is not available), then carry on from where you left off.";

/// The placeholder a runner template puts the session id in.
pub const SESSION_PLACEHOLDER: &str = "sessionId";

/// The placeholder a runner template puts the hook settings in: `--settings <file>`, two words
/// or nothing, so it is written unquoted like the others.
pub const SETTINGS_PLACEHOLDER: &str = "settings";

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
    settings: Option<&str>,
) -> String {
    hub_line(
        template.unwrap_or(DEFAULT_HUB_RUNNER),
        env,
        name,
        session,
        prompt,
        settings,
    )
}

/// The command line that reopens the hub session `session`.
pub fn hub_resume_command(
    template: Option<&str>,
    env: &[(String, String)],
    name: &str,
    session: &str,
    prompt: &str,
    settings: Option<&str>,
) -> String {
    hub_line(
        template.unwrap_or(DEFAULT_HUB_RESUME_RUNNER),
        env,
        name,
        session,
        prompt,
        settings,
    )
}

fn hub_line(
    template: &str,
    env: &[(String, String)],
    name: &str,
    session: &str,
    prompt: &str,
    settings: Option<&str>,
) -> String {
    let flag = settings_flag(settings);
    let template = settings_template(template, settings);
    let mut command = render(
        &template,
        &[
            ("name", Sub::Quoted(name)),
            (SESSION_PLACEHOLDER, Sub::Quoted(session)),
            ("prompt", Sub::Quoted(prompt)),
            (SETTINGS_PLACEHOLDER, Sub::Raw(&flag)),
        ],
    );
    if !contains_placeholder(&template, "prompt") {
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
    settings: Option<&str>,
) -> String {
    worker_line(
        template.unwrap_or(DEFAULT_AGENT_RUNNER),
        env,
        session,
        prompt,
        worktree,
        title,
        settings,
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
    settings: Option<&str>,
) -> String {
    worker_line(
        template.unwrap_or(DEFAULT_AGENT_RESUME_RUNNER),
        env,
        session,
        prompt,
        worktree,
        title,
        settings,
    )
}

fn worker_line(
    template: &str,
    env: &[(String, String)],
    session: &str,
    prompt: &str,
    worktree: &str,
    title: &str,
    settings: Option<&str>,
) -> String {
    let flag = settings_flag(settings);
    let template = settings_template(template, settings);
    let mut command = render(
        &template,
        &[
            (SESSION_PLACEHOLDER, Sub::Quoted(session)),
            ("prompt", Sub::Quoted(prompt)),
            ("worktree", Sub::Quoted(worktree)),
            ("title", Sub::Quoted(title)),
            (SETTINGS_PLACEHOLDER, Sub::Raw(&flag)),
        ],
    );
    // A template with nowhere to put the prompt would start an agent with no instructions,
    // which looks like a hung worker rather than a config mistake. Appending is the reading
    // that matches every CLI agent's own argument order.
    if !contains_placeholder(&template, "prompt") {
        command = format!("{command} {}", sh_quote(prompt));
    }
    with_env(env, command)
}

/// What `{settings}` renders to: `--settings <file>`, or nothing when there is no file. Raw
/// because it is two words (or none), which a quoted value would make one.
fn settings_flag(settings: Option<&str>) -> String {
    settings
        .map(|path| format!("--settings {}", sh_quote(path)))
        .unwrap_or_default()
}

/// Whether a launch with this template wants the hook settings file written: the template
/// names `{settings}`, or runs `claude` without passing a `--settings` of its own.
pub fn wants_settings(template: &str) -> bool {
    if contains_placeholder(template, SETTINGS_PLACEHOLDER) {
        return true;
    }
    agent_token(template).is_some_and(|span| agent_name(&template[span]) == "claude")
        && !passes_own_settings(template)
}

/// Whether the template already passes a `--settings` flag. Claude Code takes the last
/// `--settings` and drops the earlier ones without an error, so adding one beside the user's
/// would silently replace theirs: theirs is left alone, and that session has no injected hooks.
fn passes_own_settings(template: &str) -> bool {
    template
        .split_whitespace()
        .any(|word| word == "--settings" || word.starts_with("--settings="))
}

/// The template `{settings}` is rendered into: the placeholder added right after the agent's
/// name when the launch wants hooks and the template does not name it, or taken out (with one
/// of the spaces beside it) when there is no file to pass.
///
/// No attempt is made to read a template the shell would not take as one command (a pipe,
/// `;`): the placeholder goes after the agent's name, and a template that needs it elsewhere
/// names it there.
fn settings_template(template: &str, settings: Option<&str>) -> String {
    let named = contains_placeholder(template, SETTINGS_PLACEHOLDER);
    match (settings, named) {
        (Some(_), false) if wants_settings(template) => match agent_token(template) {
            Some(span) => format!(
                "{} {{{SETTINGS_PLACEHOLDER}}}{}",
                &template[..span.end],
                &template[span.end..]
            ),
            None => template.to_string(),
        },
        (Some(_), _) => template.to_string(),
        (None, _) => {
            let placeholder = format!("{{{SETTINGS_PLACEHOLDER}}}");
            template
                .replace(&format!(" {placeholder}"), "")
                .replace(&format!("{placeholder} "), "")
                .replace(&placeholder, "")
        }
    }
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

/// Extract the agent harness / program name from a runner command template.
///
/// Skips any `env` wrappers or variable assignments (`KEY=VAL`), returning the base name
/// of the first executable token (e.g. "claude", "agy", "codex"). Defaults to "claude"
/// when no executable token can be identified.
pub fn agent_from_runner(runner: &str) -> String {
    match agent_token(runner) {
        Some(span) => agent_name(&runner[span]),
        None => "claude".to_string(),
    }
}

fn agent_name(token: &str) -> String {
    std::path::Path::new(token)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| token.to_string())
}

/// Where the agent's word is in the template: the first one that is not `env` or a variable
/// assignment (whose quoted value may hold spaces).
fn agent_token(runner: &str) -> Option<Range<usize>> {
    let mut in_quote: Option<char> = None;
    for part in runner.split_whitespace() {
        if let Some(q) = in_quote {
            if part.contains(q) {
                in_quote = None;
            }
            continue;
        }
        if part == "env" || part.contains('=') {
            for q in ['\'', '"'] {
                if part.matches(q).count() % 2 == 1 {
                    in_quote = Some(q);
                }
            }
            continue;
        }
        if !agent_name(part).is_empty() {
            // `part` is a slice of `runner`, so its offset is the difference of the pointers.
            let start = part.as_ptr() as usize - runner.as_ptr() as usize;
            return Some(start..start + part.len());
        }
    }
    None
}

pub fn runner_for_procedure(settings: &config::Settings, procedure: &str) -> Option<String> {
    match procedure {
        "adj-hub" => settings.hub_runner.clone(),
        _ => settings.agent_runner.clone(),
    }
}

pub fn resolve_runner_for(cwd: Option<&Path>, procedure: &str) -> Option<String> {
    let info = identity::resolve_in(cwd, None, None).ok()?;
    let settings = config::resolve_config(&info.nwo).ok()?.settings;
    runner_for_procedure(&settings, procedure)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

    #[test]
    fn the_default_runner_starts_a_worker_that_can_run_its_own_build() {
        assert_eq!(
            worker_command(None, &[], SID, "go now", "/wt/wid-957", "WID-957", None),
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
            None,
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
            worker_command(
                Some("codex exec '{prompt}'"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None
            ),
            "codex exec go"
        );
        assert_eq!(
            worker_command(
                Some("agy run {prompt} --cwd {worktree}"),
                &[],
                SID,
                "go",
                "/wt/x",
                "T",
                None
            ),
            "agy run go --cwd /wt/x"
        );
    }

    #[test]
    fn a_template_that_forgot_the_prompt_still_gets_one() {
        assert_eq!(
            worker_command(
                Some("myagent --resume"),
                &[],
                SID,
                "go now",
                "/wt",
                "T",
                None
            ),
            "myagent --resume 'go now'"
        );
    }

    #[test]
    fn the_hub_is_started_by_name_because_the_name_is_the_address() {
        let out = hub_command(
            None,
            &[],
            "adjutant-acme-widget",
            SID,
            HUB_STARTUP_PROMPT,
            None,
        );
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
                "T",
                None
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
            None,
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
        let hub = hub_command(None, &[], "adjutant-acme-widget", SID, "go", None);
        assert!(hub.contains(&format!("--session-id {SID}")), "{hub}");
        let worker = worker_command(None, &[], SID, "go", "/wt", "T", None);
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
        let hub = hub_resume_command(
            None,
            &[],
            "adjutant-acme-widget",
            SID,
            HUB_RESUME_PROMPT,
            None,
        );
        assert!(
            hub.starts_with(&format!("claude -n adjutant-acme-widget --resume {SID} ")),
            "{hub}"
        );
        assert!(!hub.contains("--session-id"), "{hub}");
        let worker = worker_resume_command(None, &[], SID, WORKER_RESUME_PROMPT, "/wt", "T", None);
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
                "T",
                None
            ),
            format!("env K=v myagent resume {SID} --in /wt/x 'go on'")
        );
    }

    #[test]
    fn agent_from_runner_extracts_executable_name() {
        assert_eq!(
            agent_from_runner("claude --session-id {sessionId} --permission-mode auto"),
            "claude"
        );
        assert_eq!(
            agent_from_runner("agy --dangerously-skip-permissions -i {prompt}"),
            "agy"
        );
        assert_eq!(
            agent_from_runner("env FOO=bar BAZ=1 /opt/bin/codex exec {prompt}"),
            "codex"
        );
        assert_eq!(
            agent_from_runner("env CLAUDE_CONFIG_DIR='/cfg/app one' claude --resume {sessionId}"),
            "claude"
        );
        assert_eq!(
            agent_from_runner("env A=\"val with spaces\" B='another space' agy -i {prompt}"),
            "agy"
        );
        assert_eq!(agent_from_runner(""), "claude");
    }

    const SETTINGS: &str = "/state/agent-hooks/claude-0123abcd.json";

    fn worker_with(template: &str, settings: Option<&str>) -> String {
        worker_command(Some(template), &[], SID, "go", "/wt", "T", settings)
    }

    #[test]
    fn the_defaults_pass_the_hook_settings_before_the_prompt() {
        let worker = worker_command(None, &[], SID, "go", "/wt", "T", Some(SETTINGS));
        assert_eq!(
            worker,
            format!("claude --session-id {SID} --permission-mode auto --settings {SETTINGS} go")
        );
        let spaced = worker_command(
            None,
            &[],
            SID,
            "go",
            "/a b/state.json",
            "T",
            Some("/a b/s.json"),
        );
        assert!(spaced.contains("--settings '/a b/s.json' go"), "{spaced}");
        let hub = hub_command(None, &[], "h", SID, "go", Some(SETTINGS));
        assert!(hub.contains(&format!("--settings {SETTINGS} go")), "{hub}");
        let hub = hub_resume_command(None, &[], "h", SID, "go", Some(SETTINGS));
        assert!(hub.contains(&format!("--settings {SETTINGS} go")), "{hub}");
        let worker = worker_resume_command(None, &[], SID, "go", "/wt", "T", Some(SETTINGS));
        assert!(
            worker.contains(&format!("--settings {SETTINGS} go")),
            "{worker}"
        );
    }

    #[test]
    fn a_claude_template_that_does_not_name_it_gets_it_right_after_claude() {
        assert_eq!(
            worker_with("claude --foo {prompt}", Some(SETTINGS)),
            format!("claude --settings {SETTINGS} --foo go")
        );
        assert_eq!(
            worker_with("env K='a b' claude --foo {prompt}", Some(SETTINGS)),
            format!("env K='a b' claude --settings {SETTINGS} --foo go")
        );
        assert_eq!(
            worker_with("/opt/bin/claude {prompt}", Some(SETTINGS)),
            format!("/opt/bin/claude --settings {SETTINGS} go")
        );
        // No prompt placeholder: the prompt is appended after it.
        assert_eq!(
            worker_with("claude", Some(SETTINGS)),
            format!("claude --settings {SETTINGS} go")
        );
    }

    #[test]
    fn a_template_with_its_own_settings_is_left_alone() {
        for template in [
            "claude --settings x {prompt}",
            "claude --settings=x {prompt}",
        ] {
            assert!(!wants_settings(template));
            assert_eq!(
                worker_with(template, Some(SETTINGS)),
                template.replace("{prompt}", "go")
            );
        }
    }

    #[test]
    fn another_agent_is_not_given_claudes_flag() {
        assert!(!wants_settings("codex exec {prompt}"));
        assert_eq!(
            worker_with("codex exec {prompt}", Some(SETTINGS)),
            "codex exec go"
        );
        assert!(!wants_settings(""));
    }

    #[test]
    fn a_template_that_names_it_elsewhere_gets_it_there_only() {
        let out = worker_with("myagent -p {prompt} {settings} --tail", Some(SETTINGS));
        assert_eq!(out, format!("myagent -p go --settings {SETTINGS} --tail"));
        assert!(wants_settings("myagent {settings}"));
        // Named, so not added a second time after the agent's name.
        let out = worker_with("claude {prompt} {settings}", Some(SETTINGS));
        assert_eq!(out, format!("claude go --settings {SETTINGS}"));
    }

    #[test]
    fn with_no_file_the_placeholder_leaves_neither_text_nor_a_double_space() {
        assert_eq!(
            worker_with("claude --a {settings} {prompt}", None),
            "claude --a go"
        );
        assert_eq!(worker_with("claude {prompt} {settings}", None), "claude go");
        assert_eq!(worker_with("claude {settings}", None), "claude go");
        assert_eq!(worker_with("{settings} claude", None), "claude go");
        // And a template that does not name it is not given one.
        assert_eq!(worker_with("claude --a {prompt}", None), "claude --a go");
    }

    #[test]
    fn agent_token_is_the_span_of_the_agent_word() {
        let runner = "env K='a b' /opt/claude --x";
        let span = agent_token(runner).unwrap();
        assert_eq!(&runner[span], "/opt/claude");
        assert!(agent_token("").is_none());
        assert!(agent_token("env A=1").is_none());
    }
}
