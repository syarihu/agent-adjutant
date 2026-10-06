//! `adj setup claude`: the hooks in the user's Claude Code settings.
//!
//! Every run here names its own settings directory, through `CLAUDE_CONFIG_DIR` (or, for the
//! fallback, `HOME`). The binary would otherwise edit the developer's real `~/.claude`.

mod common;
use common::*;

use serde_json::{Value, json};

/// `CLAUDE_EVENTS.len()`; a unit test in `kernel::agent_hooks` pins it.
const EVENTS: usize = 11;

struct Setup {
    fx: Fixture,
    dir: PathBuf,
}

impl Setup {
    fn new() -> Self {
        let fx = Fixture::new(QUIET);
        let dir = fx._dir.path().join("claude-config");
        Setup { fx, dir }
    }

    fn file(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = self.fx.command(args);
        command.env("CLAUDE_CONFIG_DIR", &self.dir);
        command
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        self.command(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn settings(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.file()).unwrap()).unwrap()
    }

    fn write(&self, text: &str) {
        std::fs::create_dir_all(&self.dir).unwrap();
        std::fs::write(self.file(), text).unwrap();
    }

    fn leftovers(&self) -> Vec<String> {
        std::fs::read_dir(&self.dir)
            .map(|entries| {
                entries
                    .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
                    .filter(|name| name != "settings.json")
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// `sh_quote` (src/infra/template.rs): a path of safe characters as it is, anything else in
/// single quotes. The library does not export it, so the rule is written out here.
fn quoted(path: &str) -> String {
    let safe = !path.is_empty()
        && !path.starts_with('=')
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c));
    match safe {
        true => path.to_string(),
        false => shell_quoted(path),
    }
}

/// What `adj setup claude` writes for `exe`.
fn global_command(exe: &str) -> String {
    let q = quoted(exe);
    format!("[ ! -x {q} ] || {q} hook claude --global || true")
}

fn ours(settings: &Value, command: &str) -> usize {
    settings["hooks"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|groups| groups.as_array().unwrap())
        .flat_map(|group| group["hooks"].as_array().unwrap())
        .filter(|handler| handler["command"] == json!(command))
        .count()
}

const USER: &str = r#"{
  "permissions": { "allow": ["Bash(ls)"] },
  "hooks": {
    "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }]
  }
}
"#;

#[test]
fn it_creates_the_file_and_directory_with_every_event() {
    let s = Setup::new();
    let out = s.ok(&["setup", "claude"]);
    assert!(out.contains("Added adjutant's hooks to"), "{out}");
    assert!(out.contains(&format!("{EVENTS} events run {BIN} hook claude --global")));
    let settings = s.settings();
    let command = global_command(BIN);
    assert_eq!(settings["hooks"].as_object().unwrap().len(), EVENTS);
    assert_eq!(ours(&settings, &command), EVENTS);
    assert_eq!(settings["hooks"]["PostToolUse"][0]["matcher"], "*");
    assert!(settings["hooks"]["Stop"][0].get("matcher").is_none());
    assert!(s.leftovers().is_empty());
}

#[test]
fn it_appends_next_to_user_hooks_and_a_second_run_changes_nothing() {
    let s = Setup::new();
    s.write(USER);
    s.ok(&["setup", "claude"]);
    let settings = s.settings();
    assert_eq!(settings["permissions"], json!({ "allow": ["Bash(ls)"] }));
    let stop = settings["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop.len(), 2);
    assert_eq!(stop[0]["hooks"][0]["command"], "say done");
    assert_eq!(ours(&settings, &global_command(BIN)), EVENTS);

    let before = std::fs::read(s.file()).unwrap();
    let out = s.ok(&["setup", "claude"]);
    assert!(out.contains("nothing to change"), "{out}");
    assert_eq!(std::fs::read(s.file()).unwrap(), before);
}

#[test]
fn it_rewrites_entries_that_name_a_binary_that_moved() {
    let s = Setup::new();
    s.write(USER);
    let old = global_command("/gone/a b/adj");
    let mut settings: Value = serde_json::from_str(USER).unwrap();
    for event in ["Stop", "SessionStart"] {
        let group = json!({ "hooks": [{ "type": "command", "command": old }] });
        match settings["hooks"]
            .get_mut(event)
            .and_then(Value::as_array_mut)
        {
            Some(groups) => groups.push(group),
            None => settings["hooks"][event] = json!([group]),
        }
    }
    s.write(&settings.to_string());
    let out = s.ok(&["setup", "claude"]);
    assert!(out.contains("Updated adjutant's hooks"), "{out}");
    let settings = s.settings();
    assert_eq!(ours(&settings, &old), 0);
    assert_eq!(ours(&settings, &global_command(BIN)), EVENTS);
    assert_eq!(settings["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert_eq!(
        settings["hooks"]["Stop"][0]["hooks"][0]["command"],
        "say done"
    );
}

#[test]
fn remove_takes_only_adjutants_entries_and_then_has_nothing_to_do() {
    let s = Setup::new();
    s.write(USER);
    s.ok(&["setup", "claude"]);
    let out = s.ok(&["setup", "claude", "--remove"]);
    assert!(
        out.contains(&format!("Removed {EVENTS} adjutant hook entries from")),
        "{out}"
    );
    assert_eq!(s.settings(), serde_json::from_str::<Value>(USER).unwrap());
    let out = s.ok(&["setup", "claude", "--remove"]);
    assert!(out.contains("nothing to remove"), "{out}");
}

#[test]
fn remove_without_a_file_creates_nothing() {
    let s = Setup::new();
    let out = s.ok(&["setup", "claude", "--remove"]);
    assert!(out.contains("nothing to remove"), "{out}");
    assert!(!s.dir.exists());
}

#[test]
fn a_file_that_is_not_settings_is_refused_and_left_as_it_was() {
    for text in ["this is not json", "", "[]", r#"{"hooks": []}"#] {
        for args in [
            &["setup", "claude"][..],
            &["setup", "claude", "--remove"][..],
        ] {
            let s = Setup::new();
            s.write(text);
            let out = s.run(args);
            assert_eq!(out.status.code(), Some(1), "{text:?} {args:?}");
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(stderr.contains("settings.json"), "{stderr}");
            assert!(stderr.contains("nothing was written"), "{stderr}");
            assert_eq!(std::fs::read_to_string(s.file()).unwrap(), text);
            assert!(s.leftovers().is_empty());
        }
    }
}

#[test]
fn without_a_config_dir_the_settings_are_under_home() {
    let s = Setup::new();
    let home = s.fx._dir.path().join("home");
    let mut command = s.fx.command(["setup", "claude"]);
    command.env("HOME", &home).env_remove("CLAUDE_CONFIG_DIR");
    assert!(command.output().unwrap().status.success());
    let file = home.join(".claude").join("settings.json");
    let settings: Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    assert_eq!(ours(&settings, &global_command(BIN)), EVENTS);

    let blank = s.fx._dir.path().join("home2");
    let mut command = s.fx.command(["setup", "claude"]);
    command.env("HOME", &blank).env("CLAUDE_CONFIG_DIR", "  ");
    assert!(command.output().unwrap().status.success());
    assert!(blank.join(".claude").join("settings.json").exists());
}

#[cfg(unix)]
#[test]
fn a_symlinked_settings_file_stays_a_link_and_its_target_keeps_its_mode() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let s = Setup::new();
    let target = s.fx._dir.path().join("dotfiles").join("settings.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, USER).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::create_dir_all(&s.dir).unwrap();
    symlink(&target, s.file()).unwrap();

    s.ok(&["setup", "claude"]);
    assert!(std::fs::symlink_metadata(s.file()).unwrap().is_symlink());
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(ours(&written, &global_command(BIN)), EVENTS);
    let mode = std::fs::metadata(&target).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[cfg(unix)]
#[test]
fn a_written_command_runs_and_its_session_shows_up() {
    let s = Setup::new();
    s.ok(&["setup", "claude"]);
    let command = s.settings()["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_string();
    let mut payload: Value = serde_json::from_str(include_str!(
        "../src/fixtures/hooks/claude/session-start.json"
    ))
    .unwrap();
    payload["cwd"] = s.fx.repo.to_string_lossy().to_string().into();
    let mut child = Command::new("sh")
        .args(["-c", &command])
        .hermetic()
        .env("ADJUTANT_STATE_DIR", &s.fx.state)
        .env("CLAUDE_PID", std::process::id().to_string())
        .current_dir(&s.fx.repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let rows = s.fx.json(&["agent-sessions", "--json"]);
    assert_eq!(rows.as_array().unwrap().len(), 1, "{rows}");
    assert_eq!(rows[0]["agent"], "claude");
}

#[test]
fn a_missing_or_unknown_agent_is_a_usage_error_and_creates_nothing() {
    let s = Setup::new();
    for args in [&["setup"][..], &["setup", "codex"][..]] {
        let out = s.run(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
    assert!(!s.dir.exists());
}

#[test]
fn help_lists_setup_and_still_hides_hook() {
    let s = Setup::new();
    let help = s.ok(&["--help"]);
    assert!(help.lines().any(|l| l.trim_start().starts_with("setup ")));
    assert!(!help.lines().any(|l| l.trim_start().starts_with("hook ")));
}
