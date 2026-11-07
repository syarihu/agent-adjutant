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
///
/// A chosen model (`--model`) is added as `--model <model>` right after `claude`. A flag is
/// passed rather than `ANTHROPIC_MODEL` because `--model` is Claude Code's highest-precedence
/// model setting (an `env` or `settings.json` model cannot override it), it shows in the
/// command line (`adj worker --dry-run`), and it does not leak into every child process the agent
/// runs (an exported `ANTHROPIC_MODEL` would reach a nested `claude -p`).
pub const DEFAULT_AGENT_RUNNER: &str =
    "claude --session-id {sessionId} --permission-mode auto {settings} {prompt}";

/// The same worker, brought back. Only reached through `--resume`, with the id the start
/// command was given.
///
/// A chosen model is added as `--model <model>` after `claude`, the same way as for
/// [`DEFAULT_AGENT_RUNNER`].
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

/// The reader of a PR, the read-only worker of the code-review hub. It has no checkout and no
/// `gh`: it reads what `adj review context` writes next to it, and hands in one JSON file.
///
/// Read-only is enforced here, not by asking. `--allowedTools` only *adds* rules, so on its own it
/// would sit on top of whatever the person's own Claude settings allow (a `gh` rule, a wildcard
/// `Bash`) and on every MCP server they have (some of which write to GitHub). Two flags take
/// those away: `--setting-sources ''` loads no user, project or local settings file, and
/// `--strict-mcp-config` loads no MCP server but those named with `--mcp-config` (none). The
/// `--settings` file this adds (`{settings}`: adjutant's own hooks) is not a setting source in
/// that sense and still applies. What is left is the list: under `--permission-mode dontAsk`
/// anything off it is denied, not asked, so the reader cannot push, comment or review, and nothing
/// waits on a prompt in a window nobody watches. With no MCP server the reader uses the `adj`
/// command for everything (`adj skill`, `adj review say`), so no `mcp__` tool is listed. A deny list
/// over `gh` would leak (`gh api -f query=mutation`, `--input`, `graphql`); having no `gh` does not.
///
/// The one file it may write is `./read-through.json`, named as an `Edit(...)` rule: Claude's file
/// checks match `Edit(path)` rules only, and an `Edit` rule covers every tool that edits a file,
/// Write included. A `Write(path)` rule would match nothing and deny the file.
///
/// `--permission-mode` comes after `--allowedTools` on purpose. The tools option takes any number
/// of values, and `{settings}` renders to nothing when there is no hooks file, so a flag has to
/// follow the list or the prompt would be read as one more tool pattern.
///
/// Reading is scoped to the reader's own directory: `Read(./**)`, `Grep(./**)` and `Glob(./**)` allow
/// what `adj review context` wrote there and deny any path outside it, so the reader cannot read
/// the person's other files (keys, tokens, other readers' material) however a pull request asks.
///
/// There is no `adj send` on the list: it can address any hub and wake it, and a reader that read
/// a hostile pull request must not be able to. `adj review say` reaches the code-review hub and
/// nobody else.
///
/// The patterns match `adj ...` as typed, so `adj` has to be on PATH. A `reviewerRunner` of your
/// own has to keep both isolation flags, or the reader gets whatever the person's settings give
/// it. Another agent is held to reading by the procedure alone.
///
/// Claude Code and agy ask whether to trust the directory they start in, and nobody answers in a
/// reader's window: `run_reader` starts a watcher that answers it for the reader's own directory
/// and fails the read at any other prompt. A runner of any other agent has no watcher.
///
/// `--setting-sources ''` also drops the person's `"tui": "fullscreen"`, which would leave Claude
/// Code's inline UI, where the board's terminal sends the mouse wheel as arrow keys that scroll
/// the input history. So `run_reader` sets `CLAUDE_CODE_NO_FLICKER=1` for a Claude reader unless
/// `agentEnv` already sets it.
pub const DEFAULT_REVIEWER_RUNNER: &str = "claude -n {name} --session-id {sessionId} --setting-sources '' --strict-mcp-config --allowedTools 'Read(./**),Grep(./**),Glob(./**),Edit(./read-through.json),Bash(adj review context:*),Bash(adj review submit:*),Bash(adj review fail:*),Bash(adj review say:*),Bash(adj skill:*)' --permission-mode dontAsk {settings} {prompt}";

/// The name the MCP server of `adj mcp` goes by in an `--mcp-config` and in the tool names Claude
/// Code makes of it (`mcp__adjutant__<tool>`).
pub const MCP_SERVER_NAME: &str = "adjutant";

/// What the code-review hub is told on startup, in the shape of `HUB_STARTUP_PROMPT`.
pub const REVIEW_HUB_STARTUP_PROMPT: &str = "Fetch the adj-review-hub procedure with adjutant_skill name=adj-review-hub (or `adj skill adj-review-hub` if that is not available), and start the code-review hub as it says.";

/// What the hub is told on startup.
///
/// What the hub is told on startup — naming two ways to fetch the procedure, and not a
/// slash command.
///
/// Prompt support differs between agents and between versions, so a hub that starts with an
/// unrecognised `/…` sits there doing nothing, which looks exactly like a hub that is up and
/// idle. Both a tool and a command are named because neither is universal: an MCP tool may
/// be deferred or absent, and a command needs the binary on PATH.
pub const HUB_STARTUP_PROMPT: &str = "Fetch the core of the adj-hub procedure with adjutant_skill name=adj-hub (or `adj skill adj-hub` if that is not available), start the resident hub as it says, and fetch each section its index names when its trigger comes.";

/// What a resumed hub is told. The conversation may have been compacted while it was gone,
/// so it reads the core again before working its inbox.
pub const HUB_RESUME_PROMPT: &str = "The session has been resumed. First read the core again (adjutant_skill name=adj-hub, or `adj skill adj-hub`) since the conversation may have been compacted, then check the inbox with adjutant_pending, sort out anything waiting as on startup, and then go back to waiting.";

/// What a resumed code-review hub is told, in the shape of `HUB_RESUME_PROMPT`: it reads its
/// own procedure again, not the repository hub's.
pub const REVIEW_HUB_RESUME_PROMPT: &str = "The session has been resumed. First read the adj-review-hub procedure again (adjutant_skill name=adj-review-hub, or `adj skill adj-review-hub`) since the conversation may have been compacted, then check the inbox with adjutant_pending, sort out anything waiting as on startup, and then go back to waiting.";

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

/// The placeholder a runner template puts the chosen agent model in.
pub const MODEL_PLACEHOLDER: &str = "model";

/// Validate a model name.
///
/// A model name is 1 to 128 ASCII characters from `A-Z a-z 0-9 . _ - : / @ [ ]`,
/// and its first character must be a letter or a digit so it can never be read as an option.
pub fn check_model(model: &str) -> Result<(), String> {
    let len = model.len();
    if (1..=128).contains(&len)
        && model.is_ascii()
        && model
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && model.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/' | '@' | '[' | ']')
        })
    {
        Ok(())
    } else {
        Err(format!(
            "refused model name {model:?}: must be 1 to 128 ASCII characters (letters, digits, or . _ - : / @ [ ]), starting with a letter or digit"
        ))
    }
}

/// How a worker template takes a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSlot {
    Placeholder,
    Appended,
    Refused(String),
}

/// Determine how a worker template takes a model.
///
/// - `Placeholder`: the template names `{model}`.
/// - `Appended`: no `{model}`, the agent is `claude`, and the template does not pass `--model` of its own.
/// - `Refused`: anything else: a non-claude agent without `{model}`, or a claude template that already passes `--model`.
pub fn model_slot(template: &str) -> ModelSlot {
    if contains_placeholder(template, MODEL_PLACEHOLDER) {
        return ModelSlot::Placeholder;
    }
    // The executable may be quoted (`"/usr/local/bin/claude"`); unquote it for this decision only.
    let agent = agent_token(template).map(|span| unquoted_word(&template[span]));
    let is_claude = agent
        .as_deref()
        .is_some_and(|word| agent_name(word) == "claude");
    // Match on the unquoted words, so `--model` inside another option's quoted value is not
    // mistaken for the option itself while `"--model"` is.
    let passes_own = shell_words(template)
        .iter()
        .any(|(_, word)| word == "--model" || word.starts_with("--model="));

    if is_claude {
        if passes_own {
            ModelSlot::Refused("already passes --model; write {model} there instead".to_string())
        } else {
            ModelSlot::Appended
        }
    } else {
        let agent = agent
            .map(|word| agent_name(&word))
            .unwrap_or_else(|| agent_from_runner(template));
        ModelSlot::Refused(format!(
            "has no {{{MODEL_PLACEHOLDER}}}, so a model cannot be passed to {agent}"
        ))
    }
}

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

/// The command line that starts the reader session `name` of a PR.
pub fn reviewer_command(
    template: Option<&str>,
    env: &[(String, String)],
    name: &str,
    session: &str,
    prompt: &str,
    settings: Option<&str>,
) -> String {
    hub_line(
        template.unwrap_or(DEFAULT_REVIEWER_RUNNER),
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
#[allow(clippy::too_many_arguments)]
pub fn worker_command(
    template: Option<&str>,
    env: &[(String, String)],
    session: &str,
    prompt: &str,
    worktree: &str,
    title: &str,
    settings: Option<&str>,
    model: Option<&str>,
) -> String {
    worker_line(
        template.unwrap_or(DEFAULT_AGENT_RUNNER),
        env,
        session,
        prompt,
        worktree,
        title,
        settings,
        model,
    )
}

/// The command line that reopens the worker session `session`.
#[allow(clippy::too_many_arguments)]
pub fn worker_resume_command(
    template: Option<&str>,
    env: &[(String, String)],
    session: &str,
    prompt: &str,
    worktree: &str,
    title: &str,
    settings: Option<&str>,
    model: Option<&str>,
) -> String {
    worker_line(
        template.unwrap_or(DEFAULT_AGENT_RESUME_RUNNER),
        env,
        session,
        prompt,
        worktree,
        title,
        settings,
        model,
    )
}

#[allow(clippy::too_many_arguments)]
fn worker_line(
    template: &str,
    env: &[(String, String)],
    session: &str,
    prompt: &str,
    worktree: &str,
    title: &str,
    settings: Option<&str>,
    model: Option<&str>,
) -> String {
    let flag = settings_flag(settings);
    let template = settings_template(template, settings);
    let template = model_template(&template, model);
    let mut subs = vec![
        (SESSION_PLACEHOLDER, Sub::Quoted(session)),
        ("prompt", Sub::Quoted(prompt)),
        ("worktree", Sub::Quoted(worktree)),
        ("title", Sub::Quoted(title)),
        (SETTINGS_PLACEHOLDER, Sub::Raw(&flag)),
    ];
    if let Some(m) = model {
        subs.push((MODEL_PLACEHOLDER, Sub::Quoted(m)));
    }
    let mut command = render(&template, &subs);
    // A template with nowhere to put the prompt would start an agent with no instructions,
    // which looks like a hung worker rather than a config mistake. Appending is the reading
    // that matches every CLI agent's own argument order.
    if !contains_placeholder(&template, "prompt") {
        command = format!("{command} {}", sh_quote(prompt));
    }
    with_env(env, command)
}

fn model_template(template: &str, model: Option<&str>) -> String {
    match model {
        Some(_) => match model_slot(template) {
            ModelSlot::Placeholder => template.to_string(),
            ModelSlot::Appended => match agent_token(template) {
                Some(span) => format!(
                    "{} --model {{{MODEL_PLACEHOLDER}}}{}",
                    &template[..span.end],
                    &template[span.end..]
                ),
                None => template.to_string(),
            },
            ModelSlot::Refused(_) => template.to_string(),
        },
        None => drop_model_placeholder(template),
    }
}

fn is_model_option(word: &str) -> bool {
    word == "-m" || (word.starts_with("--") && word.contains("model"))
}

/// Remove every `{model}` from a template that is run without a model, together with the
/// option that introduces it (`--model {model}`, `-m {model}`, `--model={model}`) so no
/// dangling flag is left behind. Words are compared unquoted, so `"--model" {model}` and
/// `'--model={model}'` are recognised like their unquoted forms.
fn drop_model_placeholder(template: &str) -> String {
    let placeholder = format!("{{{MODEL_PLACEHOLDER}}}");
    let mut template = template.to_string();
    while template.contains(&placeholder) {
        let words = shell_words(&template);
        let Some(idx) = words
            .iter()
            .position(|(span, _)| template[span.clone()].contains(&placeholder))
        else {
            break;
        };
        let (span, text) = &words[idx];
        let next = if *text == placeholder {
            match idx.checked_sub(1).map(|i| &words[i]) {
                Some((prev, prev_text)) if is_model_option(prev_text) => {
                    remove_span_and_space(&template, prev.start..span.end)
                }
                _ => remove_span_and_space(&template, span.clone()),
            }
        } else if text.starts_with("--")
            && let Some((opt, val)) = text.split_once('=')
            && opt.contains("model")
            && val == placeholder
        {
            remove_span_and_space(&template, span.clone())
        } else {
            // A placeholder embedded in some other word: drop it plainly, all occurrences.
            return drop_placeholder(&template, MODEL_PLACEHOLDER);
        };
        template = next;
    }
    template
}

/// The first shell word of `s`, unquoted (the word itself when it holds no quotes).
fn unquoted_word(s: &str) -> String {
    shell_words(s)
        .into_iter()
        .next()
        .map(|(_, word)| word)
        .unwrap_or_else(|| s.to_string())
}

/// Split a template into shell words: each word's byte span in `s` and its unquoted text.
///
/// Single quotes are literal. Inside double quotes a backslash escapes only `"`, `\`, `$`
/// and a backtick. Outside quotes a backslash escapes the next character. An unterminated
/// quote runs to the end of the string.
fn shell_words(s: &str) -> Vec<(Range<usize>, String)> {
    let mut words = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut text = String::new();
        let mut end = start;
        while let Some(&(i, c)) = chars.peek() {
            if c.is_whitespace() {
                break;
            }
            chars.next();
            end = i + c.len_utf8();
            match c {
                '\'' => {
                    for (j, q) in chars.by_ref() {
                        end = j + q.len_utf8();
                        if q == '\'' {
                            break;
                        }
                        text.push(q);
                    }
                }
                '"' => {
                    while let Some((j, q)) = chars.next() {
                        end = j + q.len_utf8();
                        match q {
                            '"' => break,
                            '\\' => match chars.peek() {
                                Some(&(k, n)) if matches!(n, '"' | '\\' | '$' | '`') => {
                                    chars.next();
                                    end = k + n.len_utf8();
                                    text.push(n);
                                }
                                _ => text.push(q),
                            },
                            _ => text.push(q),
                        }
                    }
                }
                '\\' => match chars.next() {
                    Some((k, n)) => {
                        end = k + n.len_utf8();
                        text.push(n);
                    }
                    None => text.push(c),
                },
                _ => text.push(c),
            }
        }
        words.push((start..end, text));
    }
    words
}

fn remove_span_and_space(template: &str, span: Range<usize>) -> String {
    if span.start > 0 && template.as_bytes()[span.start - 1] == b' ' {
        format!("{}{}", &template[..span.start - 1], &template[span.end..])
    } else if span.end < template.len() && template.as_bytes()[span.end] == b' ' {
        format!("{}{}", &template[..span.start], &template[span.end + 1..])
    } else {
        format!("{}{}", &template[..span.start], &template[span.end..])
    }
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
        (None, _) => drop_placeholder(template, SETTINGS_PLACEHOLDER),
    }
}

/// `template` without `{name}` and one of the spaces beside it, so no double space or empty
/// argument is left behind.
fn drop_placeholder(template: &str, name: &str) -> String {
    let placeholder = format!("{{{name}}}");
    template
        .replace(&format!(" {placeholder}"), "")
        .replace(&format!("{placeholder} "), "")
        .replace(&placeholder, "")
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
        "adj-hub" | "adj-review-hub" => settings.hub_runner.clone(),
        "adj-reviewer" => settings.reviewer_runner.clone(),
        _ => settings.agent_runner.clone(),
    }
}

pub fn resolve_runner_for(cwd: Option<&Path>, procedure: &str) -> Option<String> {
    // The review procedures run outside any repository.
    let hub =
        matches!(procedure, "adj-review-hub" | "adj-reviewer").then_some(identity::REVIEW_HUB_ID);
    let info = identity::resolve_in(cwd, None, hub).ok()?;
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
            worker_command(
                None,
                &[],
                SID,
                "go now",
                "/wt/wid-957",
                "WID-957",
                None,
                None
            ),
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
                None,
                None,
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
                None,
                None,
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
                None,
                None,
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
    fn the_reader_line_lists_its_tools_and_ends_with_the_prompt_with_or_without_hooks() {
        for settings in [None, Some("/state/hooks.json")] {
            let out = reviewer_command(
                None,
                &[],
                "adjutant-read-acme~widget~7",
                SID,
                "read the pull request acme/widget#7",
                settings,
            );
            // Reading is the reader's own directory, never a bare `Read` that takes any path.
            assert!(out.contains(
                "--allowedTools 'Read(./**),Grep(./**),Glob(./**),Edit(./read-through.json),"
            ));
            assert!(!out.contains("'Read,") && !out.contains(",Read,"), "{out}");
            assert!(out.contains("Bash(adj review submit:*)"), "{out}");
            assert!(out.contains("Bash(adj review fail:*)"), "{out}");
            assert!(out.contains("Bash(adj review say:*)"), "{out}");
            // `adj send` can address any hub, which a reader must not be able to.
            assert!(!out.contains("adj send"), "{out}");
            assert!(out.contains("--permission-mode dontAsk"), "{out}");
            // The person's own settings and MCP servers are not the reader's; `''` stays an empty word.
            assert!(out.contains("--setting-sources '' "), "{out}");
            assert!(out.contains("--strict-mcp-config"), "{out}");
            assert!(!out.contains("mcp__"), "{out}");
            // No `gh` and no `git` anywhere on the list.
            assert!(
                !out.contains("Bash(gh") && !out.contains("Bash(git"),
                "{out}"
            );
            // The tools list is ended by a flag, so the prompt cannot be taken for a tool.
            assert!(out.contains("' --permission-mode dontAsk"), "{out}");
            assert!(
                out.ends_with("'read the pull request acme/widget#7'"),
                "{out}"
            );
            assert_eq!(out.contains("--settings"), settings.is_some(), "{out}");
        }
    }

    #[test]
    fn the_empty_setting_sources_reaches_the_agent_as_an_empty_argument() {
        let line = reviewer_command(None, &[], "n", SID, "go", Some("/h.json"));
        // `claude` is a function that prints its arguments one by one.
        let script =
            format!("claude() {{ for a in \"$@\"; do printf '<%s>' \"$a\"; done; }}; {line}");
        let out = std::process::Command::new("sh")
            .args(["-c", &script])
            .output()
            .unwrap();
        let args = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            args.contains("<--setting-sources><><--strict-mcp-config>"),
            "{args}"
        );
        assert!(args.ends_with("<--settings></h.json><go>"), "{args}");
    }

    #[test]
    fn the_review_hub_startup_prompt_is_not_a_slash_command() {
        assert!(!REVIEW_HUB_STARTUP_PROMPT.trim_start().starts_with('/'));
        assert!(REVIEW_HUB_STARTUP_PROMPT.contains("adjutant_skill name=adj-review-hub"));
        assert!(REVIEW_HUB_STARTUP_PROMPT.contains("`adj skill adj-review-hub`"));
    }

    #[test]
    fn the_hub_startup_prompt_is_not_a_slash_command() {
        // An agent that does not recognise a slash command starts a session that sits there
        // doing nothing, which looks exactly like a hub that is up and idle.
        assert!(!HUB_STARTUP_PROMPT.trim_start().starts_with('/'));
        // Two ways in, because neither is universal.
        assert!(HUB_STARTUP_PROMPT.contains("adjutant_skill name=adj-hub"));
        assert!(HUB_STARTUP_PROMPT.contains("`adj skill adj-hub`"));
        assert!(HUB_STARTUP_PROMPT.contains("core"));
    }

    #[test]
    fn the_hub_resume_prompt_reads_the_core_again_first() {
        assert!(!HUB_RESUME_PROMPT.trim_start().starts_with('/'));
        assert!(HUB_RESUME_PROMPT.contains("adjutant_skill name=adj-hub"));
        assert!(HUB_RESUME_PROMPT.contains("`adj skill adj-hub`"));
        assert!(HUB_RESUME_PROMPT.contains("compacted"));
        assert!(HUB_RESUME_PROMPT.contains("adjutant_pending"));
        // The core comes first: a prompt that sends the hub to its inbox before it has read
        // how to deal with what is there would pass the checks above.
        let inbox = HUB_RESUME_PROMPT.find("adjutant_pending").unwrap();
        assert!(
            HUB_RESUME_PROMPT
                .find("adjutant_skill name=adj-hub")
                .unwrap()
                < inbox
        );
        assert!(HUB_RESUME_PROMPT.find("`adj skill adj-hub`").unwrap() < inbox);
    }

    #[test]
    fn the_review_hub_resumes_by_reading_its_own_procedure() {
        assert!(REVIEW_HUB_RESUME_PROMPT.contains("adjutant_skill name=adj-review-hub"));
        assert!(REVIEW_HUB_RESUME_PROMPT.contains("`adj skill adj-review-hub`"));
        assert!(!REVIEW_HUB_RESUME_PROMPT.contains("name=adj-hub"));
        let inbox = REVIEW_HUB_RESUME_PROMPT.find("adjutant_pending").unwrap();
        assert!(REVIEW_HUB_RESUME_PROMPT.find("adj-review-hub").unwrap() < inbox);
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
                None,
                None,
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
        let worker = worker_command(None, &[], SID, "go", "/wt", "T", None, None);
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
        let worker =
            worker_resume_command(None, &[], SID, WORKER_RESUME_PROMPT, "/wt", "T", None, None);
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
                None,
                None,
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
        worker_command(Some(template), &[], SID, "go", "/wt", "T", settings, None)
    }

    #[test]
    fn the_defaults_pass_the_hook_settings_before_the_prompt() {
        let worker = worker_command(None, &[], SID, "go", "/wt", "T", Some(SETTINGS), None);
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
            None,
        );
        assert!(spaced.contains("--settings '/a b/s.json' go"), "{spaced}");
        let hub = hub_command(None, &[], "h", SID, "go", Some(SETTINGS));
        assert!(hub.contains(&format!("--settings {SETTINGS} go")), "{hub}");
        let hub = hub_resume_command(None, &[], "h", SID, "go", Some(SETTINGS));
        assert!(hub.contains(&format!("--settings {SETTINGS} go")), "{hub}");
        let worker = worker_resume_command(None, &[], SID, "go", "/wt", "T", Some(SETTINGS), None);
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

    #[test]
    fn check_model_accepts_valid_names_and_refuses_invalid_names() {
        for valid in [
            "opus",
            "sonnet",
            "haiku",
            "claude-opus-5-5",
            "sonnet[1m]",
            "us.anthropic.claude-sonnet-5-5:0",
            "openai/gpt-5",
            "gemini-3.8-flash-high",
        ] {
            assert!(check_model(valid).is_ok(), "expected {valid} to be ok");
        }

        let long_name = "a".repeat(129);
        for invalid in [
            "",
            " opus",
            "-m",
            "opus; rm -rf ~",
            "$(x)",
            "a b",
            "o'pus",
            &long_name,
            "opus🦀",
            "ずんだ",
        ] {
            let err = check_model(invalid).unwrap_err();
            assert!(
                err.contains(&format!("{invalid:?}")),
                "expected error to name refused value: {err}"
            );
            assert!(
                err.contains("1 to 128 ASCII characters"),
                "expected error to say what is allowed: {err}"
            );
        }
    }

    #[test]
    fn model_slot_categorises_templates_correctly() {
        assert_eq!(model_slot(DEFAULT_AGENT_RUNNER), ModelSlot::Appended);
        assert_eq!(model_slot(DEFAULT_AGENT_RESUME_RUNNER), ModelSlot::Appended);
        assert_eq!(
            model_slot("claude --model {model} {prompt}"),
            ModelSlot::Placeholder
        );
        assert_eq!(
            model_slot("myagent {model} {prompt}"),
            ModelSlot::Placeholder
        );

        match model_slot("codex exec {prompt}") {
            ModelSlot::Refused(reason) => {
                assert!(reason.contains("has no {model}"), "{reason}");
                assert!(reason.contains("codex"), "{reason}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }

        for template in [
            "claude --model opus {prompt}",
            "claude --model=opus {prompt}",
        ] {
            match model_slot(template) {
                ModelSlot::Refused(reason) => {
                    assert!(reason.contains("already passes --model"), "{reason}");
                }
                other => panic!("expected Refused, got {other:?}"),
            }
        }
    }

    #[test]
    fn shell_words_unquotes_and_reports_spans() {
        let s = r#"a 'b c' "d \"e\" \x" f\ g --m='x y'"#;
        let words = shell_words(s);
        let texts: Vec<&str> = words.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, ["a", "b c", "d \"e\" \\x", "f g", "--m=x y"]);
        assert_eq!(&s[words[1].0.clone()], "'b c'");
        assert_eq!(&s[words[4].0.clone()], "--m='x y'");
    }

    #[test]
    fn model_slot_recognises_a_quoted_claude_executable() {
        for template in [
            "\"/usr/local/bin/claude\" {prompt}",
            "'/opt/claude' {prompt}",
        ] {
            assert_eq!(model_slot(template), ModelSlot::Appended, "{template}");
        }
        assert!(matches!(
            model_slot("\"/usr/local/bin/claude\" --model opus {prompt}"),
            ModelSlot::Refused(_)
        ));
        let line = worker_command(
            Some("'/opt/claude' {prompt}"),
            &[],
            SID,
            "go",
            "/wt",
            "T",
            None,
            Some("opus"),
        );
        assert_eq!(line, "'/opt/claude' --model opus go");
    }

    #[test]
    fn model_slot_ignores_model_text_inside_quoted_values() {
        assert_eq!(
            model_slot("claude --append-system-prompt 'Use --model for selection' {prompt}"),
            ModelSlot::Appended
        );
        for template in [
            "claude --model opus {prompt}",
            "claude --model=opus {prompt}",
            "claude \"--model\" opus {prompt}",
            "claude '--model=opus' {prompt}",
        ] {
            assert!(
                matches!(model_slot(template), ModelSlot::Refused(_)),
                "{template}"
            );
        }
    }

    #[test]
    fn model_placeholder_removal_drops_every_occurrence_and_quoted_options() {
        for (template, expected) in [
            (
                "claude --model {model} --fallback-model {model} {prompt}",
                "claude go",
            ),
            ("claude \"--model\" {model} {prompt}", "claude go"),
            ("claude '-m' {model} {prompt}", "claude go"),
            ("claude '--model={model}' {prompt}", "claude go"),
            ("claude --verbose {model} {prompt}", "claude --verbose go"),
        ] {
            let line = worker_command(Some(template), &[], SID, "go", "/wt", "T", None, None);
            assert_eq!(line, expected, "{template}");
        }
    }

    #[test]
    fn model_placeholder_renders_quoted() {
        let out = worker_command(
            Some("myagent --model {model} {prompt}"),
            &[],
            SID,
            "go",
            "/wt",
            "T",
            None,
            Some("sonnet[1m]"),
        );
        assert_eq!(out, "myagent --model 'sonnet[1m]' go");

        let out = worker_command(
            Some("myagent --model \"{model}\" {prompt}"),
            &[],
            SID,
            "go",
            "/wt",
            "T",
            None,
            Some("opus"),
        );
        assert_eq!(out, "myagent --model opus go");
    }

    #[test]
    fn model_placeholder_removal_forms_with_no_model_leave_no_double_space() {
        assert_eq!(
            worker_command(
                Some("claude --verbose {model} {prompt}"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None,
                None,
            ),
            "claude --verbose go"
        );
        assert_eq!(
            worker_command(
                Some("claude -p {model} {prompt}"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None,
                None,
            ),
            "claude -p go"
        );
        assert_eq!(
            worker_command(
                Some("myagent --agent-model {model} {prompt}"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None,
                None,
            ),
            "myagent go"
        );
        assert_eq!(
            worker_command(
                Some("myagent --model \"{model}\" {prompt}"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None,
                None,
            ),
            "myagent go"
        );
        assert_eq!(
            worker_command(
                Some("myagent --model '{model}' {prompt}"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None,
                None,
            ),
            "myagent go"
        );
        assert_eq!(
            worker_command(
                Some("claude --model {model} {prompt}"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None,
                None,
            ),
            "claude go"
        );
        assert_eq!(
            worker_command(
                Some("claude -m {model} {prompt}"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None,
                None,
            ),
            "claude go"
        );
        assert_eq!(
            worker_command(
                Some("claude --model={model} {prompt}"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None,
                None,
            ),
            "claude go"
        );
        assert_eq!(
            worker_command(
                Some("claude {prompt} {model}"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None,
                None,
            ),
            "claude go"
        );
        assert_eq!(
            worker_command(
                Some("claude {model} {prompt}"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None,
                None,
            ),
            "claude go"
        );
        assert_eq!(
            worker_command(
                Some("{model} claude {prompt}"),
                &[],
                SID,
                "go",
                "/wt",
                "T",
                None,
                None,
            ),
            "claude go"
        );
    }

    #[test]
    fn default_runner_with_model_gives_exact_expected_line() {
        // Fresh with settings
        assert_eq!(
            worker_command(
                None,
                &[],
                SID,
                "go",
                "/wt",
                "T",
                Some(SETTINGS),
                Some("opus")
            ),
            format!(
                "claude --model opus --session-id {SID} --permission-mode auto --settings {SETTINGS} go"
            )
        );
        // Fresh without settings
        assert_eq!(
            worker_command(None, &[], SID, "go", "/wt", "T", None, Some("opus")),
            format!("claude --model opus --session-id {SID} --permission-mode auto go")
        );
        // Resume with settings
        assert_eq!(
            worker_resume_command(
                None,
                &[],
                SID,
                "go",
                "/wt",
                "T",
                Some(SETTINGS),
                Some("opus")
            ),
            format!(
                "claude --model opus --resume {SID} --permission-mode auto --settings {SETTINGS} go"
            )
        );
        // Resume without settings
        assert_eq!(
            worker_resume_command(None, &[], SID, "go", "/wt", "T", None, Some("opus")),
            format!("claude --model opus --resume {SID} --permission-mode auto go")
        );
    }
}
