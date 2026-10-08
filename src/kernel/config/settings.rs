use super::*;
use serde_json::Value;

/// Whether the worker asks the human before requesting a Copilot review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopilotReview {
    Ask,
    Always,
    Never,
}

impl CopilotReview {
    /// How the brief writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            CopilotReview::Ask => "ask",
            CopilotReview::Always => "always",
            CopilotReview::Never => "never",
        }
    }

    /// Anything but "always" and "never" is `Ask`; `resolve_from_value` warns about it.
    fn read(value: Option<&Value>) -> Self {
        match value.and_then(Value::as_str) {
            Some("always") => CopilotReview::Always,
            Some("never") => CopilotReview::Never,
            _ => CopilotReview::Ask,
        }
    }
}

/// Which engine reads the diff in a self-review round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewEngine {
    Auto,
    Claude,
    Codex,
    /// A value that is none of the three. Holds the configured value's JSON text, so it is
    /// written as it was configured: `"sometimes"` with its quotes for a string, `3` for a
    /// number.
    Other(String),
}

impl ReviewEngine {
    pub fn as_str(&self) -> &str {
        match self {
            ReviewEngine::Auto => "auto",
            ReviewEngine::Claude => "claude",
            ReviewEngine::Codex => "codex",
            ReviewEngine::Other(text) => text,
        }
    }

    /// Absent is `Auto`, the built-in.
    fn read(value: Option<&Value>) -> Self {
        match value {
            None => ReviewEngine::Auto,
            Some(Value::String(s)) if s == "auto" => ReviewEngine::Auto,
            Some(Value::String(s)) if s == "claude" => ReviewEngine::Claude,
            Some(Value::String(s)) if s == "codex" => ReviewEngine::Codex,
            Some(other) => ReviewEngine::Other(other.to_string()),
        }
    }
}

/// `Default` is hand-written rather than derived because `startup_dashboard` is the one
/// field whose "nothing was configured" answer is not the type's zero. Derived, a
/// `Settings::default()` would say the dashboard is off, which is the opposite of what an
/// empty config means.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub terminal: TerminalSettings,
    /// Command template for telling the human something happened.
    pub notification: Hook,
    /// Command template for getting a *running hub's* attention once a message has landed
    /// in its inbox. A file appearing in a directory wakes nobody, and how you poke a live
    /// session is entirely a property of the terminal and the agent in front of you — so it
    /// is a template like everything else rather than a regression to live with.
    pub hub_wake: Wake,
    /// The same, for the other direction: a worker whose outbox just gained an entry.
    /// Separate from `hub_wake` because the two sessions are not always the same kind of
    /// thing — a worker may be a different agent, in a tab opened a different way.
    pub worker_wake: Wake,
    /// Command template that starts a worker agent. `None` = built-in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_runner: Option<String>,
    /// Environment the worker is started with. A repo that needs its own agent profile says
    /// so here, rather than the runner template learning one agent's variable names.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub agent_env: Vec<(String, String)>,
    /// Command template that starts the hub itself. `None` = built-in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hub_runner: Option<String>,
    /// Command template that reopens a worker's session (`adj worker --resume`). `None` =
    /// built-in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_resume_runner: Option<String>,
    /// Command template that reopens the hub's session (`adj hub --resume`). `None` =
    /// built-in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hub_resume_runner: Option<String>,
    /// Editor preset name or a command template containing `{worktree}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ide: Option<String>,
    /// Where a worktree goes when no convention tool answers. `None` = built-in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_pattern: Option<String>,
    /// Whether a starting hub hands the dashboard collection to a subagent before it goes to
    /// wait. Off means it comes up with nothing but its own settings read, and the listing is
    /// there for the asking instead.
    ///
    /// Never skipped when serialising, unlike the `Option` fields above it: the hub's
    /// procedure branches on this value, and a key that disappears when it is `true` makes
    /// the prompt read "absent" and "off" as the same thing. That reversal is silent — a hub
    /// that simply never collects, on the machine that changed nothing.
    pub startup_dashboard: bool,
    /// Whether the MCP server started under a hub also serves that hub's board, for as long
    /// as the hub runs. Off leaves the board to `adj serve`, started by hand.
    ///
    /// Never skipped when serialising, for the reason given for `startup_dashboard`.
    pub hub_serve: bool,
    /// How many hours after a hub ended a plain `adj hub` resumes it instead of starting a
    /// new one. `0` turns that off, leaving `--resume` as the only way back.
    pub hub_auto_resume_hours: f64,
    /// How many live workers one checkout may have at once. `adj work` refuses past it.
    /// `None` = no limit, which is what an empty config has always meant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_workers: Option<u32>,
    /// Minutes in one phase after which the board marks a worker as stuck. `0` turns the
    /// badge off; a dead worker is flagged regardless.
    pub stuck_after_minutes: f64,
    /// Command that prints the Jules API key on stdout. Built-in: the macOS keychain item
    /// `jules-api`. `false` turns handing tasks to Jules off.
    ///
    /// A command rather than the key: `adj config` prints every setting, and an agent reads
    /// that output. What it prints here is how to get the key, which is worth nothing without
    /// the keychain's consent.
    pub jules_key: Hook,
    /// The language the person reads what agents write for the board in, as it was written
    /// (`"ja"`, `"Japanese"`). `None` = not set: the hub uses the language the person talks
    /// to it in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Skipped when serialising, with the four below: the resolved `config` printed next to
    /// `settings` already carries these keys, and a second copy would be two answers to one
    /// question (see the note on `ide` in `defaults.rs`).
    #[serde(skip)]
    pub copilot_review: CopilotReview,
    /// Commands the worker runs before it reports. Empty for an unregistered repo.
    #[serde(skip)]
    pub verify: Vec<String>,
    #[serde(skip)]
    pub review_engine: ReviewEngine,
    /// Repository to issue-key prefix, as `issueKeys` spells it. Empty for an unregistered repo.
    #[serde(skip)]
    pub issue_keys: serde_json::Map<String, Value>,
    /// The normalised task sources, as `config.taskSources` holds them.
    #[serde(skip)]
    pub task_sources: Vec<Value>,
}

impl Settings {
    /// What `resolve_from_value` fills in for a registered repo, read from the merged `config`
    /// (`verify`, `copilotReview`, `reviewEngine`, `issueKeys`) and the normalised sources.
    pub(super) fn read_task_keys(
        &mut self,
        resolved: &serde_json::Map<String, Value>,
        sources: &[Value],
    ) {
        self.copilot_review = CopilotReview::read(resolved.get("copilotReview"));
        self.verify = resolved
            .get("verify")
            .and_then(Value::as_array)
            .map(|commands| {
                commands
                    .iter()
                    .map(|c| c.as_str().map_or_else(|| c.to_string(), str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        self.review_engine = ReviewEngine::read(resolved.get("reviewEngine"));
        self.issue_keys = resolved
            .get("issueKeys")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        self.task_sources = sources.to_vec();
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            terminal: TerminalSettings::default(),
            notification: Hook::default(),
            hub_wake: Wake::default(),
            worker_wake: Wake::default(),
            agent_runner: None,
            agent_env: Vec::new(),
            hub_runner: None,
            agent_resume_runner: None,
            hub_resume_runner: None,
            ide: None,
            worktree_pattern: None,
            startup_dashboard: true,
            hub_serve: true,
            hub_auto_resume_hours: DEFAULT_HUB_AUTO_RESUME_HOURS,
            max_workers: None,
            stuck_after_minutes: DEFAULT_STUCK_AFTER_MINUTES,
            jules_key: Hook::default(),
            language: None,
            copilot_review: CopilotReview::Ask,
            verify: Vec::new(),
            review_engine: ReviewEngine::Auto,
            issue_keys: serde_json::Map::new(),
            task_sources: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub registered: bool,
    pub config_path: String,
    pub warnings: Vec<String>,
    /// `None` when the repo has no entry. Absence is not an error: an unregistered repo can
    /// still spawn tabs and send reports, it just has no task sources to pick work from.
    pub config: Option<Value>,
    pub settings: Settings,
}
