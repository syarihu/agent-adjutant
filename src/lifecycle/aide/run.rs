use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

use crate::infra::agent::Agent;
use crate::infra::env::{CLAUDE_CONFIG_DIR_ENV, HUB_ENV, STATE_DIR_ENV};
use crate::infra::fs::read_json;
use crate::infra::paths::{exe_path_absolute, home_dir};
use crate::infra::terminal;
use crate::kernel::config::resolve_config;
use crate::kernel::identity::AIDE_CONFIG_KEY;
use crate::kernel::runner::{aide_command, aide_mcp_config, names_adjutant_server};
use crate::lifecycle::agent_command;
use crate::lifecycle::review::reader_env;
use crate::registry::{self, clear_aide_if, note_aide, valid_session_id};

use super::{aide_home, write_aide_settings};

/// Become the aide that runs as `session`: write down this process and its window, then replace
/// this process with the agent. Returns only when it cannot, with what to tell the person. The
/// settings file (hooks and status line) is written first, and without it nothing is started.
///
/// Refused when the record no longer names this session (the aide was reset before it got here),
/// so a superseded launch starts no agent. A launch that fails after that takes its own claim
/// back, so the next open does not wait out the start grace. A reset in the gap between the
/// window opening and `note_aide` leaves that superseded window showing its refusal until it is
/// closed (the window is held), which is harmless.
pub fn run_aide(root: &Path, session: &str) -> String {
    if !valid_session_id(session) {
        return format!("not a session id: {session}");
    }
    let settings = match resolve_config(AIDE_CONFIG_KEY) {
        Ok(resolved) => resolved.settings,
        Err(e) => return e,
    };
    let pid = std::process::id();
    let location = terminal::own_location(&settings.terminal);
    if let Err(e) = note_aide(
        root,
        session,
        pid,
        registry::ps_started(pid),
        Some(location),
    ) {
        return e;
    }
    let fail = |why: String| {
        let _ = clear_aide_if(root, session);
        why
    };
    // The aide works in its own directory, whatever this was started from.
    let home = aide_home(root);
    if let Err(e) = std::fs::create_dir_all(&home).and_then(|()| std::env::set_current_dir(&home)) {
        return fail(format!("cannot work in {}: {e}", home.display()));
    }
    let exe = match exe_path_absolute() {
        Ok(exe) => exe,
        Err(e) => return fail(e),
    };
    // Without the file the aide would run with no session row and no context figure, which is a
    // session nobody can see: it is not started, and the window shows why.
    let hooks = match write_aide_settings(root, &exe) {
        Ok(path) => path,
        Err(why) => return fail(format!("cannot write the aide's settings: {why}")),
    };
    let hooks = hooks.to_string_lossy();
    let env = aide_env(settings.agent_env.clone(), root);
    // The person's own entry is used when there is one: a second entry under the same name with
    // another command would show a conflict in `/mcp`. A config that cannot be read names nothing.
    let claude_json = claude_json_path(
        &env,
        std::env::var(CLAUDE_CONFIG_DIR_ENV).ok().as_deref(),
        &home_dir(),
    );
    let named = read_json(&claude_json).is_some_and(|config| {
        let dir = home.to_string_lossy();
        // Claude Code keys local scope by the real path, which differs from `home` where a
        // parent directory is a link (macOS `/var`).
        let real = std::fs::canonicalize(&home).unwrap_or_else(|_| home.clone());
        names_adjutant_server(&config, &dir)
            || names_adjutant_server(&config, &real.to_string_lossy())
    });
    let mcp = (!named).then(|| aide_mcp_config(&exe));
    let command = aide_command(&env, session, mcp.as_deref(), Some(&hooks));
    let error = agent_command(&command).exec();
    fail(format!("cannot start the aide: {error}"))
}

/// Where Claude Code keeps the person's config (`.claude.json`): in `CLAUDE_CONFIG_DIR` when the
/// aide's environment sets it (`agentEnv`), else when this process has it, else in the home
/// directory. `process_dir` and `home` are passed in so no test reads the real ones.
pub(crate) fn claude_json_path(
    env: &[(String, String)],
    process_dir: Option<&str>,
    home: &Path,
) -> PathBuf {
    let given = env
        .iter()
        .rev()
        .find(|(key, _)| key == CLAUDE_CONFIG_DIR_ENV)
        .map(|(_, dir)| dir.as_str())
        .or(process_dir)
        .filter(|dir| !dir.is_empty());
    match given {
        Some(dir) => Path::new(dir).join(".claude.json"),
        None => home.join(".claude.json"),
    }
}

/// The environment the aide's agent gets: the person's `agentEnv`, without a hub (the aide has
/// none, and every tool would then default to it) and with the state directory set to `state`,
/// absolute, because the aide's directory is no checkout to find it from. Claude Code gets its
/// fullscreen UI for the reason the reader does.
pub(crate) fn aide_env(agent_env: Vec<(String, String)>, state: &Path) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = agent_env
        .into_iter()
        .filter(|(key, _)| key != HUB_ENV && key != STATE_DIR_ENV)
        .collect();
    let state = std::path::absolute(state).unwrap_or_else(|_| state.to_path_buf());
    env.push((
        STATE_DIR_ENV.to_string(),
        state.to_string_lossy().to_string(),
    ));
    reader_env(env, Agent::Claude)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    #[test]
    fn the_state_directory_is_absolute_and_a_hub_is_never_named() {
        let given = vec![
            pair("ADJUTANT_HUB", "@review"),
            pair("ADJUTANT_STATE_DIR", "relative"),
            pair("CLAUDE_CONFIG_DIR", "/cfg"),
        ];
        let env = aide_env(given, Path::new("/state"));
        assert!(!env.iter().any(|(k, _)| k == "ADJUTANT_HUB"), "{env:?}");
        assert_eq!(
            env.iter()
                .filter(|(k, _)| k == "ADJUTANT_STATE_DIR")
                .collect::<Vec<_>>(),
            vec![&pair("ADJUTANT_STATE_DIR", "/state")]
        );
        assert!(env.contains(&pair("CLAUDE_CONFIG_DIR", "/cfg")));
        assert_eq!(
            env.iter()
                .filter(|(k, _)| k == "CLAUDE_CODE_NO_FLICKER")
                .count(),
            1
        );
    }

    #[test]
    fn the_claude_config_is_read_from_the_agent_env_first() {
        let home = Path::new("/home/me");
        let env = vec![pair("CLAUDE_CONFIG_DIR", "/cfg")];
        assert_eq!(
            claude_json_path(&env, Some("/p"), home),
            Path::new("/cfg/.claude.json")
        );
        assert_eq!(
            claude_json_path(&[], Some("/p"), home),
            Path::new("/p/.claude.json")
        );
        assert_eq!(
            claude_json_path(&[], None, home),
            Path::new("/home/me/.claude.json")
        );
    }

    #[test]
    fn a_fullscreen_setting_of_the_persons_own_is_kept() {
        let env = aide_env(vec![pair("CLAUDE_CODE_NO_FLICKER", "0")], Path::new("/s"));
        assert_eq!(
            env.iter()
                .filter(|(k, _)| k == "CLAUDE_CODE_NO_FLICKER")
                .collect::<Vec<_>>(),
            vec![&pair("CLAUDE_CODE_NO_FLICKER", "0")]
        );
    }
}
