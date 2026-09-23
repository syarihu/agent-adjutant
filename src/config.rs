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
use std::path::PathBuf;

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
const SETTING_KEYS: [&str; 14] = [
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
    // How recently a hub has to have ended for a plain `adj hub` to bring it back rather than
    // start a new one. About how somebody works, like the one above.
    "hubAutoResumeHours",
];

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
        // The one knob that is a yes/no rather than a command line. Without its own arm it
        // fell through to the string default below, and every `true` anybody wrote was
        // reported as the wrong shape and dropped — a setting that warns when used correctly.
        "startupDashboard" => &["true", "false"],
        "hubAutoResumeHours" => &["a number"],
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
        ("spawn", &["a string"][..]),
        ("focus", &["a string"][..]),
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
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) if !dir.is_empty() => expand_home(&dir),
        _ => home_dir().join(".config"),
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
    pub spawn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
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
    /// How many hours after a hub ended a plain `adj hub` resumes it instead of starting a
    /// new one. `0` turns that off, leaving `--resume` as the only way back.
    pub hub_auto_resume_hours: f64,
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
            hub_auto_resume_hours: DEFAULT_HUB_AUTO_RESUME_HOURS,
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
    Settings {
        terminal: TerminalSettings {
            spawn: str_field(&terminal, "spawn"),
            focus: str_field(&terminal, "focus"),
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
        hub_auto_resume_hours: auto_resume_hours(pick("hubAutoResumeHours").as_ref(), warnings),
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
mod tests {
    use super::*;

    /// No flag, always and explicitly. The one that a hub exports is `resolve_config`'s to
    /// read, and a test that picked it up from the terminal would be reporting on the tab it
    /// was run in.
    fn resolve(raw: Value, nwo: &str) -> (Option<Value>, Settings, Vec<String>) {
        let (_, config, settings, warnings) = resolve_from_value(&raw, nwo, None);
        (config, settings, warnings)
    }

    #[test]
    fn an_unregistered_repo_still_gets_settings() {
        let (config, settings, _) = resolve(
            json!({"terminal": {"spawn": "tmux new-window {command}"}, "repos": {}}),
            "acme/widget",
        );
        assert!(config.is_none());
        assert_eq!(
            settings.terminal.spawn.as_deref(),
            Some("tmux new-window {command}")
        );
    }

    #[test]
    fn the_flat_shorthand_becomes_one_source() {
        let (config, _, _) = resolve(
            json!({"repos": {"acme/web": {
                "taskSource": "github",
                "issueRepo": "acme/web",
                "issueKeys": {"acme/web": "WEB"},
                "ide": "code"
            }}}),
            "acme/web",
        );
        let config = config.unwrap();
        let sources = config["taskSources"].as_array().unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0]["type"], "github");
        assert_eq!(sources[0]["issueRepo"], "acme/web");
        // The source keys are lifted, not copied: two places to read `issueRepo` from is how
        // a prompt ends up reading the stale one.
        assert!(config.get("issueRepo").is_none());
        assert!(config.get("taskSource").is_none());
    }

    #[test]
    fn every_source_gets_a_worktree_name_so_two_trackers_cannot_collide() {
        let (config, _, _) = resolve(
            json!({"repos": {"acme/app": {"taskSources": [
                {"type": "github-project", "projectOwner": "acme", "projectNumber": 9}
            ], "issueKeys": {"acme/app": "WID"}, "ide": "studio"}}}),
            "acme/app",
        );
        assert_eq!(
            config.unwrap()["taskSources"][0]["worktreeName"],
            DEFAULT_WORKTREE_NAME
        );
    }

    #[test]
    fn defaults_merge_under_the_entry_but_never_supply_sources() {
        let (config, _, warnings) = resolve(
            json!({
                "defaults": {"selfReviewRounds": 3, "draftPr": true, "taskSource": "github"},
                "repos": {"acme/app": {
                    "selfReviewRounds": 9,
                    "taskSources": [{"type": "github", "issueRepo": "acme/app"}],
                    "issueKeys": {"acme/app": "WID"},
                    "ide": "code"
                }}
            }),
            "acme/app",
        );
        let config = config.unwrap();
        assert_eq!(config["selfReviewRounds"], 9);
        assert_eq!(config["draftPr"], true);
        assert_eq!(config["taskSources"].as_array().unwrap().len(), 1);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("taskSource in defaults"))
        );
    }

    #[test]
    fn builtin_defaults_fill_the_gaps() {
        let (config, _, _) = resolve(
            json!({"repos": {"acme/app": {"taskSource": "github", "issueRepo": "acme/app",
                   "issueKeys": {"acme/app": "WID"}, "ide": "code"}}}),
            "acme/app",
        );
        let config = config.unwrap();
        assert_eq!(config["reviewEffort"], "high");
        assert_eq!(config["selfReviewRounds"], 5);
        assert_eq!(config["baseBranch"], "auto");
    }

    #[test]
    fn the_resolved_settings_are_spelled_the_way_the_config_file_spells_them() {
        let (_, settings, _) = resolve(
            // Every optional key is set, because several are skipped when absent and a
            // skipped key would pass an "is it spelled right" check for free.
            json!({"hubWake": "poke", "workerWake": "poke2", "agentRunner": "run {prompt}",
                   "hubRunner": "start {name}", "worktreePattern": ".wt/{name}",
                   "agentResumeRunner": "again {sessionId}", "hubAutoResumeHours": 1,
                   "hubResumeRunner": "again {name} {sessionId}",
                   "agentEnv": {"K": "v"}, "ide": "code", "startupDashboard": false,
                   "terminal": {"spawn": "s", "focus": "f", "close": "c", "title": "t"},
                   "repos": {}}),
            "acme/app",
        );
        let text = serde_json::to_value(settings).unwrap();
        for key in [
            "hubWake",
            "workerWake",
            "agentRunner",
            "hubRunner",
            "agentResumeRunner",
            "hubResumeRunner",
            "worktreePattern",
            "agentEnv",
            "startupDashboard",
            "hubAutoResumeHours",
        ] {
            assert!(text.get(key).is_some(), "{key} is missing from {text}");
        }
        for key in [
            "hub_wake",
            "worker_wake",
            "agent_runner",
            "hub_runner",
            "agent_resume_runner",
            "hub_resume_runner",
            "hub_auto_resume_hours",
            "worktree_pattern",
            "agent_env",
            "startup_dashboard",
        ] {
            assert!(
                text.get(key).is_none(),
                "{key} is still snake_case in {text}"
            );
        }
    }

    #[test]
    fn per_key_documentation_is_dropped_too() {
        // The schema documents individual keys with a `//<key>` sibling, not only with a
        // bare `//`. Both are prose and neither belongs in what a session reads.
        let (config, _, _) = resolve(
            json!({"repos": {"acme/app": {
                "//": "bare prose",
                "//verify": "prose about verify",
                "verify": ["cargo test"],
                "issueCreate": {"//notes": "prose about notes", "command": "x"},
                "taskSource": "github", "issueRepo": "acme/app",
                "issueKeys": {"acme/app": "WID"}, "ide": "code"
            }}}),
            "acme/app",
        );
        let text = serde_json::to_string(&config.unwrap()).unwrap();
        assert!(!text.contains("prose"), "{text}");
    }

    #[test]
    fn documentation_keys_never_reach_the_reader() {
        let (config, _, _) = resolve(
            json!({"repos": {"acme/app": {
                "//": "prose the session does not need",
                "taskSources": [{"//": "also prose", "type": "github", "issueRepo": "acme/app"}],
                "issueKeys": {"acme/app": "WID"},
                "ide": "code"
            }}}),
            "acme/app",
        );
        let text = serde_json::to_string(&config.unwrap()).unwrap();
        assert!(!text.contains("prose"), "{text}");
    }

    #[test]
    fn a_source_missing_its_required_keys_warns_instead_of_failing() {
        let (config, _, warnings) = resolve(
            json!({"repos": {"acme/app": {"taskSources": [{"type": "jira", "jira": {"project": "ABC"}}],
                   "issueKeys": {"acme/app": "WID"}, "ide": "code"}}}),
            "acme/app",
        );
        assert!(config.is_some());
        assert!(
            warnings.iter().any(|w| w.contains("jira.cloudId")),
            "{warnings:?}"
        );
    }

    #[test]
    fn a_duplicated_issue_key_warns() {
        let (_, _, warnings) = resolve(
            json!({"repos": {"acme/app": {
                "taskSources": [{"type": "github", "issueRepo": "acme/app"}],
                "issueKeys": {"acme/app": "WID", "acme/other": "WID"},
                "ide": "code"
            }}}),
            "acme/app",
        );
        assert!(
            warnings.iter().any(|w| w.contains("used more than once")),
            "{warnings:?}"
        );
    }

    #[test]
    fn an_issue_repo_outside_issue_keys_warns_because_its_issues_would_vanish() {
        let (_, _, warnings) = resolve(
            json!({"repos": {"acme/app": {
                "taskSources": [{"type": "github", "issueRepo": "acme/other"}],
                "issueKeys": {"acme/app": "WID"},
                "ide": "code"
            }}}),
            "acme/app",
        );
        assert!(
            warnings.iter().any(|w| w.contains("acme/other")),
            "{warnings:?}"
        );
    }

    #[test]
    fn a_missing_ide_warns() {
        let (_, settings, warnings) = resolve(
            json!({"repos": {"acme/app": {"taskSource": "github", "issueRepo": "acme/app",
                   "issueKeys": {"acme/app": "WID"}}}}),
            "acme/app",
        );
        assert!(settings.ide.is_none());
        assert!(warnings.iter().any(|w| w.contains("ide")), "{warnings:?}");
    }

    #[test]
    fn the_repo_key_matches_case_insensitively() {
        let (config, _, _) = resolve(
            json!({"repos": {"Acme/App": {"taskSource": "github", "issueRepo": "Acme/App",
                   "issueKeys": {"Acme/App": "WID"}, "ide": "code"}}}),
            "acme/app",
        );
        assert!(config.is_some());
    }

    #[test]
    fn settings_take_the_most_specific_answer() {
        let (_, settings, _) = resolve(
            json!({
                "agentRunner": "codex exec '{prompt}'",
                "notification": {"command": "curl -d {message} https://example.invalid"},
                "defaults": {"agentRunner": "agy run '{prompt}'", "ide": "code"},
                "repos": {"acme/app": {
                    "agentRunner": "claude '{prompt}'",
                    "taskSource": "github", "issueRepo": "acme/app",
                    "issueKeys": {"acme/app": "WID"}, "ide": "studio"
                }}
            }),
            "acme/app",
        );
        assert_eq!(settings.agent_runner.as_deref(), Some("claude '{prompt}'"));
        assert_eq!(settings.ide.as_deref(), Some("studio"));
        assert_eq!(
            settings.notification.template(),
            Some("curl -d {message} https://example.invalid")
        );
    }

    #[test]
    fn a_repo_can_carry_its_own_agent_environment() {
        let (_, settings, _) = resolve(
            json!({"repos": {"acme/app": {"agentEnv": {"CLAUDE_CONFIG_DIR": "/cfg/app"},
                   "taskSource": "github", "issueRepo": "acme/app",
                   "issueKeys": {"acme/app": "WID"}, "ide": "code"}}}),
            "acme/app",
        );
        assert_eq!(
            settings.agent_env,
            vec![("CLAUDE_CONFIG_DIR".to_string(), "/cfg/app".to_string())]
        );
    }

    #[test]
    fn a_hub_collects_the_dashboard_at_startup_until_somebody_says_not_to() {
        // Nothing configured and no flag: the hub collects, which is what every machine that
        // has never heard of this setting has to keep doing.
        assert!(startup_dashboard(None, None));
        assert!(!startup_dashboard(Some(&json!(false)), None));
        assert!(startup_dashboard(Some(&json!(true)), None));
    }

    #[test]
    fn the_flag_a_hub_was_started_under_outranks_what_the_config_says() {
        // `--no-dashboard` against a config that says collect…
        assert!(!startup_dashboard(Some(&json!(true)), Some("0")));
        // …and `--dashboard` against a config that says don't, which is the case the second
        // flag exists for: a standing preference is not a thing you want to edit twice.
        assert!(startup_dashboard(Some(&json!(false)), Some("1")));
        // Anything that is neither leaves the configured answer standing. The variable is
        // inherited by every process a hub starts, so a mistyped export that reversed a
        // setting would follow the person around all day.
        //
        // Both fixtures, on purpose: an implementation that read every value other than
        // `"1"` as off would satisfy the second on its own, and the pair is what tells
        // "falls through" apart from "unknown means no".
        assert!(startup_dashboard(Some(&json!(true)), Some("no")));
        assert!(!startup_dashboard(Some(&json!(false)), Some("no")));
    }

    #[test]
    fn a_repository_can_keep_its_own_hub_from_collecting_at_startup() {
        // Machine level says collect, this repository says don't: the more specific level
        // wins, as it does for every other setting. A repo whose board is enormous is
        // exactly the one that wants this, and it is the only one that should get it.
        //
        // Asked of the level picker rather than of the resolved `bool`, because the resolved
        // one also answers to `ADJUTANT_STARTUP_DASHBOARD` — and a `cargo test` typed in a
        // hub's own tab inherits that. Split this way the two halves are each checkable on
        // their own: which level wins here, and what the flag does to it above.
        let map = |value: Value| value.as_object().cloned().unwrap();
        let none = Map::new();
        let machine_on = map(json!({"startupDashboard": true}));
        let repo_off = map(json!({"startupDashboard": false}));
        assert_eq!(
            pick_level(&repo_off, &none, &machine_on, "startupDashboard"),
            Some(json!(false))
        );
        // And nothing at the specific level leaves the machine's answer standing, or the
        // setting would only ever be writable per repository.
        assert_eq!(
            pick_level(&none, &none, &machine_on, "startupDashboard"),
            Some(json!(true))
        );
        assert_eq!(pick_level(&none, &none, &none, "startupDashboard"), None);
    }

    #[test]
    fn a_startup_dashboard_that_is_not_a_yes_or_no_is_said_out_loud_and_dropped() {
        // `"false"` the string is the shape somebody writes when they are thinking of the
        // command-line settings around it, and it reads as "set" to a `as_bool` that then
        // says `None`. Dropped either way — but dropped in silence is a person who turned
        // the dashboard off and watched it collect anyway.
        let (_, _, warnings) = resolve(
            json!({"startupDashboard": "false", "repos": {}}),
            "acme/app",
        );
        assert!(
            warnings.iter().any(|w| w.contains("startupDashboard")),
            "{warnings:?}"
        );
        // And what it resolves to once dropped, asked of the decision directly so that no
        // ambient variable can answer for it.
        assert!(startup_dashboard(Some(&json!("false")), None));
    }

    #[test]
    fn machine_settings_stay_out_of_the_repo_config() {
        let (config, _, _) = resolve(
            json!({"repos": {"acme/app": {
                "terminal": {"spawn": "x {command}"}, "agentRunner": "y {prompt}",
                "worktreePattern": ".worktrees/{name}", "startupDashboard": false,
                "taskSource": "github", "issueRepo": "acme/app",
                "issueKeys": {"acme/app": "WID"}, "ide": "code"
            }}}),
            "acme/app",
        );
        let config = config.unwrap();
        for key in [
            "terminal",
            "agentRunner",
            "agentEnv",
            "worktreePattern",
            "notification",
            "startupDashboard",
        ] {
            assert!(config.get(key).is_none(), "{key} leaked into config");
        }
    }

    #[test]
    fn waking_says_how_and_what_independently() {
        let (_, settings, _) = resolve(
            json!({
                // How to poke is a property of the terminal…
                "wake": "tmux send-keys -t {tty} {line} Enter",
                // …and what to say is a property of the agent, so a repo running a
                // different one restates only that half.
                "defaults": {"ide": "code"},
                "repos": {"acme/app": {
                    "workerWake": {"line": "check /adj-outbox"},
                    "taskSource": "github", "issueRepo": "acme/app",
                    "issueKeys": {"acme/app": "WID"}
                }}
            }),
            "acme/app",
        );
        assert_eq!(
            settings.hub_wake.hook.template(),
            Some("tmux send-keys -t {tty} {line} Enter")
        );
        assert_eq!(settings.hub_wake.line, None);
        assert_eq!(settings.hub_wake.line_or("default"), "default");

        // The worker inherits the machine's poke and overrides only the sentence.
        assert_eq!(
            settings.worker_wake.hook.template(),
            Some("tmux send-keys -t {tty} {line} Enter")
        );
        assert_eq!(settings.worker_wake.line_or("default"), "check /adj-outbox");
    }

    #[test]
    fn a_repo_can_be_woken_a_different_way_from_the_machine_default() {
        let (_, settings, _) = resolve(
            json!({
                "wake": "wake-tab {tty} {line}",
                "repos": {"acme/app": {
                    "hubWake": {"command": "curl -s -d {line} http://localhost:9/poke",
                                "line": "check the inbox"},
                    "taskSource": "github", "issueRepo": "acme/app",
                    "issueKeys": {"acme/app": "WID"}, "ide": "code"
                }}
            }),
            "acme/app",
        );
        assert_eq!(
            settings.hub_wake.hook.template(),
            Some("curl -s -d {line} http://localhost:9/poke")
        );
        assert_eq!(settings.hub_wake.line_or("x"), "check the inbox");
    }

    #[test]
    fn waking_can_be_turned_off_in_either_form() {
        let (_, a, _) = resolve(json!({"wake": false}), "acme/app");
        assert!(a.hub_wake.hook.is_off());
        let (_, b, _) = resolve(json!({"hubWake": {"command": false}}), "acme/app");
        assert!(b.hub_wake.hook.is_off());
        // …and turning one direction off leaves the other alone.
        let (_, c, _) = resolve(
            json!({"wake": "poke {pid}", "workerWake": false}),
            "acme/app",
        );
        assert!(c.worker_wake.hook.is_off());
        assert_eq!(c.hub_wake.hook.template(), Some("poke {pid}"));
    }

    #[test]
    fn a_notification_given_as_a_bare_string_works_too() {
        let (_, settings, _) = resolve(json!({"notification": "printf '\\a'"}), "acme/app");
        assert_eq!(settings.notification.template(), Some("printf '\\a'"));
    }

    #[test]
    fn a_non_array_task_sources_warns_rather_than_panicking() {
        let (_, _, warnings) = resolve(
            json!({"repos": {"acme/app": {"taskSources": "github", "ide": "code"}}}),
            "acme/app",
        );
        assert!(
            warnings.iter().any(|w| w.contains("not an array")),
            "{warnings:?}"
        );
    }

    /// A registered repo, so the interesting warnings are the only ones in the list.
    fn a_repo(extra: Value) -> Value {
        let mut entry = json!({
            "taskSource": "github", "issueRepo": "acme/app",
            "issueKeys": {"acme/app": "WID"}, "ide": "code"
        });
        let map = entry.as_object_mut().unwrap();
        for (k, v) in extra.as_object().unwrap() {
            map.insert(k.clone(), v.clone());
        }
        json!({
            "hubWake": {"command": "tmux send-keys {line}"},
            "repos": {"acme/app": entry}
        })
    }

    fn warning_about(warnings: &[String], needle: &str) -> String {
        warnings
            .iter()
            .find(|w| w.contains(needle))
            .unwrap_or_else(|| panic!("nothing said about {needle}: {warnings:?}"))
            .clone()
    }

    #[test]
    fn the_long_form_of_notification_can_be_turned_off() {
        // The short form has always been able to say "off". The long form dropped the
        // `false` on the floor and ran the built-in notifier anyway, which is the one
        // answer the person writing it was trying to prevent.
        let (_, settings, _) = resolve(
            a_repo(json!({"notification": {"command": false}})),
            "acme/app",
        );
        assert!(
            settings.notification.is_off(),
            "{:?}",
            settings.notification
        );

        let (_, settings, _) = resolve(
            a_repo(json!({"notification": {"command": "say hi"}})),
            "acme/app",
        );
        assert_eq!(settings.notification.template(), Some("say hi"));
    }

    #[test]
    fn a_config_of_the_wrong_shape_says_so_instead_of_vanishing() {
        // `repos` as an array resolved to "this repository is not registered", which is
        // also what a correct config for a different repository looks like.
        let (config, _, warnings) = resolve(json!({"repos": [{"acme/app": {}}]}), "acme/app");
        assert!(config.is_none());
        assert!(warning_about(&warnings, "repos").contains("an array"));

        let (_, _, warnings) = resolve(json!({"repos": {"acme/app": "github"}}), "acme/app");
        assert!(warning_about(&warnings, "entry").contains("a string"));
    }

    #[test]
    fn a_setting_of_the_wrong_shape_says_so_instead_of_being_dropped() {
        let (_, settings, warnings) = resolve(
            a_repo(json!({"terminal": "iterm", "hubRunner": 5})),
            "acme/app",
        );
        assert!(warning_about(&warnings, "terminal").contains("a string"));
        assert!(warning_about(&warnings, "hubRunner").contains("a number"));
        // Still dropped — a half-understood setting is worse than none. What changed is
        // that the person is told.
        assert_eq!(settings.terminal.spawn, None);
        assert_eq!(settings.hub_runner, None);

        let (_, _, warnings) = resolve(a_repo(json!({"terminal": {"title": 5}})), "acme/app");
        assert!(warning_about(&warnings, "terminal.title").contains("a number"));

        let (_, _, warnings) = resolve(a_repo(json!({"terminal": {"close": 5}})), "acme/app");
        assert!(warning_about(&warnings, "terminal.close").contains("a number"));
    }

    #[test]
    fn closing_a_tab_is_a_template_like_opening_one() {
        // Reaching the struct and being in the shape table are two independent additions: a
        // field the table has never heard of reads a string perfectly well and says nothing
        // at all about the config that put a number there.
        let (_, settings, warnings) = resolve(
            a_repo(json!({"terminal": {"close": "close-tab {tty}"}})),
            "acme/app",
        );
        assert_eq!(settings.terminal.close.template(), Some("close-tab {tty}"));
        assert!(warnings.is_empty(), "{warnings:?}");

        // And off is its own answer rather than a missing one: read as unset, `false`
        // would hand the tab to the built-in closer, which is the opposite of what it says.
        let (_, settings, warnings) =
            resolve(a_repo(json!({"terminal": {"close": false}})), "acme/app");
        assert!(settings.terminal.close.is_off());
        assert_eq!(settings.terminal.close.template(), None);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn an_env_key_that_is_not_a_variable_name_never_reaches_a_command_line() {
        // `env K=V …` is built as a shell line, so a `;` in a key is a command separator in
        // front of every worker this repository ever starts.
        let (_, settings, warnings) = resolve(
            a_repo(json!({"agentEnv": {"OK_ONE": "v", "BAD;touch /tmp/pwned": "v", "N": 5}})),
            "acme/app",
        );
        assert_eq!(
            settings.agent_env,
            vec![("OK_ONE".to_string(), "v".to_string())]
        );
        assert!(warning_about(&warnings, "BAD").contains("not a variable name"));
        assert!(warning_about(&warnings, "agentEnv N").contains("a number"));
    }

    #[test]
    fn a_hub_runner_with_nowhere_to_put_the_name_is_called_out() {
        let (_, _, warnings) =
            resolve(a_repo(json!({"hubRunner": "myagent --resume"})), "acme/app");
        assert!(warning_about(&warnings, "hubRunner").contains("{name}"));
        assert!(warning_about(&warnings, "hubRunner").contains("still runs"));

        let (_, _, warnings) = resolve(
            a_repo(json!({"hubRunner": "myagent -n {name}"})),
            "acme/app",
        );
        assert!(
            !warnings.iter().any(|w| w.contains("hubRunner")),
            "{warnings:?}"
        );
    }

    #[test]
    fn the_auto_resume_window_is_hours_and_falls_back_when_it_cannot_be_one() {
        let (_, settings, _) = resolve(a_repo(json!({})), "acme/app");
        assert_eq!(
            settings.hub_auto_resume_hours,
            DEFAULT_HUB_AUTO_RESUME_HOURS
        );

        let (_, settings, warnings) =
            resolve(a_repo(json!({"hubAutoResumeHours": 0.5})), "acme/app");
        assert_eq!(settings.hub_auto_resume_hours, 0.5);
        assert!(
            !warnings.iter().any(|w| w.contains("hubAutoResumeHours")),
            "{warnings:?}"
        );

        let (_, settings, warnings) = resolve(a_repo(json!({"hubAutoResumeHours": 0})), "acme/app");
        assert_eq!(settings.hub_auto_resume_hours, 0.0);
        assert!(warnings.is_empty() || !warnings.iter().any(|w| w.contains("hubAutoResume")));

        let (_, settings, warnings) =
            resolve(a_repo(json!({"hubAutoResumeHours": -1})), "acme/app");
        assert_eq!(
            settings.hub_auto_resume_hours,
            DEFAULT_HUB_AUTO_RESUME_HOURS
        );
        assert!(warning_about(&warnings, "hubAutoResumeHours").contains("0 or more"));

        let (_, settings, warnings) =
            resolve(a_repo(json!({"hubAutoResumeHours": "3"})), "acme/app");
        assert_eq!(
            settings.hub_auto_resume_hours,
            DEFAULT_HUB_AUTO_RESUME_HOURS
        );
        assert!(warning_about(&warnings, "hubAutoResumeHours").contains("a number"));
    }

    #[test]
    fn a_resume_runner_that_cannot_be_told_the_session_is_called_out() {
        let (_, settings, warnings) = resolve(
            a_repo(json!({"hubResumeRunner": "myagent --continue",
                          "agentResumeRunner": "myagent resume {sessionId}"})),
            "acme/app",
        );
        assert!(warning_about(&warnings, "hubResumeRunner").contains("{sessionId}"));
        assert!(
            !warnings.iter().any(|w| w.contains("agentResumeRunner")),
            "{warnings:?}"
        );
        assert_eq!(
            settings.agent_resume_runner.as_deref(),
            Some("myagent resume {sessionId}")
        );
    }

    #[test]
    fn the_editor_is_answered_once_not_twice() {
        // `ide` describes the machine, like `terminal` and `notification`. Left in the
        // per-repo config as well, `adj config` gave two answers that could disagree.
        let (config, settings, _) = resolve(a_repo(json!({})), "acme/app");
        assert_eq!(settings.ide.as_deref(), Some("code"));
        assert!(config.unwrap().get("ide").is_none());
    }

    #[test]
    fn a_missing_home_still_gives_an_absolute_anchor() {
        // A relative anchor puts the state directory under whatever directory the process
        // started in, so a hub and a worker started from different places end up with
        // different inboxes and neither can see anything wrong.
        for absent in [None, Some(""), Some("relative/home")] {
            let home = home_from(absent);
            assert!(home.is_absolute(), "{absent:?} gave {}", home.display());
        }
        assert_eq!(home_from(Some("/home/x")), PathBuf::from("/home/x"));
    }

    #[test]
    fn one_repo_changing_its_tab_title_keeps_the_machines_terminal() {
        // Picked whole, this entry dropped `spawn` and `focus` for that repository — and
        // the symptom of a missing `spawn` is a tab that never opens.
        let (_, settings, _) = resolve(
            json!({
                "terminal": {"spawn": "tmux new-window -c {cwd} {command}", "focus": "raise {pid}",
                             "close": "close-tab {tty}"},
                "repos": {"acme/app": {
                    "taskSource": "github", "issueRepo": "acme/app",
                    "issueKeys": {"acme/app": "WID"}, "ide": "code",
                    "terminal": {"title": "tmux rename-window {title}"}
                }}
            }),
            "acme/app",
        );
        assert_eq!(
            settings.terminal.spawn.as_deref(),
            Some("tmux new-window -c {cwd} {command}")
        );
        assert_eq!(settings.terminal.focus.as_deref(), Some("raise {pid}"));
        assert_eq!(settings.terminal.close.template(), Some("close-tab {tty}"));
        assert_eq!(
            settings.terminal.title.template(),
            Some("tmux rename-window {title}")
        );
    }

    #[test]
    fn a_repo_changing_only_the_sentence_keeps_the_machines_wake() {
        // The same rule `Wake::resolve` applies between `wake` and `hubWake` has to apply
        // between the levels, or a repository that changes what is *said* silently throws
        // away how this machine *pokes*.
        let (_, settings, _) = resolve(
            a_repo(json!({"hubWake": {"line": "look in your inbox"}})),
            "acme/app",
        );
        assert_eq!(
            settings.hub_wake.hook.template(),
            Some("tmux send-keys {line}")
        );
        assert_eq!(settings.hub_wake.line_or("built-in"), "look in your inbox");

        // A bare value still replaces outright: it names a whole mechanism, and merging it
        // into an object would invent a setting nobody wrote.
        let (_, settings, _) = resolve(a_repo(json!({"hubWake": false})), "acme/app");
        assert!(settings.hub_wake.hook.is_off());
    }

    #[test]
    fn the_sentence_being_the_wrong_type_is_reported_like_everything_else() {
        let (_, settings, warnings) = resolve(a_repo(json!({"hubWake": {"line": 5}})), "acme/app");
        assert!(warning_about(&warnings, "hubWake.line").contains("a number"));
        assert_eq!(settings.hub_wake.line, None);
    }

    #[test]
    fn the_home_fallback_is_absolute_and_not_shared_with_anyone() {
        // One shared directory would put two people's configs and inboxes in one place.
        for absent in [None, Some(""), Some("relative/home")] {
            let home = home_from(absent);
            assert!(home.is_absolute(), "{absent:?} gave {}", home.display());
            assert!(
                home.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("adjutant-no-home-"),
                "{}",
                home.display()
            );
        }
    }
}
