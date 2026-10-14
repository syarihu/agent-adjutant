use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;

use crate::infra::agent::Agent;
use crate::infra::env::{HUB_ENV, STATE_DIR_ENV};
use crate::infra::paths::exe_path_absolute;
use crate::infra::terminal;
use crate::kernel::config::resolve_config;
use crate::kernel::identity::AIDE_CONFIG_KEY;
use crate::kernel::runner::{aide_command, aide_mcp_config};
use crate::lifecycle::agent_command;
use crate::lifecycle::review::reader_env;
use crate::registry::{self, clear_aide_if, note_aide, valid_session_id};

use super::{aide_home, write_aide_settings};

/// Become the aide that runs as `session`: write down this process and its window, then replace
/// this process with the agent. Returns only when it cannot, with what to tell the person.
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
    // Without the file the agent still starts, only with no hooks and no context figure: a
    // missing file must not stop a session that would otherwise run.
    let hooks = match write_aide_settings(root, &exe) {
        Ok(path) => Some(path),
        Err(why) => {
            let _ = writeln!(
                std::io::stderr(),
                "starting the aide without hooks or a context figure: {why}"
            );
            None
        }
    };
    let hooks = hooks.as_ref().map(|path| path.to_string_lossy());
    let env = aide_env(settings.agent_env.clone(), root);
    let command = aide_command(&env, session, &aide_mcp_config(&exe), hooks.as_deref());
    let error = agent_command(&command).exec();
    fail(format!("cannot start the aide: {error}"))
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
