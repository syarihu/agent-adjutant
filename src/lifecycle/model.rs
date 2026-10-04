use std::time::Duration;

use crate::infra::paths::exe_path;
use crate::kernel::config::Settings;
use crate::kernel::runner;
use crate::registry;

// ── spawn / focus / close / work / ide ───────────────────────────────

/// The command that names a new tab from inside it, if one should.
///
/// A tab names itself rather than being named by the terminal's API, so that whatever
/// `terminal.title` is set to governs every tab the same way. `terminal::spawn` decides
/// whether it can be used at all: only a terminal taking a shell line can run it.
pub fn title_command(settings: &Settings, title: &str) -> Option<String> {
    if settings.terminal.title.is_off() || title.trim().is_empty() {
        return None;
    }
    // stdin closed: `adjutant title` reads a title of `-` from stdin, and in a new tab stdin is
    // the terminal — a task titled `-` would sit there waiting for input, and the worker after
    // it would never start.
    Some(format!(
        "{} < /dev/null",
        crate::infra::template::sh_join(&[
            exe_path(),
            "title".to_string(),
            // One word, `--title=…`: a title starting with `--` given as the next word is read
            // by clap as an option of its own, and `--help` would print help instead.
            format!("--title={title}"),
        ])
    ))
}

/// Open a tab and start a worker agent in it. One command rather than two so the runner
/// template is read in exactly one place.
/// The environment a new tab has to be handed on its command line, because a terminal is
/// given a command line and nothing else.
///
/// `ADJUTANT_HUB` already travels as a flag, and `ADJUTANT_STARTUP_DASHBOARD` as one too.
/// `ADJUTANT_CONFIG`, `XDG_CONFIG_HOME` (which picks the config when `ADJUTANT_CONFIG` is
/// unset) and `ADJUTANT_STATE_DIR` have none, and losing them does not fail — it *splits*:
/// the tab reads the default config and the default state directory, so the agent it starts
/// registers in one world while the hub that dispatched it waits in another. The worker reports into an
/// inbox nobody is reading, and both halves look healthy from where they stand.
///
/// Forwarded only when this process was given them. A machine that never sets them gets the
/// command line it always had. The state directory is sent as `root`, the absolute one this
/// command read, because the tab would otherwise read a relative one against its own worktree.
pub fn forwarded_env(root: &std::path::Path) -> Vec<String> {
    let set: Vec<String> = [
        crate::infra::env::CONFIG_ENV,
        crate::infra::env::XDG_CONFIG_HOME_ENV,
        crate::infra::env::STATE_DIR_ENV,
        crate::infra::env::TMUX_SOCKET_ENV,
        crate::infra::env::TMUX_SESSION_ENV,
    ]
    .iter()
    .filter_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.is_empty())
            .map(|value| {
                if *name == crate::infra::env::STATE_DIR_ENV {
                    format!("{name}={}", root.display())
                } else {
                    format!("{name}={value}")
                }
            })
    })
    .collect();
    if set.is_empty() {
        return Vec::new();
    }
    let mut parts = vec!["env".to_string()];
    parts.extend(set);
    parts
}

/// The shell the agent is started through, by `hub` and `worker` alike.
///
/// The agent gets the caller's environment, minus what would make it answer for something it
/// is not. The adjutant variables are set on the command line itself when they apply, so an
/// inherited one could only name another hub. The git variables in
/// `crate::infra::git::REPOSITORY_LOCATION_ENV` would point every git command the agent runs at another
/// repository, while adjutant — which ignores them — works on the one it was started in.
pub fn agent_command(command: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .env_remove(crate::infra::env::HUB_SESSION_ENV)
        .env_remove(crate::infra::env::HUB_SERVE_ENV)
        .env_remove(crate::infra::env::HUB_ENV);
    for name in crate::infra::git::REPOSITORY_LOCATION_ENV {
        cmd.env_remove(name);
    }
    cmd
}

/// The resume template to use, refusing one that has nowhere to put the session id.
///
/// Without `{sessionId}` the agent is not told which conversation to reopen, and opens
/// whichever one it would pick on its own — which for a hub in the main checkout is as likely
/// to be somebody's unrelated work. Refusing is the one answer that cannot be that.
pub fn resume_template<'a>(
    configured: Option<&'a str>,
    key: &str,
) -> Result<Option<&'a str>, String> {
    match configured {
        Some(template) if !runner::records_session(Some(template), "") => Err(format!(
            "{key} has no {{sessionId}}, so it cannot be told which session to reopen"
        )),
        other => Ok(other),
    }
}

/// How long to wait for a closed tab's worker to actually be gone, and how often to look.
///
/// Closing a tab hangs its session up and the process in it then unwinds, which is not
/// instant — a single look straight afterwards would call a live worker gone. How long the
/// unwinding takes is the agent's business: a Claude Code session routinely needs more than
/// two seconds, and a budget shorter than that reports a worker whose tab is already closed
/// as still there. The budget only costs anything when the process really does stay, and a
/// terminal waiting for someone to confirm the close does not answer sooner for being
/// given less time, so it is set well past how long an agent takes rather than close to it.
/// Polling keeps the common case — gone at the first few looks — as quick as before.
pub const GONE_BUDGET: Duration = Duration::from_secs(10);
pub const GONE_POLL: Duration = Duration::from_millis(100);

/// Wait for a worker to be gone, and answer with what was actually seen.
///
/// Polled rather than slept through: a process that has already exited by the first look is
/// the common case, and a cleanup step that always cost the whole budget is a step people
/// stop running. `look` and `wait` are handed in for the reason `terminal::close_with` takes
/// its runner — this decision has to be testable without spending the budget in real time.
pub fn settled(
    mut look: impl FnMut() -> registry::Liveness,
    mut wait: impl FnMut(Duration),
    budget: Duration,
    poll: Duration,
) -> registry::Liveness {
    // One look before any waiting, then one more per interval until the budget is spent.
    // Counted rather than accumulated, so that a zero interval cannot spin here forever.
    let looks = 1 + budget.as_millis() / poll.as_millis().max(1);
    let mut answer = look();
    for _ in 1..looks {
        if answer == registry::Liveness::Gone {
            return answer;
        }
        wait(poll);
        answer = look();
    }
    answer
}
