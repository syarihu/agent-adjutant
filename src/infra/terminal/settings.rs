use serde_json::{Map, Value};

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
    pub(crate) fn read(value: Option<Value>) -> Self {
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
    pub(crate) fn read(value: Option<Value>) -> Self {
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
    pub(crate) fn resolve(base: Option<Value>, specific: Option<Value>) -> Self {
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
