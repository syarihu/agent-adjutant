//! One procedure, rendered for the agent that will read it: the one operation behind
//! `adj skill`, MCP `prompts/get` and `adjutant_skill`.
use std::fmt;
use std::path::Path;

use super::{Agent, PROMPTS, description, find, render_for, resolve_agent};
use crate::kernel::runner::{agent_from_runner, resolve_runner_for};

pub struct SkillRequest<'a> {
    pub name: &'a str,
    pub arguments: &'a str,
    /// As asked for; must be a name `Agent::parse` takes.
    pub agent: Option<&'a str>,
    /// The MCP client's name, when there is one.
    pub client_name: Option<&'a str>,
    /// Where the runner is looked up from; `None` is the current directory.
    pub runner_dir: Option<&'a Path>,
}

pub struct RenderedSkill {
    pub name: &'static str,
    pub description: String,
    pub agent: Agent,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillError {
    NoSuchProcedure {
        name: String,
        known: Vec<&'static str>,
    },
    UnknownAgent(String),
}

impl fmt::Display for SkillError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkillError::NoSuchProcedure { name, known } => {
                write!(f, "no such procedure: {name} ({})", known.join(" / "))
            }
            SkillError::UnknownAgent(value) => {
                write!(f, "agent must be claude, agy, or generic: {value}")
            }
        }
    }
}

pub fn render_skill(req: &SkillRequest) -> Result<RenderedSkill, SkillError> {
    let prompt = find(req.name).ok_or_else(|| SkillError::NoSuchProcedure {
        name: req.name.to_string(),
        known: PROMPTS.iter().map(|p| p.name).collect(),
    })?;
    if let Some(value) = req.agent
        && Agent::parse(value).is_none()
    {
        return Err(SkillError::UnknownAgent(value.to_string()));
    }
    // An agent that was asked for wins in `resolve_agent`, so the runner is only read without one.
    let runner_agent = match req.agent {
        Some(_) => None,
        None => resolve_runner_for(req.runner_dir, req.name)
            .as_deref()
            .map(agent_from_runner),
    };
    let agent = resolve_agent(req.agent, req.client_name, runner_agent.as_deref());
    Ok(RenderedSkill {
        name: prompt.name,
        description: description(prompt),
        agent,
        text: render_for(prompt, req.arguments, agent),
    })
}
