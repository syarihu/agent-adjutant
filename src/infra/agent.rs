//! The kinds of agent adjutant knows how to start and talk to.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Agent {
    #[default]
    Claude,
    Agy,
    Generic,
}

impl Agent {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Agent::Claude),
            "agy" | "antigravity" => Some(Agent::Agy),
            "generic" | "codex" => Some(Agent::Generic),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Agy => "agy",
            Agent::Generic => "generic",
        }
    }
}
