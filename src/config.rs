//! Reading `config.json` and turning it into the one shape the prompts are allowed to see.
//!
//! Everything mechanical about the config lives here — the defaults merge, the flat
//! shorthand, the required-key checks — because a rule written as prose in three prompt
//! files is a rule with three subtly different versions.
//!
//! The resolver never fails on a bad config. It returns warnings instead: a hub that
//! refuses to start because one repo entry is malformed is worse than a hub that starts and
//! says what is wrong.

use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// Falls back to `defaults`, then to these. `ide` has no built-in on purpose: guessing an
/// editor puts the user in the wrong one, and the prompts know how to ask.
pub fn builtin_defaults() -> Map<String, Value> {
    json!({
        "baseBranch": "auto",
        "reviewBots": [],
        "reviewEffort": "high",
        "reviewEngine": "auto",
        "selfReviewRounds": 5,
        "draftPr": true,
        // Whether the worker asks before requesting a Copilot review on the PR it opens.
        // Separate from `reviewBots`, which names the reviews to wait for, not whether to
        // ask for one.
        "copilotReview": "ask",
        "issueKeys": {},
        "verify": [],
        "postCreate": [],
        "onWorktreeRemove": [],
        // Named procedures the prompts hand off to — PR style, comment style, issue
        // creation. Absent means "skip that step", which is what makes the prompts usable
        // on a machine that has none of them.
        "skills": {},
    })
    .as_object()
    .cloned()
    .unwrap_or_default()
}

/// Keys that describe *a task source*, not the repo. In the flat shorthand they sit beside
/// a top-level `taskSource`; normalising lifts them into `taskSources[0]`.
pub const SOURCE_KEYS: [&str; 8] = [
    "issueRepo",
    "projectOwner",
    "projectNumber",
    "projectFields",
    "branchPattern",
    "worktreeName",
    "linear",
    "jira",
];

/// Trackers number their issues independently, so WID-233 and XYZ-233 both exist and a
/// key-less worktree name collides.
pub const DEFAULT_WORKTREE_NAME: &str = "{issuekey-lowercase}-{issue}";

/// Keys that configure *the machine*, not the work. They are resolved into `Settings` and
/// kept out of the per-repo config so there is only ever one copy of each.
const SETTING_KEYS: [&str; 18] = [
    "terminal",
    "notification",
    "agentRunner",
    "agentEnv",
    "hubRunner",
    // What `--resume` runs instead of the two above. Beside them for the reason they are here:
    // which agent a session is, and how it is reopened, is the machine's business.
    "agentResumeRunner",
    "hubResumeRunner",
    "wake",
    "hubWake",
    "workerWake",
    "worktreePattern",
    // Which editor to open a worktree in is a property of the machine, like the rest of
    // these. Left out of this list it survived into the per-repo config as well as into
    // `Settings`, so `adj config` answered with it twice and the two could disagree.
    "ide",
    // Whether the hub spends its first block dispatching the dashboard collector. Machine
    // level because it is about how somebody wants their hub to come up, not about the work
    // the repository holds — and it is here rather than in the prompt because the prompt is
    // the same text on every machine.
    "startupDashboard",
    // Whether a hub's MCP server serves that hub's board. About the machine — whether a
    // local port may be opened, and whether somebody runs `adj serve` themselves — like the
    // one above.
    "hubServe",
    // How recently a hub has to have ended for a plain `adj hub` to bring it back rather than
    // start a new one. About how somebody works, like the one above.
    "hubAutoResumeHours",
    // How many workers may run at once. A property of the machine — its memory, its CPU, the
    // agent's rate limit — and not of any one repository's work.
    "maxWorkers",
    // How long a worker may stay in one phase before the board flags it. About how somebody
    // works, like the two above it.
    "stuckAfterMinutes",
    // How to get the Jules API key. The key belongs to a person's account and sits in their
    // keychain, not to any one repository, so there is one answer per machine.
    "julesKey",
];

/// How long a worker may sit in one phase before the board calls it stuck, when nothing is
/// configured. Long enough for a review round or a slow build; short enough that a worker
/// that stopped at lunch is noticed in the afternoon.
pub const DEFAULT_STUCK_AFTER_MINUTES: f64 = 120.0;

/// The window a plain `adj hub` resumes in, when nothing is configured.
///
/// Long enough to cover an update, a crash, lunch; short enough that the first hub of the
/// morning starts clean. The hub is meant to be one a day, and a conversation that carries on
/// forever carries every task it ever dispatched along with it.
pub const DEFAULT_HUB_AUTO_RESUME_HOURS: f64 = 3.0;

/// Overrides `startupDashboard` for one hub, set by `adj hub --no-dashboard` / `--dashboard`.
///
/// A flag on a command that `exec`s an agent has no other way to reach the prompt: the hub
/// reads its settings through `adjutant_config`, which is served by an MCP server that is
/// the agent's own child, so the environment is the one channel that survives both hops.
/// The same trick `ADJUTANT_HUB` uses, for the same reason.
///
/// `"1"` and `"0"` and nothing else. Anything else falls through to the configured value
/// rather than picking a side, because a variable somebody exported with a typo in it should
/// not quietly reverse a setting they wrote down on purpose.
pub const STARTUP_DASHBOARD_ENV: &str = "ADJUTANT_STARTUP_DASHBOARD";

/// What each machine-level key is allowed to be.
///
/// The readers below take the shape they expect and ignore anything else, which turns a
/// typo into silence: `{"repos": [ … ]}` resolves to a machine with no settings and a
/// repository that was never registered, and nothing said so.
fn accepted_shape(key: &str) -> &'static [&'static str] {
    match key {
        "terminal" | "agentEnv" => &["an object"],
        "notification" | "wake" | "hubWake" | "workerWake" => &["a string", "false", "an object"],
        "julesKey" => &["a string", "false"],
        // The one knob that is a yes/no rather than a command line. Without its own arm it
        // fell through to the string default below, and every `true` anybody wrote was
        // reported as the wrong shape and dropped — a setting that warns when used correctly.
        "startupDashboard" | "hubServe" => &["true", "false"],
        "hubAutoResumeHours" | "maxWorkers" | "stuckAfterMinutes" => &["a number"],
        _ => &["a string"],
    }
}

fn shape_of(value: &Value) -> &'static str {
    match value {
        Value::Object(_) => "an object",
        Value::Array(_) => "an array",
        Value::String(_) => "a string",
        Value::Number(_) => "a number",
        Value::Bool(true) => "true",
        Value::Bool(false) => "false",
        Value::Null => "null",
    }
}

/// Say so when a setting is the wrong shape, naming where it was found. Dropping it is
/// still the behaviour — a half-understood setting is worse than none — but dropping it
/// quietly is what made a broken config look like an unregistered repository.
fn check_shapes(place: &str, map: &Map<String, Value>, warnings: &mut Vec<String>) {
    for key in SETTING_KEYS {
        let Some(value) = map.get(key) else { continue };
        if value.is_null() {
            continue;
        }
        let accepted = accepted_shape(key);
        if !accepted.contains(&shape_of(value)) {
            warnings.push(format!(
                "{place}{key} is {} but has to be {}: ignored",
                shape_of(value),
                accepted.join(" or ")
            ));
        }
    }
    // The long forms have inner keys, and they get the same treatment for the same reason:
    // a `command` that is a number reads as "unset", so the built-in runs instead of the
    // one that was asked for.
    for key in ["notification", "wake", "hubWake", "workerWake"] {
        let Some(command) = map
            .get(key)
            .and_then(Value::as_object)
            .and_then(|inner| inner.get("command"))
        else {
            continue;
        };
        if !command.is_null() && !["a string", "false"].contains(&shape_of(command)) {
            warnings.push(format!(
                "{place}{key}.command is {} but has to be a string or false: ignored",
                shape_of(command)
            ));
        }
    }
    // `line` is the other half of the long form and was dropped just as quietly: a session
    // then gets woken with the built-in sentence, which names tools the agent may not have.
    for key in ["wake", "hubWake", "workerWake"] {
        let Some(line) = map
            .get(key)
            .and_then(Value::as_object)
            .and_then(|inner| inner.get("line"))
        else {
            continue;
        };
        if !line.is_null() && shape_of(line) != "a string" {
            warnings.push(format!(
                "{place}{key}.line is {} but has to be a string: ignored",
                shape_of(line)
            ));
        }
    }
    let Some(terminal) = map.get("terminal").and_then(Value::as_object) else {
        return;
    };
    for (key, accepted) in [
        ("preset", &["a string"][..]),
        ("session", &["a string"][..]),
        ("socket", &["a string"][..]),
        ("spawn", &["a string"][..]),
        ("focus", &["a string"][..]),
        ("attach", &["a string"][..]),
        // `false` as well as a string, unlike its neighbours: `close` is the one of these
        // that destroys something, so "do not do this at all" has to be sayable.
        ("close", &["a string", "false"][..]),
        ("title", &["a string", "false"][..]),
    ] {
        let Some(value) = terminal.get(key) else {
            continue;
        };
        if !value.is_null() && !accepted.contains(&shape_of(value)) {
            warnings.push(format!(
                "{place}terminal.{key} is {} but has to be {}: ignored",
                shape_of(value),
                accepted.join(" or ")
            ));
        }
    }
}

/// A name `env` will pass on as a variable rather than as something else.
///
/// `with_env` builds `env K=V …` as a shell line, so a key is not just data: `K;rm -rf ~`
/// is a command separator sitting in the middle of the line that starts every worker for
/// that repository. Refused rather than sanitised — a name that needed sanitising was not
/// one of ours.
fn is_env_name(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn required_keys(source_type: &str) -> &'static [&'static str] {
    match source_type {
        "github" => &["issueRepo"],
        "github-project" => &["projectOwner", "projectNumber"],
        "jira" => &["jira.project", "jira.cloudId"],
        "linear" => &["linear.team"],
        _ => &[],
    }
}

// ── where the config lives ───────────────────────────────────────────

/// The file this binary reads its configuration out of. Named here rather than spelled in
/// each place that forwards it: a tab that is handed the wrong one reads a different world.
pub const CONFIG_ENV: &str = "ADJUTANT_CONFIG";
pub const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
pub const TMUX_SOCKET_ENV: &str = "ADJUTANT_TMUX_SOCKET";
pub const TMUX_SESSION_ENV: &str = "ADJUTANT_TMUX_SESSION";

/// `ADJUTANT_CONFIG` wins, then `$XDG_CONFIG_HOME/adjutant/config.json`, then
/// `~/.config/adjutant/config.json`. Not under a specific agent's config directory: the
/// point of this tool is that the same config serves whichever agent is driving.
pub fn config_path() -> PathBuf {
    if let Ok(path) = std::env::var(CONFIG_ENV)
        && !path.is_empty()
    {
        return expand_home(&path);
    }
    config_home().join("adjutant").join("config.json")
}

fn config_home() -> PathBuf {
    match std::env::var(XDG_CONFIG_HOME_ENV) {
        Ok(dir) if !dir.is_empty() => expand_home(&dir),
        _ => home_dir().join(".config"),
    }
}

/// `value` made absolute against `cwd` when it is a relative path, and `None` when it has to
/// be left exactly as it is: empty, already absolute, or starting with `~` the way
/// `expand_home` understands it (`~` or `~/...`), which is anchored to `HOME` rather than to
/// any directory.
fn anchored(value: &str, cwd: &Path) -> Option<PathBuf> {
    if value.is_empty() || value == "~" || value.starts_with("~/") {
        return None;
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return None;
    }
    std::path::absolute(cwd.join(path)).ok()
}

/// Make a relative `ADJUTANT_CONFIG` and `XDG_CONFIG_HOME` absolute, once, against the
/// directory this process was started in.
///
/// A relative value is otherwise resolved against whichever directory each process stands in,
/// and every process adjutant starts stands somewhere else — the hub moves to the main
/// checkout before it execs, and tabs open at the checkout or the worktree. Rewriting the
/// variable, rather than resolving it where it is read, is what makes the answer travel: exec
/// and spawned children inherit it, and `forwarded_env` hands the same value to a tab.
/// `ADJUTANT_STATE_DIR` is deliberately not included; see the comment above the directory move
/// in `hub`.
pub fn anchor_config_env() {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    for name in [CONFIG_ENV, XDG_CONFIG_HOME_ENV] {
        let Ok(value) = std::env::var(name) else {
            continue;
        };
        if let Some(path) = anchored(&value, &cwd) {
            // SAFETY: called first thing in `run`, before any thread is started.
            unsafe { std::env::set_var(name, path) };
        }
    }
}

/// Where `~` points, and the anchor under which every path this program uses is derived.
///
/// An unset `HOME` used to make that anchor the empty string, which left the state
/// directory *relative*: it landed under whatever directory the process happened to start
/// in, so a hub and a worker started from different places read different inboxes — and
/// neither is wrong about anything it can see, which is why nobody would find it. An
/// absolute fallback keeps the two agreeing, and saying so on stderr is the only way the
/// person running them learns that `HOME` is missing.
pub fn home_dir() -> PathBuf {
    home_from(std::env::var("HOME").ok().as_deref())
}

/// Split out from `home_dir` so the answer can be checked without a test reaching into the
/// environment every other test is reading.
fn home_from(home: Option<&str>) -> PathBuf {
    match home {
        Some(home) if home.starts_with('/') => PathBuf::from(home),
        _ => {
            // Per person, and absolute whatever `TMPDIR` says. `temp_dir()` is one shared
            // directory on most Linux systems and follows a `TMPDIR` that may itself be
            // relative — so a bare name under it would put two people's configs and
            // inboxes in one place, and a relative `TMPDIR` would undo the very thing this
            // fallback is for.
            let temp = std::env::temp_dir();
            let temp = match temp.is_absolute() {
                true => temp,
                false => PathBuf::from("/tmp"),
            };
            let whose = ["USER", "LOGNAME"]
                .iter()
                .find_map(|key| std::env::var(key).ok())
                .filter(|name| !name.is_empty() && !name.contains('/'))
                .unwrap_or_else(|| "unknown".to_string());
            let fallback = temp.join(format!("adjutant-no-home-{whose}"));
            static SAID: std::sync::Once = std::sync::Once::new();
            SAID.call_once(|| {
                eprintln!(
                    "adjutant: HOME is not set to an absolute path, falling back to {} — set HOME so every session agrees on one location",
                    fallback.display()
                );
            });
            fallback
        }
    }
}

pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home_dir().join(rest),
        None if path == "~" => home_dir(),
        None => PathBuf::from(path),
    }
}

// ── the resolved shape ───────────────────────────────────────────────

/// A behaviour with three states rather than two.
///
/// "Unset" and "off" are different answers and both are needed: a machine with no
/// notifier still wants the built-in one tried, and a person who does not want to be
/// interrupted needs a way to say so that an empty string cannot express (an empty
/// string is indistinguishable from a typo).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Hook {
    /// Nothing said — use whatever this platform does out of the box.
    #[default]
    BuiltIn,
    /// Deliberately disabled.
    Off,
    /// A command template.
    Command(String),
}

impl Hook {
    fn read(value: Option<Value>) -> Self {
        match value {
            Some(Value::Bool(false)) => Hook::Off,
            Some(Value::String(s)) if !s.is_empty() => Hook::Command(s),
            _ => Hook::BuiltIn,
        }
    }

    /// The template to render, or `None` for both "built-in" and "off" — the caller tells
    /// them apart with `is_off`.
    pub fn template(&self) -> Option<&str> {
        match self {
            Hook::Command(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_off(&self) -> bool {
        matches!(self, Hook::Off)
    }
}

impl serde::Serialize for Hook {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Hook::BuiltIn => serializer.serialize_str("(built-in)"),
            Hook::Off => serializer.serialize_bool(false),
            Hook::Command(s) => serializer.serialize_str(s),
        }
    }
}

/// Waking a session: how to poke it, and what to say once poked.
///
/// The two are separate because they vary independently. *How* is a property of the
/// terminal — the same `write text` reaches every agent running in it. *What to say* is a
/// property of the agent: the default sentence names MCP tools, which is the wrong
/// instruction for an agent that only has the CLI, or one that wants a slash command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wake {
    pub hook: Hook,
    /// The sentence typed at the woken session. `None` = the built-in for that direction.
    pub line: Option<String>,
}

impl Wake {
    fn read(value: Option<Value>) -> Self {
        match value {
            // `{"command": …, "line": …}` — either half may be omitted, so a config can
            // change what is said without restating how to say it.
            Some(Value::Object(map)) => Wake {
                hook: Hook::read(map.get("command").cloned()),
                line: line_of(&map),
            },
            other => Wake {
                hook: Hook::read(other),
                line: None,
            },
        }
    }

    /// `wake` says how this machine pokes a session; `hubWake` / `workerWake` override it
    /// for one direction. Written as an overlay rather than a plain override so that
    /// changing only the sentence for one direction does not silently drop the mechanism
    /// the machine was configured with.
    fn resolve(base: Option<Value>, specific: Option<Value>) -> Self {
        let mut wake = Wake::read(base);
        match specific {
            None | Some(Value::Null) => {}
            Some(Value::Object(map)) => {
                if map.contains_key("command") {
                    wake.hook = Hook::read(map.get("command").cloned());
                }
                if let Some(line) = line_of(&map) {
                    wake.line = Some(line);
                }
            }
            // A bare value names a mechanism, and says nothing about the sentence.
            other => wake.hook = Hook::read(other),
        }
        wake
    }

    /// What to type, falling back to the caller's built-in for this direction.
    pub fn line_or<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.line.as_deref().unwrap_or(fallback)
    }
}

fn line_of(map: &Map<String, Value>) -> Option<String> {
    map.get("line")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl serde::Serialize for Wake {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.line {
            None => self.hook.serialize(serializer),
            Some(line) => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("command", &self.hook)?;
                map.serialize_entry("line", line)?;
                map.end()
            }
        }
    }
}

/// How to open a tab, raise one, name this one, and get a running session's attention.
///
/// All four are replaceable for the same reason: the tool has no business knowing which
/// terminal is in front of the person using it.
/// Serialised in the same spelling the config file uses. A reader that has just written
/// `hubWake` should be able to find `hubWake` in the resolved output; two spellings for one
/// key is a lookup that silently misses.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    /// How the board opens a tmux session in the person's own terminal. Its own template
    /// because `spawn` opens a tmux *window*, and attaching from inside tmux would nest one in
    /// the other. Unset, the board uses iTerm2 where it is installed and otherwise says the
    /// key is missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attach: Option<String>,
    /// Close a worker's tab once its task is over. Separate from `focus` because raising a
    /// tab and disposing of one are different verbs in every terminal, and a machine that
    /// can do one cannot be assumed to do the other with the same command line.
    ///
    /// A `Hook` rather than a plain template, which `spawn` and `focus` get away with: the
    /// documented promise is that any of these can be turned off with `false`, and a
    /// `false` read as a string reads as "unset" — so `"close": false` would fall through
    /// to the built-in closer and dispose of the tab somebody had just said not to touch.
    pub close: Hook,
    /// Name the tab this process is running in. Distinct from `spawn`'s title, which names
    /// a tab being created.
    pub title: Hook,
}

impl TerminalSettings {
    pub fn is_tmux(&self) -> bool {
        self.preset.as_deref() == Some("tmux")
    }

    pub fn tmux_session(&self) -> &str {
        self.session.as_deref().unwrap_or("adjutant")
    }

    pub fn tmux_socket(&self) -> Option<&str> {
        self.socket.as_deref()
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

// ── the resolver ─────────────────────────────────────────────────────

/// Drop the `//` documentation keys. They are the schema's prose, and reprinting them on
/// every resolve is pure transcript weight for the session reading this.
pub fn strip_comments(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                // Any key that starts with `//`, not just the bare one: the schema also
                // documents individual keys with a `//<key>` sibling, and those are prose
                // too. Filtering only the exact `"//"` left every one of them in the
                // resolved config, which is transcript weight for whoever reads it.
                .filter(|(k, _)| !k.starts_with("//"))
                .map(|(k, v)| (k.clone(), strip_comments(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_comments).collect()),
        other => other.clone(),
    }
}

fn lookup_entry<'a>(repos: &'a Map<String, Value>, nwo: &str) -> Option<&'a Value> {
    repos.get(nwo).or_else(|| {
        repos
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(nwo))
            .map(|(_, v)| v)
    })
}

/// Flat shorthand -> `taskSources`. A top-level `taskSource` plus its sibling keys is one
/// source; that is what keeps single-source configs readable.
pub fn normalise_sources(entry: &Map<String, Value>, warnings: &mut Vec<String>) -> Vec<Value> {
    if let Some(sources) = entry.get("taskSources") {
        let Some(list) = sources.as_array() else {
            warnings.push("taskSources is not an array".to_string());
            return vec![];
        };
        return list.iter().filter(|s| s.is_object()).cloned().collect();
    }
    if let Some(kind) = entry.get("taskSource") {
        let mut source = Map::new();
        source.insert("type".to_string(), kind.clone());
        for key in SOURCE_KEYS {
            if let Some(value) = entry.get(key) {
                source.insert(key.to_string(), value.clone());
            }
        }
        return vec![Value::Object(source)];
    }
    vec![]
}

fn missing_source_keys(source: &Map<String, Value>) -> Vec<&'static str> {
    let kind = source.get("type").and_then(Value::as_str).unwrap_or("");
    required_keys(kind)
        .iter()
        .copied()
        .filter(|path| {
            let value = match path.split_once('.') {
                Some((head, tail)) => source.get(head).and_then(|v| v.get(tail)),
                None => source.get(*path),
            };
            match value {
                None | Some(Value::Null) => true,
                Some(Value::String(s)) => s.is_empty(),
                Some(Value::Array(a)) => a.is_empty(),
                Some(Value::Object(o)) => o.is_empty(),
                _ => false,
            }
        })
        .collect()
}

fn as_object(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .map(|m| strip_comments(&Value::Object(m.clone())))
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// What a key says at the most specific level that names it: repo entry > `defaults` > top
/// level.
///
/// A free function rather than the closure it used to be so that a test can ask it which
/// level won without going through `resolve_settings`, whose answer for `startupDashboard`
/// also depends on the environment. Checking that precedence against the resolved `bool`
/// meant a `cargo test` run in a hub's own tab was answering about that hub's flag rather
/// than about the fixture in front of it — which is the same trap `Sandbox::new` clears
/// `ADJUTANT_STARTUP_DASHBOARD` for, and these tests hold no sandbox.
///
/// The null filter is deliberately after the chain, not inside it: an explicit `null` at the
/// most specific level means "unset", and is not a hole for the level below to show through.
fn pick_level(
    entry: &Map<String, Value>,
    defaults: &Map<String, Value>,
    root: &Map<String, Value>,
    key: &str,
) -> Option<Value> {
    entry
        .get(key)
        .or_else(|| defaults.get(key))
        .or_else(|| root.get(key))
        .filter(|v| !v.is_null())
        .cloned()
}

/// Machine-level knobs, most specific wins: repo entry > `defaults` > top level > built-in.
///
/// The top level is where `terminal` and `notification` normally sit — they describe the
/// machine, and repeating them per repo is how they drift apart.
///
/// `startup_flag` is `ADJUTANT_STARTUP_DASHBOARD` as the entry point found it, handed down
/// rather than read here. See `resolve_config`, which is the one place that looks.
fn resolve_settings(
    root: &Map<String, Value>,
    defaults: &Map<String, Value>,
    entry: &Map<String, Value>,
    startup_flag: Option<&str>,
    warnings: &mut Vec<String>,
) -> Settings {
    for (place, map) in [
        ("", root),
        ("defaults: ", defaults),
        ("this repository: ", entry),
    ] {
        check_shapes(place, map, warnings);
    }
    let pick = |key: &str| -> Option<Value> { pick_level(entry, defaults, root, key) };
    let pick_str = |key: &str| -> Option<String> {
        pick(key)
            .and_then(|v| v.as_str().map(str::to_string))
            .filter(|s| !s.is_empty())
    };
    // Merged key by key rather than picked whole, which is what `Wake::resolve` already
    // does and for the same reason: a repository that wants a different tab title should
    // not have to restate how tabs are opened and raised. Picking the object entire meant
    // `{"terminal": {"title": "…"}}` on one repo silently dropped that machine's `spawn`
    // and `focus`, and the symptom is a tab that never opens.
    let terminal = overlay(&[root, defaults, entry], "terminal");
    let merged = |key: &str| -> Option<Value> {
        let mut answer: Option<Value> = None;
        for level in [root, defaults, entry] {
            let Some(value) = level.get(key).filter(|v| !v.is_null()) else {
                continue;
            };
            answer = match (answer, value) {
                // Two objects at two levels: the more specific one overrides the keys it
                // names and leaves the rest standing.
                (Some(Value::Object(mut base)), Value::Object(more)) => {
                    for (k, v) in more {
                        base.insert(k.clone(), v.clone());
                    }
                    Some(Value::Object(base))
                }
                // Anything else replaces: a bare string or `false` names a whole mechanism,
                // and half-merging that with an object would invent a setting nobody wrote.
                _ => Some(value.clone()),
            };
        }
        answer
    };
    let str_field = |map: &Map<String, Value>, key: &str| -> Option<String> {
        map.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    // The one setting whose answer comes partly from outside the config file. Picked here,
    // decided by a function of two arguments, and the outside half arrives as one of them.
    let configured_dashboard = pick("startupDashboard");
    let preset = str_field(&terminal, "preset");
    let session = std::env::var(TMUX_SESSION_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| str_field(&terminal, "session"))
        .or_else(|| {
            if preset.as_deref() == Some("tmux") {
                Some("adjutant".to_string())
            } else {
                None
            }
        });
    let socket = std::env::var(TMUX_SOCKET_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| str_field(&terminal, "socket"));
    Settings {
        terminal: TerminalSettings {
            preset,
            session,
            socket,
            spawn: str_field(&terminal, "spawn"),
            focus: str_field(&terminal, "focus"),
            attach: str_field(&terminal, "attach"),
            close: Hook::read(terminal.get("close").cloned()),
            title: Hook::read(terminal.get("title").cloned()),
        },
        notification: match pick("notification") {
            // Accepts either `"notification": "cmd"` or `"notification": {"command": "cmd"}`
            // — the object form leaves room for later keys without a breaking change. The
            // inner value goes to `Hook::read` whole rather than through a string filter,
            // which is what `{"command": false}` needs to mean off: filtered, `false` was
            // not a string, so it read as unset and the built-in notifier ran anyway.
            Some(Value::Object(map)) => Hook::read(map.get("command").cloned()),
            other => Hook::read(other),
        },
        // Merged down the levels first, then across the directions. `pick` alone took the
        // most specific level whole, so a repository that set `hubWake: {"line": …}` threw
        // away the machine's `hubWake.command` — the very thing `Wake::resolve` exists to
        // stop happening between `wake` and `hubWake`, one level up.
        hub_wake: Wake::resolve(merged("wake"), merged("hubWake")),
        worker_wake: Wake::resolve(merged("wake"), merged("workerWake")),
        agent_runner: pick_str("agentRunner"),
        hub_runner: pick_str("hubRunner"),
        agent_resume_runner: pick_str("agentResumeRunner"),
        hub_resume_runner: pick_str("hubResumeRunner"),
        agent_env: agent_env(pick("agentEnv"), warnings),
        ide: pick_str("ide"),
        worktree_pattern: pick_str("worktreePattern"),
        // Read here rather than where the flag is parsed, so that there is one answer to the
        // question. `adj config` and the MCP tool both come out of this function, and the
        // agent asks the tool — resolving the override in the command layer would leave the
        // hub reading a `settings` block that disagrees with the flag it was started under.
        startup_dashboard: startup_dashboard(configured_dashboard.as_ref(), startup_flag),
        // A non-boolean was already reported by `check_shapes`, and falls back to the default.
        hub_serve: pick("hubServe").and_then(|v| v.as_bool()).unwrap_or(true),
        hub_auto_resume_hours: auto_resume_hours(pick("hubAutoResumeHours").as_ref(), warnings),
        max_workers: max_workers(pick("maxWorkers").as_ref(), warnings),
        stuck_after_minutes: stuck_after_minutes(pick("stuckAfterMinutes").as_ref(), warnings),
        jules_key: Hook::read(pick("julesKey")),
    }
}

/// `ADJUTANT_STARTUP_DASHBOARD` over the configured value over `true`.
///
/// The environment wins because it is how a flag typed just now reaches an agent that is
/// three processes away; the config is the standing preference, and a standing preference
/// that a person can no longer override for one session is a setting they end up editing
/// twice a day.
///
/// Both inputs are arguments, and the variable itself is read at the entry point rather than
/// here — see `resolve_config`. A test that had to set the variable to exercise this would be
/// writing process-global state, and most of the tests around it resolve a config without
/// taking any lock at all, so the one that wrote would be read by whichever of its siblings
/// happened to be running beside it. Passed as a parameter, every case is decided by an
/// ordinary function call and nothing in this file has to touch the environment at all.
fn startup_dashboard(configured: Option<&Value>, flag: Option<&str>) -> bool {
    match flag {
        Some("1") => return true,
        Some("0") => return false,
        // Anything else is not an answer. Falling through beats guessing: the variable is
        // inherited by everything a hub starts, so one malformed export would otherwise
        // follow the person into every session they open from there.
        _ => {}
    }
    configured.and_then(Value::as_bool).unwrap_or(true)
}

/// The auto-resume window, in hours. A number that is not a finite, non-negative one is said
/// and replaced by the default — a negative window would read as "never", which is what `0`
/// is for, and saying nothing would leave somebody wondering why their hubs never came back.
fn auto_resume_hours(configured: Option<&Value>, warnings: &mut Vec<String>) -> f64 {
    let Some(value) = configured else {
        return DEFAULT_HUB_AUTO_RESUME_HOURS;
    };
    match value.as_f64() {
        Some(hours) if hours.is_finite() && hours >= 0.0 => hours,
        // A non-number was already reported by `check_shapes`.
        Some(_) => {
            warnings.push(format!(
                "hubAutoResumeHours is {value} but has to be 0 or more: using {DEFAULT_HUB_AUTO_RESUME_HOURS}"
            ));
            DEFAULT_HUB_AUTO_RESUME_HOURS
        }
        None => DEFAULT_HUB_AUTO_RESUME_HOURS,
    }
}

/// The stuck threshold, in minutes. Negative or not finite is said and replaced by the
/// default, as `hubAutoResumeHours` does; `0` is the way to turn it off.
fn stuck_after_minutes(configured: Option<&Value>, warnings: &mut Vec<String>) -> f64 {
    let Some(value) = configured else {
        return DEFAULT_STUCK_AFTER_MINUTES;
    };
    match value.as_f64() {
        Some(minutes) if minutes.is_finite() && minutes >= 0.0 => minutes,
        // A non-number was already reported by `check_shapes`.
        Some(_) => {
            warnings.push(format!(
                "stuckAfterMinutes is {value} but has to be 0 or more: using {DEFAULT_STUCK_AFTER_MINUTES}"
            ));
            DEFAULT_STUCK_AFTER_MINUTES
        }
        None => DEFAULT_STUCK_AFTER_MINUTES,
    }
}

/// The worker limit. Anything but a whole number of 1 or more is said and dropped, and dropped
/// means no limit: `0` would refuse every dispatch, which nobody writes on purpose, and
/// rounding `2.5` either way is guessing at what was meant.
fn max_workers(configured: Option<&Value>, warnings: &mut Vec<String>) -> Option<u32> {
    let value = configured?;
    // A non-number was already reported by `check_shapes`.
    if !value.is_number() {
        return None;
    }
    match value.as_u64().filter(|n| *n >= 1).map(u32::try_from) {
        Some(Ok(n)) => Some(n),
        _ => {
            warnings.push(format!(
                "maxWorkers is {value} but has to be a whole number of 1 or more: not limiting workers"
            ));
            None
        }
    }
}

/// The hub's name is its *address*: a worker derives it, finds the session record under it
/// and sends there. A `hubRunner` with nowhere to put the name still starts something, and
/// presence still works — that rests on the recorded start time, not on the name — but the
/// session is then nameless in every listing a person reads.
///
/// A resume template with no `{sessionId}` is the other thing worth saying: it has no way to
/// be told which conversation to reopen, so every `--resume` through it would open whatever
/// the agent picks on its own.
///
/// `{prompt}` is appended when a template forgets it; `{name}` cannot be, because where it
/// goes is the agent's own flag. So the only thing to do is say so.
fn check_runner(settings: &Settings, warnings: &mut Vec<String>) {
    if let Some(template) = &settings.hub_runner
        && !template.contains("{name}")
    {
        warnings.push(
            "hubRunner has no {name}: the hub still runs, but nothing a person reads will show which session it is".to_string(),
        );
    }
    for (key, template) in [
        ("hubResumeRunner", &settings.hub_resume_runner),
        ("agentResumeRunner", &settings.agent_resume_runner),
    ] {
        if let Some(template) = template
            && !template.contains("{sessionId}")
        {
            warnings.push(format!(
                "{key} has no {{sessionId}}: --resume could not say which session to reopen, so it refuses to run"
            ));
        }
    }
}

/// One object built from all three levels, most specific last. Only keys that are present
/// override; a level that says nothing about a key leaves the one below it standing.
fn overlay(levels: &[&Map<String, Value>], key: &str) -> Map<String, Value> {
    let mut merged = Map::new();
    for level in levels {
        let Some(map) = level.get(key).and_then(Value::as_object) else {
            continue;
        };
        for (inner, value) in map {
            if !value.is_null() {
                merged.insert(inner.clone(), value.clone());
            }
        }
    }
    merged
}

/// The variables a worker is started with, minus anything that is not one.
fn agent_env(value: Option<Value>, warnings: &mut Vec<String>) -> Vec<(String, String)> {
    let Some(map) = value.and_then(|v| v.as_object().cloned()) else {
        return vec![];
    };
    let mut env = Vec::new();
    for (key, value) in map {
        if !is_env_name(&key) {
            warnings.push(format!(
                "agentEnv {key} is not a variable name: dropped before it reaches a command line"
            ));
            continue;
        }
        match value.as_str() {
            Some(value) => env.push((key, value.to_string())),
            None => warnings.push(format!(
                "agentEnv {key} is {} but has to be a string: ignored",
                shape_of(&value)
            )),
        }
    }
    env
}

/// Resolve one repo's entry out of an already-parsed config document.
///
/// Split from the file reading so it can be tested as what it is: JSON in, JSON out. That
/// claim is only true while it stays true of the whole call tree, which is why
/// `startup_flag` is threaded through rather than read where it is used — see
/// `resolve_config`.
pub fn resolve_from_value(
    raw: &Value,
    nwo: &str,
    startup_flag: Option<&str>,
) -> (bool, Option<Value>, Settings, Vec<String>) {
    let mut warnings: Vec<String> = Vec::new();
    let root = raw.as_object().cloned().unwrap_or_default();
    if !raw.is_object() {
        warnings.push(format!(
            "the config is {} but has to be an object: nothing in it is read",
            shape_of(raw)
        ));
    }
    // These three used to be read with "an object, or else nothing", and the difference
    // between "no repositories" and "repos is an array" was invisible in the answer.
    for (key, value) in [
        ("repos", root.get("repos")),
        ("defaults", root.get("defaults")),
    ] {
        if let Some(value) = value.filter(|v| !v.is_null() && !v.is_object()) {
            warnings.push(format!(
                "{key} is {} but has to be an object: ignored",
                shape_of(value)
            ));
        }
    }
    let repos = root
        .get("repos")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut defaults = as_object(root.get("defaults"));
    // Sources are the one thing `defaults` must not supply: merged into an entry that
    // already declares its own, a default source adds a phantom with no issueRepo behind it.
    for key in ["taskSource", "taskSources"] {
        if defaults.remove(key).is_some() {
            warnings.push(format!(
                "ignored {key} in defaults: task sources are not inherited"
            ));
        }
    }

    let Some(entry_raw) = lookup_entry(&repos, nwo) else {
        let settings = resolve_settings(&root, &defaults, &Map::new(), startup_flag, &mut warnings);
        check_runner(&settings, &mut warnings);
        return (false, None, settings, warnings);
    };
    if !entry_raw.is_object() {
        warnings.push(format!(
            "this repository's entry is {} but has to be an object: it is read as empty",
            shape_of(entry_raw)
        ));
    }
    let entry = as_object(Some(entry_raw));
    let settings = resolve_settings(&root, &defaults, &entry, startup_flag, &mut warnings);
    check_runner(&settings, &mut warnings);

    let mut resolved = builtin_defaults();
    resolved.extend(defaults.clone());
    resolved.extend(entry.clone());

    let mut sources = normalise_sources(&entry, &mut warnings);
    for key in std::iter::once("taskSource")
        .chain(SOURCE_KEYS)
        .chain(SETTING_KEYS)
    {
        resolved.remove(key);
    }

    for (index, source) in sources.iter_mut().enumerate() {
        let Some(map) = source.as_object_mut() else {
            continue;
        };
        let kind = map
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if kind.is_empty() {
            warnings.push(format!("taskSources[{index}] has no type"));
            continue;
        }
        map.entry("worktreeName")
            .or_insert_with(|| Value::String(DEFAULT_WORKTREE_NAME.to_string()));
        let missing = missing_source_keys(map);
        if !missing.is_empty() {
            warnings.push(format!(
                "taskSources[{index}] ({kind}) is missing {}",
                missing.join(", ")
            ));
        }
    }
    resolved.insert("taskSources".to_string(), Value::Array(sources.clone()));

    if sources.is_empty() {
        warnings.push("no task sources: treating this repository as unregistered".to_string());
    }
    if settings.ide.is_none() {
        warnings.push("ide is not set".to_string());
    }

    let issue_keys = resolved
        .get("issueKeys")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut seen: Vec<(String, Vec<String>)> = Vec::new();
    for (repo, key) in &issue_keys {
        let key = key.as_str().unwrap_or_default().to_string();
        match seen.iter_mut().find(|(k, _)| *k == key) {
            Some((_, repos)) => repos.push(repo.clone()),
            None => seen.push((key, vec![repo.clone()])),
        }
    }
    for (key, repos) in &seen {
        if repos.len() > 1 {
            warnings.push(format!(
                "issueKeys value {key} is used more than once: {}",
                repos.join(", ")
            ));
        }
    }

    // `adj task brief` copies this into the worker brief verbatim, and the worker falls back to asking
    // on anything it does not recognise. Without a warning, a typo such as "alway" would
    // quietly keep the question coming while the person believes it is switched off.
    if let Some(value) = resolved.get("copilotReview")
        && !matches!(value.as_str(), Some("ask" | "always" | "never"))
    {
        warnings.push(format!(
            "copilotReview {value} is not one of ask, always, never: treated as ask"
        ));
    }

    // A github issue with no key has no branch name, so it silently drops out of the list.
    // jira and linear carry their own key and never need issueKeys.
    for (index, source) in sources.iter().enumerate() {
        let kind = source.get("type").and_then(Value::as_str).unwrap_or("");
        if kind != "github" && kind != "github-project" {
            continue;
        }
        let repo = source.get("issueRepo").and_then(Value::as_str);
        match repo {
            Some(repo) if !issue_keys.contains_key(repo) => warnings.push(format!(
                "taskSources[{index}] issueRepo {repo} is not in issueKeys: issues from that repository are skipped"
            )),
            None if issue_keys.is_empty() => warnings.push(format!(
                "taskSources[{index}] ({kind}) has no issueKeys: every issue on the board is skipped"
            )),
            _ => {}
        }
    }

    (true, Some(Value::Object(resolved)), settings, warnings)
}

/// The runtime entry point: the config file on disk, plus the one answer that does not come
/// from it.
///
/// This is the only place `ADJUTANT_STARTUP_DASHBOARD` is read — `config_path` above has its
/// own reasons to look at the environment, and they are about *which file*, not what is in
/// it. Everything below this takes the value as an argument, which is what lets the rest of
/// the resolver be tested as a function of its inputs — the tests call it
/// directly, in parallel, holding no lock, and a variable exported by the hub whose tab
/// `cargo test` was typed in cannot reach them. Read one layer down instead, it could: the
/// suite would be answering about that hub's `--no-dashboard` rather than about its fixture.
pub fn resolve_config(nwo: &str) -> Result<Resolved, String> {
    let path = config_path();
    let shown = path.to_string_lossy().to_string();
    let startup_flag = std::env::var(STARTUP_DASHBOARD_ENV).ok();
    let startup_flag = startup_flag.as_deref();
    if !path.exists() {
        // No config at all is the first-run state, not a failure. Everything that does not
        // need task sources — spawn, send, notify — still works off the built-ins.
        let (_, _, settings, _) = resolve_from_value(&json!({}), nwo, startup_flag);
        return Ok(Resolved {
            registered: false,
            config_path: shown.clone(),
            warnings: vec![format!("{shown} does not exist")],
            config: None,
            settings,
        });
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {shown}: {e}"))?;
    let raw: Value =
        serde_json::from_str(&text).map_err(|e| format!("cannot parse {shown}: {e}"))?;
    let (registered, config, settings, warnings) = resolve_from_value(&raw, nwo, startup_flag);
    Ok(Resolved {
        registered,
        config_path: shown,
        warnings,
        config,
        settings,
    })
}

#[cfg(test)]
mod tests;
