use std::path::Path;

use crate::infra::paths::exe_path;
use crate::infra::template::sh_join;
use crate::infra::terminal::{self, Performed, SpawnRequest};
use crate::kernel::config::Settings;
use crate::lifecycle::{forwarded_env, title_command};

use super::aide_home;

/// What opening a window answers: what was done, and the pane when one can be watched.
type Spawned = Result<(Performed, Option<String>), String>;

/// The title of the aide's window.
const TITLE: &str = "adj-aide";

/// Open a window that runs the aide as `session`, in its own directory under `root`. `Err` when
/// the window could not be opened or closed at once, with what it printed.
pub fn launch_aide(root: &Path, settings: &Settings, session: &str) -> Result<Performed, String> {
    launch_aide_with(
        root,
        settings,
        session,
        &|req| terminal::spawn_held(&settings.terminal, req, false),
        &|pane| terminal::tmux_watch_start(settings.terminal.tmux_socket(), pane),
    )
}

/// `launch_aide`, with the way a window is opened handed in.
pub(crate) fn launch_aide_with(
    root: &Path,
    settings: &Settings,
    session: &str,
    spawn: &dyn Fn(&SpawnRequest) -> Spawned,
    watch: &dyn Fn(&str) -> Result<terminal::StartWatch, String>,
) -> Result<Performed, String> {
    let home = aide_home(root);
    std::fs::create_dir_all(&home).map_err(|e| format!("cannot create {}: {e}", home.display()))?;
    let mut parts = forwarded_env(root);
    parts.extend([
        exe_path(),
        "aide".to_string(),
        "run".to_string(),
        "--session".to_string(),
        session.to_string(),
    ]);
    let name_it = title_command(settings, TITLE);
    let cwd = home.to_string_lossy();
    let (performed, pane) = spawn(&SpawnRequest {
        cwd: &cwd,
        title: TITLE,
        command: &sh_join(&parts),
        title_command: name_it.as_deref(),
    })?;
    // An aide that cannot start exits at once and its window was kept to read what it printed.
    let Some(pane) = pane else {
        return Ok(performed);
    };
    match watch(&pane) {
        Ok(terminal::StartWatch::Alive) => Ok(performed),
        // tmux could not be asked: the aide may well be running, so this is not a failed start
        // (the claim must stay, or the next open starts a second one).
        Err(why) => Ok(Performed {
            description: format!(
                "{}; it could not be confirmed that the aide started: {why}",
                performed.description
            ),
            ..performed
        }),
        Ok(terminal::StartWatch::Exited { status, screen }) => {
            let status = status.map(|s| format!(" (status {s})")).unwrap_or_default();
            Err(match screen.as_deref() {
                Some(screen) if !screen.is_empty() => {
                    format!("the aide exited at once{status}: {screen}")
                }
                _ => format!("the aide exited at once{status} without a message"),
            })
        }
        Ok(terminal::StartWatch::Gone) => {
            Err("the aide's window closed at once, before it could be read".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done() -> Performed {
        Performed {
            description: "opened".to_string(),
            script: String::new(),
            ran: true,
            screen: false,
        }
    }

    #[test]
    fn the_window_runs_the_hidden_command_in_the_aides_own_directory() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::default();
        let seen = std::cell::RefCell::new(None);
        let performed = launch_aide_with(
            dir.path(),
            &settings,
            "s-1",
            &|req| {
                *seen.borrow_mut() = Some((
                    req.cwd.to_string(),
                    req.title.to_string(),
                    req.command.to_string(),
                ));
                Ok((done(), None))
            },
            &|_| panic!("no pane to watch"),
        )
        .unwrap();
        assert_eq!(performed.description, "opened");
        let (cwd, title, command) = seen.borrow().clone().unwrap();
        assert_eq!(cwd, dir.path().join("aide").to_string_lossy());
        assert!(dir.path().join("aide").is_dir());
        assert_eq!(title, "adj-aide");
        assert!(command.contains("aide run --session s-1"), "{command}");
    }

    #[test]
    fn a_window_that_could_not_be_opened_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = launch_aide_with(
            dir.path(),
            &Settings::default(),
            "s",
            &|_| Err("no tmux".to_string()),
            &|_| panic!("no pane to watch"),
        )
        .unwrap_err();
        assert_eq!(err, "no tmux");
    }

    #[test]
    fn a_watch_that_cannot_be_asked_is_not_a_failed_start() {
        let dir = tempfile::tempdir().unwrap();
        let performed = launch_aide_with(
            dir.path(),
            &Settings::default(),
            "s",
            &|_| Ok((done(), Some("%1".to_string()))),
            &|_| Err("tmux did not answer".to_string()),
        )
        .unwrap();
        assert!(performed.description.contains("could not be confirmed"));
        assert!(performed.description.contains("tmux did not answer"));
    }

    #[test]
    fn an_aide_that_exited_or_whose_window_closed_is_a_failed_start() {
        let dir = tempfile::tempdir().unwrap();
        let start = |watch: terminal::StartWatch| {
            launch_aide_with(
                dir.path(),
                &Settings::default(),
                "s",
                &|_| Ok((done(), Some("%1".to_string()))),
                &move |_| Ok(watch.clone()),
            )
        };
        assert!(start(terminal::StartWatch::Gone).is_err());
        let exited = terminal::StartWatch::Exited {
            status: Some(1),
            screen: Some("boom".to_string()),
        };
        assert!(start(exited).unwrap_err().contains("boom"));
    }
}
