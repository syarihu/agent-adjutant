//! The three procedures, baked into the binary.
//!
//! They are shipped as MCP prompts rather than written into each agent's commands directory
//! because a file that is copied into place is a file that drifts: upgrade the tool and the
//! copies stay behind, one per config directory, all subtly different. A prompt served over
//! MCP is a pointer — one upgrade moves every caller at once.
//!
//! The same text is also reachable as a tool (`adjutant_skill`), because MCP prompt support
//! is uneven across agents and a procedure nobody can fetch is a procedure nobody follows.

use crate::infra::agent::Agent;

mod render_skill;
pub use render_skill::*;

pub struct PromptDef {
    pub name: &'static str,
    pub raw_content: &'static str,
}

pub const PROMPTS: [PromptDef; 3] = [
    PromptDef {
        name: "adj-hub",
        raw_content: include_str!("../../commands/adj-hub.md"),
    },
    PromptDef {
        name: "adj-worker",
        raw_content: include_str!("../../commands/adj-worker.md"),
    },
    PromptDef {
        name: "adj-report",
        raw_content: include_str!("../../commands/adj-report.md"),
    },
];

/// The one thing worth spending always-on context on.
///
/// The 1500 lines of procedure matter only while a hub or a worker is running, and both
/// fetch them on purpose. This is the exception: a worker cannot ask for the report
/// procedure unless it already knows that reporting is a thing it is allowed to do.
pub const INSTRUCTIONS: &str = "\
adjutant hands work out to workers and takes their bug reports back in.
While you are working a task, a bug you find OUTSIDE that task is not yours to fix and not
yours to file: an unrelated fix pollutes this task's diff, and a diff nobody can review is a
diff nobody can revert. Hand it over instead — `adj skill adj-report` (or adjutant_skill,
name=adj-report), follow it, go back to your task. Hub: `adj hub`. Worker: adj-worker.";

pub fn find(name: &str) -> Option<&'static PromptDef> {
    PROMPTS.iter().find(|p| p.name == name)
}

/// Split a leading `---` block off the top, returning its lines and the body.
///
/// The frontmatter is an agent-specific header (a description, a tool allowlist) that means
/// nothing over MCP, but the description in it is the one-line summary a prompt listing
/// wants — so it is parsed rather than merely dropped.
pub fn strip_frontmatter(raw: &str) -> (Vec<(String, String)>, &str) {
    let Some(rest) = raw.strip_prefix("---\n") else {
        return (vec![], raw);
    };
    let Some(end) = rest.find("\n---\n") else {
        return (vec![], raw);
    };
    let (header, body) = rest.split_at(end);
    let fields = header
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    (fields, body["\n---\n".len()..].trim_start_matches('\n'))
}

pub fn description(prompt: &PromptDef) -> String {
    let (fields, _) = strip_frontmatter(prompt.raw_content);
    fields
        .iter()
        .find(|(k, _)| k == "description")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| prompt.name.to_string())
}

/// The body with `$ARGUMENTS` filled in.
///
/// A procedure invoked with nothing to say is normal (the hub is just starting up), so the
/// placeholder is replaced with an empty string rather than left as literal text for the
/// reader to puzzle over.
pub fn render(prompt: &PromptDef, arguments: &str) -> String {
    let (_, body) = strip_frontmatter(prompt.raw_content);
    body.replace("$ARGUMENTS", arguments)
}

/// Resolve which agent format to render procedures for.
///
/// `runner_agent` is the runner's program name, which `render_skill` reads with
/// `runner::agent_from_runner`.
pub fn resolve_agent(
    explicit: Option<&str>,
    client_name: Option<&str>,
    runner_agent: Option<&str>,
) -> Agent {
    if let Some(s) = explicit.and_then(Agent::parse) {
        return s;
    }
    if let Some(s) = std::env::var("ADJUTANT_AGENT")
        .ok()
        .as_deref()
        .and_then(Agent::parse)
    {
        return s;
    }
    if let Some(client) = client_name {
        let lower = client.to_ascii_lowercase();
        if lower.contains("agy") || lower.contains("antigravity") {
            return Agent::Agy;
        }
        if lower.contains("claude") {
            return Agent::Claude;
        }
    }
    if runner_agent.and_then(Agent::parse) == Some(Agent::Agy) {
        return Agent::Agy;
    }
    Agent::Claude
}

/// The procedure body rendered for a specific agent.
pub fn render_for(prompt: &PromptDef, arguments: &str, agent: Agent) -> String {
    let text = render(prompt, arguments);
    match agent {
        Agent::Claude => text,
        Agent::Agy => tailor_for_agy(&text),
        Agent::Generic => tailor_for_generic(&text),
    }
}

fn tailor_for_agy(text: &str) -> String {
    text.replace("`AskUserQuestion`", "`ask_question`")
        .replace("AskUserQuestion", "ask_question")
}

fn tailor_for_generic(text: &str) -> String {
    text.replace("`AskUserQuestion`", "`ask_question`")
        .replace("AskUserQuestion", "ask_question")
}

#[cfg(test)]
mod tests;
