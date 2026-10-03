use super::*;
use serde_json::{Map, Value, json};

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
pub(super) fn pick_level(
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
pub(super) fn startup_dashboard(configured: Option<&Value>, flag: Option<&str>) -> bool {
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
