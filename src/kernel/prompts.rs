//! The procedures, baked into the binary.
//!
//! Procedures come as a core (always read: role, general rules, and an index of triggers)
//! and sections fetched by id. An agent that loads 200 KB reads parts by guess; a trigger
//! index makes the part it needs the one it opens.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub id: &'static str,
    pub text: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection<'a> {
    Core,
    All,
    Ids(Vec<&'a str>),
}

impl<'a> Selection<'a> {
    pub fn parse(s: Option<&'a str>) -> Self {
        let Some(s) = s else {
            return Selection::Core;
        };
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed == "core" {
            return Selection::Core;
        }
        if trimmed == "all" {
            return Selection::All;
        }
        let ids: Vec<&'a str> = trimmed
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .collect();
        // Only commas is a mistyped list, not a request for the core: let it be refused as an id.
        if ids.is_empty() {
            Selection::Ids(vec![trimmed])
        } else {
            Selection::Ids(ids)
        }
    }
}

pub const PROMPTS: [PromptDef; 5] = [
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
    PromptDef {
        name: "adj-review-hub",
        raw_content: include_str!("../../commands/adj-review-hub.md"),
    },
    PromptDef {
        name: "adj-reviewer",
        raw_content: include_str!("../../commands/adj-reviewer.md"),
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
name=adj-report), follow it, go back to your task. Hub: `adj hub`. Worker: adj-worker.
Procedures come as a core and sections: read the core, and fetch the section its index names before the step it covers (adj skill <name> --section <id>, or adjutant_skill section).
After opening a PR from a worktree, before asking for a review: adj pr check-base --pr <n> (or adjutant_pr_check_base); if it fails, tell the user.
For the hub's Dashboard or a parent task's listing: adj collect [--parent <key>] --json (or adjutant_collect) collects the github/github-project sources instead of a collection sub-agent.";

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
#[cfg(test)]
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

/// The core and sections of a procedure.
pub fn parts(prompt: &PromptDef) -> (&'static str, Vec<Section>) {
    let (_, body) = strip_frontmatter(prompt.raw_content);
    parts_from_body(body)
}

/// Parse a markdown body into its core and sections, ignoring markers inside fenced code blocks.
pub fn parts_from_body(body: &'static str) -> (&'static str, Vec<Section>) {
    // The open fence's character and length: a ``` block is not closed by ~~~, nor the reverse.
    let mut fence: Option<(char, usize)> = None;
    let mut markers: Vec<(&'static str, usize)> = Vec::new();
    let mut byte_offset = 0;

    for line in body.split_inclusive('\n') {
        let trimmed_start = line.trim_start();
        let fence_char = trimmed_start
            .chars()
            .next()
            .filter(|c| matches!(c, '`' | '~'));
        let count = fence_char.map_or(0, |c| trimmed_start.chars().take_while(|&d| d == c).count());
        if let (Some(c), true) = (fence_char, count >= 3) {
            match fence {
                None => fence = Some((c, count)),
                Some((open, len)) if open == c && count >= len => fence = None,
                Some(_) => {}
            }
        } else if fence.is_none() {
            let stripped = line.trim_end_matches(['\r', '\n']);
            if let Some(id) = stripped
                .strip_prefix("<!-- section: ")
                .and_then(|rest| rest.strip_suffix(" -->"))
            {
                markers.push((id, byte_offset));
            }
        }
        byte_offset += line.len();
    }

    if markers.is_empty() {
        return (body, Vec::new());
    }

    let core = &body[..markers[0].1];
    let mut sections = Vec::with_capacity(markers.len());
    for i in 0..markers.len() {
        let (id, start) = markers[i];
        let end = if i + 1 < markers.len() {
            markers[i + 1].1
        } else {
            body.len()
        };
        sections.push(Section {
            id,
            text: &body[start..end],
        });
    }

    (core, sections)
}

/// Render a specific selection (core, all, or specific sections) of a procedure.
pub fn render_selection(
    prompt: &PromptDef,
    arguments: &str,
    agent: Agent,
    selection: &Selection<'_>,
) -> Result<String, SkillError> {
    let (core, sections) = parts(prompt);
    let raw_text = match selection {
        Selection::Core => core.to_string(),
        Selection::All => {
            let (_, body) = strip_frontmatter(prompt.raw_content);
            body.to_string()
        }
        Selection::Ids(ids) => {
            let known: Vec<&'static str> = sections.iter().map(|s| s.id).collect();
            if let Some(&section) = ids
                .iter()
                .find(|id| sections.is_empty() && !matches!(**id, "core" | "all"))
            {
                let section = section.to_string();
                return Err(SkillError::NoSuchSection {
                    name: prompt.name.to_string(),
                    section,
                    known: vec![],
                });
            }
            let mut pieces = Vec::with_capacity(ids.len());
            for &id in ids {
                if id == "core" {
                    pieces.push(core);
                } else if id == "all" {
                    let (_, body) = strip_frontmatter(prompt.raw_content);
                    pieces.push(body);
                } else if let Some(s) = sections.iter().find(|s| s.id == id) {
                    pieces.push(s.text);
                } else {
                    return Err(SkillError::NoSuchSection {
                        name: prompt.name.to_string(),
                        section: id.to_string(),
                        known,
                    });
                }
            }
            let joined = pieces
                .iter()
                .map(|p| p.trim_end_matches(['\r', '\n']))
                .collect::<Vec<_>>()
                .join("\n\n");
            if joined.is_empty() {
                String::new()
            } else {
                format!("{joined}\n")
            }
        }
    };
    let replaced = raw_text.replace("$ARGUMENTS", arguments);
    Ok(tailor(replaced, agent))
}

fn tailor(text: String, agent: Agent) -> String {
    match agent {
        Agent::Claude => text,
        Agent::Agy | Agent::Generic => text
            .replace("`AskUserQuestion`", "`ask_question`")
            .replace("AskUserQuestion", "ask_question"),
    }
}

#[cfg(test)]
mod tests;
