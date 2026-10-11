use std::path::Path;
use std::time::Duration;

use crate::infra::agent::Agent;
use crate::infra::shell::run_shell;
use crate::infra::terminal::{
    PaneScreen, look_at_tmux_pane, tmux_key_script, tmux_kill_pane_script,
};
use crate::kernel::runner;
use crate::lifecycle::review::{FailError, fail};
use crate::mail::{PaneState, last_output_line, pane_state, trust_dialog, wake_agent};
use crate::others;
use crate::registry;
use crate::task::PrRef;

/// How often the reader's screen is looked at while it settles.
pub(crate) const SETTLE_POLL: Duration = Duration::from_millis(500);

/// How long the reader's agent has to get past its first screen. Its folder-trust question
/// comes up within a second or two of the start, and a screen that is still not an idle prompt
/// or a running turn after this is not one adj knows how to get past.
pub(crate) const SETTLE_DEADLINE: Duration = Duration::from_secs(20);

/// Polls the wait for the dialog to go away after it was answered lasts (about 5 s): the agent
/// redraws within a few frames, and a dialog still up after that was not accepted.
const ANSWER_GRACE_POLLS: u32 = 10;

/// Looks in a row that find no pane before it is taken to be gone: one capture that fails is not
/// the pane closing, and giving up on it would leave the reader at its dialog.
const GONE_LOOKS: u32 = 3;

/// Looks after a `Down` for the pointer to be on "Yes", one poll apart.
const POINTER_LOOKS: u32 = 3;

const NOT_POINTED: &str = "the reader's agent asks whether to trust its workspace, and the pointer could not be put on \"Yes\"; it was not answered";

/// Watch the reader of `pr` in `pane` through its first seconds (`adj review settle`, which
/// `run_reader` starts detached just before it becomes the agent, so nothing that starts readers
/// waits on it). Its agent asks whether to trust the reader's directory, and nobody is there to
/// answer: this answers it, for the reader's own directory and no other, and fails the read
/// with the reason when the agent is stopped at anything else, instead of leaving it `running`
/// for good. A read that is failed here has its pane closed, so no agent is left waiting, and
/// the next reader in line is started.
///
/// `socket` is the tmux server the reader's pane is on, as the reader found it in `$TMUX`; the
/// settings' own socket only stands in when it is not given.
///
/// `Err` only when the watcher itself could not do its part.
pub fn settle_reader(
    root: &Path,
    pr: &PrRef,
    session: &str,
    pane: &str,
    socket: Option<&str>,
) -> Result<(), String> {
    let ctx = registry::review_context(root)?;
    let template = ctx.settings.reviewer_runner.as_deref();
    let agent = wake_agent(Some(template.unwrap_or(runner::DEFAULT_REVIEWER_RUNNER)));
    let socket = socket.or_else(|| ctx.settings.terminal.tmux_socket());
    let polls = (SETTLE_DEADLINE.as_millis() / SETTLE_POLL.as_millis()) as u32;
    let settled = settle_reader_with(
        || look_at_tmux_pane(socket, pane),
        |key| run_shell(&tmux_key_script(socket, pane, key)).map(|_| ()),
        std::thread::sleep,
        agent,
        &others::reader_dir(root, pr),
        polls,
    );
    let Err(why) = settled else {
        return Ok(());
    };
    // Only a read still held by this reader is failed, and only then is its pane closed: a read
    // that was asked for again belongs to a newer reader whose pane this is not.
    match fail(root, pr, Some(session), &why) {
        // `fail` has started the next reader by now, and the watcher ignores the hang-up that
        // closing the pane sends it.
        Ok(_) => {
            let _ = run_shell(&tmux_kill_pane_script(socket, pane));
            Ok(())
        }
        Err(FailError::NotCurrent) => Ok(()),
        Err(FailError::Failed(e)) => Err(e),
    }
}

/// `settle_reader` with the screen, the keys and the waiting handed in. `Ok` when the agent is
/// at its prompt or running (or its pane is gone, which the queue's reaping of dead readers
/// already handles; or its screen is still blank at the deadline), `Err` with why the read should be failed otherwise.
///
/// The trust dialog is answered for `own_dir` alone, by exact match (the directory as given,
/// as it resolves, and a `~/` written out), and at most once: the pointer is put on "Yes, I
/// trust this folder" and read back before Enter, because a key sent at a screen that moved
/// would choose something else.
pub(crate) fn settle_reader_with(
    mut look: impl FnMut() -> Option<PaneScreen>,
    mut send_key: impl FnMut(&str) -> Result<(), String>,
    mut pause: impl FnMut(Duration),
    agent: Agent,
    own_dir: &Path,
    polls: u32,
) -> Result<(), String> {
    if agent == Agent::Generic {
        return Ok(());
    }
    let mut send = |key: &str| {
        send_key(key).map_err(|e| format!("could not send a key to the reader's pane: {e}"))
    };
    let own = own_dirs(own_dir);
    let mut answered = false;
    let mut down_sent = false;
    let mut since_answer = 0;
    let mut last: Option<PaneScreen> = None;
    let mut gone = 0;
    let mut copying = false;
    for _ in 0..polls {
        copying = false;
        let Some(screen) = look() else {
            gone += 1;
            if gone >= GONE_LOOKS {
                return Ok(());
            }
            pause(SETTLE_POLL);
            continue;
        };
        gone = 0;
        // In copy mode a key goes to tmux and not to the agent, so nothing is sent until it ends.
        if screen.in_mode {
            copying = true;
            last = Some(screen);
            pause(SETTLE_POLL);
            continue;
        }
        if let Some(dialog) = trust_dialog(agent, &screen) {
            if answered {
                since_answer += 1;
                if since_answer >= ANSWER_GRACE_POLLS {
                    return Err(
                        "the reader's agent still asks whether to trust its workspace after it was answered".to_string(),
                    );
                }
            } else if !is_own(&dialog.dir, &own) {
                return Err(format!(
                    "the reader's agent asks to trust {}, which is not its own workspace; it was not answered",
                    dialog.dir
                ));
            } else {
                let mut pointed = dialog.yes_pointed;
                if !pointed {
                    // Down is sent once. A pointer still on "No" after it, whether in the looks
                    // below or in a later one, is not something another key can be trusted to fix.
                    if down_sent {
                        return Err(NOT_POINTED.to_string());
                    }
                    send("Down")?;
                    down_sent = true;
                    // The agent may not have redrawn yet, so the pointer is waited for a few looks.
                    let mut seen = false;
                    for _ in 0..POINTER_LOOKS {
                        pause(SETTLE_POLL);
                        if let Some(moved) = look() {
                            seen = true;
                            if trust_dialog(agent, &moved).is_some_and(|d| d.yes_pointed) {
                                pointed = true;
                                break;
                            }
                        }
                    }
                    if !pointed {
                        if seen {
                            return Err(NOT_POINTED.to_string());
                        }
                        // Nothing could be looked at: the dialog is read afresh on the next look.
                        pause(SETTLE_POLL);
                        continue;
                    }
                }
                send("Enter")?;
                answered = true;
            }
        } else {
            match pane_state(agent, &screen) {
                PaneState::Idle | PaneState::Working => return Ok(()),
                _ => {}
            }
        }
        last = Some(screen);
        pause(SETTLE_POLL);
    }
    // Nothing drawn yet is an agent that is slow to start, not a prompt to fail a healthy reader
    // over. Nothing watches a live reader that draws nothing from here on; what exists is that a
    // new request takes over a reader running longer than `HUNG_READER_SECS`.
    // The screen of a pane in copy mode was never read, so nothing is known against it either.
    if copying || last.as_ref().is_none_or(|s| s.text.trim().is_empty()) {
        return Ok(());
    }
    let line = last
        .and_then(|screen| last_output_line(agent, &screen))
        .map(|line| format!(": {line}"))
        .unwrap_or_default();
    Err(format!(
        "the reader is waiting at a prompt adj does not know{line}"
    ))
}

/// The spellings of the reader's own directory a dialog may name it by.
fn own_dirs(own_dir: &Path) -> Vec<String> {
    let mut own = vec![own_dir.to_string_lossy().to_string()];
    if let Ok(resolved) = std::fs::canonicalize(own_dir) {
        own.push(resolved.to_string_lossy().to_string());
    }
    own
}

/// Whether `dir`, as a dialog draws it, is one of `own`: whole, never a prefix, so a directory
/// that merely contains or starts like the reader's is not it.
fn is_own(dir: &str, own: &[String]) -> bool {
    // A path wrapped at a space has lost it, so whitespace is not compared at all.
    let squeeze = |s: &str| s.split_whitespace().collect::<String>();
    let written_out = dir
        .strip_prefix("~/")
        .zip(std::env::var("HOME").ok().filter(|h| !h.is_empty()))
        .map(|(rest, home)| format!("{}/{rest}", home.trim_end_matches('/')));
    let dir = squeeze(dir);
    let dir = dir.trim_end_matches('/');
    own.iter().any(|o| {
        let o = squeeze(o);
        let o = o.trim_end_matches('/');
        o == dir
            || written_out
                .as_deref()
                .is_some_and(|w| squeeze(w).trim_end_matches('/') == o)
    })
}

/// The shell line that starts `adj review settle` for a reader, detached: the shell returns at
/// once and the watcher goes on in the background, so it is nobody's child to be left a zombie
/// when the reader becomes its agent. It stays in the pane's session, so closing the pane would
/// hang it up before it had started the next reader; it ignores `HUP`, which the exec keeps.
pub(crate) fn watcher_command(
    exe: &str,
    id: &str,
    session: &str,
    pane: &str,
    socket: Option<&str>,
) -> String {
    let mut parts = vec![
        exe,
        "review",
        "settle",
        "--pr",
        id,
        "--session",
        session,
        "--pane",
        pane,
    ];
    if let Some(socket) = socket {
        parts.extend(["--socket", socket]);
    }
    format!(
        "trap '' HUP; {} </dev/null >/dev/null 2>&1 &",
        crate::infra::template::sh_join(&parts)
    )
}

/// Start the watcher of the reader of `pr` in `pane`, on tmux server `socket`. Failing to start
/// it is not the reader's to stop for: the reader goes on, as it did before there was a watcher.
pub(crate) fn start_watcher(pr: &PrRef, session: &str, pane: &str, socket: Option<&str>) {
    let line = watcher_command(
        &crate::infra::paths::exe_path(),
        &others::pr_id(pr),
        session,
        pane,
        socket,
    );
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(line)
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::terminal::parse_pane_screen;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    const OWN: &str = "/state/review/o~r~1";

    fn claude_menu(pointed: bool, dir: &str) -> PaneScreen {
        let text =
            include_str!("../../fixtures/panes/claude-menu.txt").replace("/home/user/project", dir);
        let text = if pointed {
            text.replace(
                " ❯ No, exit\n   Yes, I trust",
                "   No, exit\n ❯ Yes, I trust",
            )
        } else {
            text
        };
        parse_pane_screen(&text)
    }

    fn claude_idle() -> PaneScreen {
        parse_pane_screen(include_str!("../../fixtures/panes/claude-idle.txt"))
    }

    /// Runs the settle loop over `looks` (the last one repeats) and returns the answer and the
    /// keys it sent.
    fn run(agent: Agent, looks: Vec<PaneScreen>) -> (Result<(), String>, Vec<String>) {
        let looks = RefCell::new(VecDeque::from(looks));
        let sent = RefCell::new(Vec::new());
        let out = settle_reader_with(
            || {
                let mut q = looks.borrow_mut();
                if q.len() > 1 {
                    q.pop_front()
                } else {
                    q.front().cloned()
                }
            },
            |key| {
                sent.borrow_mut().push(key.to_string());
                Ok(())
            },
            |_| {},
            agent,
            Path::new(OWN),
            40,
        );
        (out, sent.into_inner())
    }

    #[test]
    fn the_claude_dialog_gets_down_then_enter_for_its_own_directory() {
        let (out, sent) = run(
            Agent::Claude,
            vec![
                claude_menu(false, OWN),
                claude_menu(true, OWN),
                claude_idle(),
            ],
        );
        assert_eq!(out, Ok(()));
        assert_eq!(sent, ["Down", "Enter"]);
    }

    #[test]
    fn a_dialog_already_on_yes_gets_only_enter() {
        let agy = |text: &str| parse_pane_screen(&text.replace("/home/user/project", OWN));
        let (out, sent) = run(
            Agent::Agy,
            vec![
                agy(include_str!("../../fixtures/panes/agy-menu.txt")),
                parse_pane_screen(include_str!("../../fixtures/panes/agy-idle.txt")),
            ],
        );
        assert_eq!(out, Ok(()));
        assert_eq!(sent, ["Enter"]);
    }

    #[test]
    fn another_directory_is_named_and_nothing_is_sent() {
        for dir in [
            "/home/user/project",
            "/state/review/o~r~1/sub",
            "/state/review/o~r~10",
        ] {
            let (out, sent) = run(Agent::Claude, vec![claude_menu(false, dir)]);
            let why = out.unwrap_err();
            assert!(
                why.contains(dir) && why.contains("not its own workspace"),
                "{why}"
            );
            assert!(sent.is_empty(), "{dir}");
        }
    }

    #[test]
    fn a_pointer_that_does_not_move_is_not_answered() {
        let (out, sent) = run(Agent::Claude, vec![claude_menu(false, OWN)]);
        assert!(out.unwrap_err().contains("pointer"));
        assert_eq!(sent, ["Down"]);
    }

    #[test]
    fn a_dialog_that_comes_back_after_the_answer_fails_the_read() {
        let (out, sent) = run(Agent::Claude, vec![claude_menu(true, OWN)]);
        assert!(out.unwrap_err().contains("after it was answered"));
        assert_eq!(sent, ["Enter"]);
    }

    #[test]
    fn a_screen_that_is_not_known_fails_at_the_deadline_with_its_last_line() {
        let screen = parse_pane_screen(include_str!("../../fixtures/panes/claude-permission.txt"));
        let (out, sent) = run(Agent::Claude, vec![screen]);
        assert!(out.unwrap_err().contains("prompt adj does not know"));
        assert!(sent.is_empty());
    }

    #[test]
    fn an_idle_prompt_is_settled_at_once_and_a_gone_pane_is_not_a_failure() {
        let (out, sent) = run(Agent::Claude, vec![claude_idle()]);
        assert_eq!((out, sent.len()), (Ok(()), 0));
        let out = settle_reader_with(
            || None,
            |_| Ok(()),
            |_| {},
            Agent::Claude,
            Path::new(OWN),
            40,
        );
        assert_eq!(out, Ok(()));
    }

    #[test]
    fn a_home_relative_directory_is_the_same_directory_written_short() {
        let own = vec!["/Users/a/state/review/o~r~1".to_string()];
        // SAFETY of the comparison: only the string rule is under test, with HOME as it is.
        if let Ok(home) = std::env::var("HOME") {
            let own = vec![format!("{}/x", home.trim_end_matches('/'))];
            assert!(is_own("~/x", &own));
            assert!(!is_own("~/x/y", &own));
        }
        assert!(is_own("/Users/a/state/review/o~r~1/", &own));
        assert!(!is_own("/Users/a/state/review", &own));
    }

    #[test]
    fn the_watcher_is_started_detached_with_every_argument_quoted() {
        assert_eq!(
            watcher_command("/opt/my bin/adj", "o/r#1", "s-1", "%4", None),
            "trap '' HUP; '/opt/my bin/adj' review settle --pr 'o/r#1' --session s-1 --pane %4 </dev/null >/dev/null 2>&1 &"
        );
        assert!(
            watcher_command("adj", "o/r#1", "s-1", "%4", Some("/tmp/t.sock"))
                .contains("--pane %4 --socket /tmp/t.sock </dev/null")
        );
    }

    fn blank() -> PaneScreen {
        parse_pane_screen("   \n\n@@adjutant:pane@@\n0\t0\t0")
    }

    #[test]
    fn one_failed_look_is_not_a_gone_pane_but_three_are() {
        let looks = RefCell::new(VecDeque::from(vec![
            None,
            Some(claude_menu(false, OWN)),
            Some(claude_menu(true, OWN)),
            Some(claude_idle()),
        ]));
        let sent = RefCell::new(Vec::new());
        let out = settle_reader_with(
            || looks.borrow_mut().pop_front().flatten(),
            |key| {
                sent.borrow_mut().push(key.to_string());
                Ok(())
            },
            |_| {},
            Agent::Claude,
            Path::new(OWN),
            40,
        );
        assert_eq!(out, Ok(()));
        assert_eq!(sent.into_inner(), ["Down", "Enter"]);
        let mut calls = 0;
        let out = settle_reader_with(
            || {
                calls += 1;
                None
            },
            |_| Ok(()),
            |_| {},
            Agent::Claude,
            Path::new(OWN),
            40,
        );
        assert_eq!((out, calls), (Ok(()), 3));
    }

    #[test]
    fn nothing_is_sent_while_the_pane_is_in_copy_mode() {
        let mut copying = claude_menu(false, OWN);
        copying.in_mode = true;
        let (out, sent) = run(Agent::Claude, vec![copying.clone(), copying]);
        // Still in copy mode at the deadline: the screen was never evaluated, so it is no failure.
        assert_eq!(out, Ok(()));
        assert!(sent.is_empty());
    }

    /// The claude dialog with its path wrapped over two lines, as the agent's own layout does.
    fn wrapped_menu(dir: &str, at: usize, pointed: bool) -> PaneScreen {
        let (a, b) = dir.split_at(at);
        let text = include_str!("../../fixtures/panes/claude-menu.txt")
            .replace("/home/user/project", &format!("{a}\n {b}"));
        let text = if pointed {
            text.replace(
                " ❯ No, exit\n   Yes, I trust",
                "   No, exit\n ❯ Yes, I trust",
            )
        } else {
            text
        };
        parse_pane_screen(&text)
    }

    #[test]
    fn a_wrapped_path_is_the_own_directory_and_is_answered() {
        let (out, sent) = run(
            Agent::Claude,
            vec![
                wrapped_menu(OWN, 10, false),
                wrapped_menu(OWN, 10, true),
                claude_idle(),
            ],
        );
        assert_eq!(out, Ok(()));
        assert_eq!(sent, ["Down", "Enter"]);
    }

    #[test]
    fn a_wrapped_path_naming_another_directory_is_not_answered() {
        let (out, sent) = run(
            Agent::Claude,
            vec![wrapped_menu("/state/review/o~r~2", 10, false)],
        );
        assert!(out.unwrap_err().contains("/state/review/o~r~2"));
        assert!(sent.is_empty());
    }

    #[test]
    fn a_key_that_cannot_be_sent_says_so() {
        let out = settle_reader_with(
            || Some(claude_menu(true, OWN)),
            |_| Err("no server".to_string()),
            |_| {},
            Agent::Claude,
            Path::new(OWN),
            40,
        );
        assert_eq!(
            out,
            Err("could not send a key to the reader's pane: no server".to_string())
        );
    }

    #[test]
    fn the_pointer_is_waited_for_after_down_and_down_is_sent_once() {
        let (out, sent) = run(
            Agent::Claude,
            vec![
                claude_menu(false, OWN),
                claude_menu(false, OWN),
                claude_menu(true, OWN),
                claude_idle(),
            ],
        );
        assert_eq!(out, Ok(()));
        assert_eq!(sent, ["Down", "Enter"]);
    }

    #[test]
    fn the_pointer_on_exactly_the_third_look_is_in_time() {
        let (out, sent) = run(
            Agent::Claude,
            vec![
                claude_menu(false, OWN),
                claude_menu(false, OWN),
                claude_menu(false, OWN),
                claude_menu(true, OWN),
                claude_idle(),
            ],
        );
        assert_eq!(out, Ok(()));
        assert_eq!(sent, ["Down", "Enter"]);
    }

    #[test]
    fn down_is_not_sent_again_when_the_pointer_is_still_on_no_later() {
        let looks = RefCell::new(VecDeque::from(vec![
            Some(claude_menu(false, OWN)),
            None,
            None,
            None,
            Some(claude_menu(false, OWN)),
        ]));
        let sent = RefCell::new(Vec::new());
        let out = settle_reader_with(
            || looks.borrow_mut().pop_front().flatten(),
            |key| {
                sent.borrow_mut().push(key.to_string());
                Ok(())
            },
            |_| {},
            Agent::Claude,
            Path::new(OWN),
            40,
        );
        assert!(out.unwrap_err().contains("pointer could not be put"));
        assert_eq!(sent.into_inner(), ["Down"]);
    }

    #[test]
    fn a_screen_still_blank_at_the_deadline_is_a_slow_start_not_a_failure() {
        let (out, sent) = run(Agent::Claude, vec![blank()]);
        assert_eq!(out, Ok(()));
        assert!(sent.is_empty());
    }
}
