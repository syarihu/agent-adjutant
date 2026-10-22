use crate::board::view::{board_session, socket_key};
use crate::board::{Server, settings_now};
use crate::infra::agent::Agent;
use crate::infra::terminal::{look_at_tmux_pane_with, tmux_key_script};
use crate::mail::{PromptRefusal, prompt_answerable};

/// How answering a prompt on a session's pane came out.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    Sent,
    NoSuchSession,
    Refused(String),
    /// The pane no longer shows the prompt the board saw: answered in the terminal, timed out,
    /// or replaced by another question.
    Gone,
    NotAnOption,
    Failed(String),
}

/// Answer the prompt a session's pane shows by sending the one key of the option pressed.
/// `seen` is the prompt's id as the board drew it; the pane is read again here, and the key is
/// sent only when it still shows that prompt and the key is one of its options.
pub fn answer_prompt(server: &Server, id: &str, seen: &str, key: &str) -> Answer {
    answer_prompt_with(server, id, seen, key, &crate::infra::shell::run_shell)
}

/// `answer_prompt` with the runner handed in.
pub fn answer_prompt_with(
    server: &Server,
    id: &str,
    seen: &str,
    key: &str,
    run: &impl Fn(&str) -> Result<String, String>,
) -> Answer {
    let (seen, key) = match checked_input(seen, key) {
        Ok(input) => input,
        Err(why) => return Answer::Refused(why),
    };
    let settings = settings_now(server);
    let Some(session) = board_session(server, &settings, id) else {
        return Answer::NoSuchSession;
    };
    if !session.present {
        return Answer::Refused("the session is not running".to_string());
    }
    if session.terminal.backend != "tmux" {
        return Answer::Refused("the session is not running in tmux".to_string());
    }
    let socket = session.terminal.socket.as_deref();
    let pane = match session.terminal.pane.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => {
            let Some(window) = session.terminal.window.as_deref() else {
                return Answer::Refused("the session has no tmux pane".to_string());
            };
            let panes = match crate::infra::terminal::list_tmux_panes_with(run, socket) {
                Ok(panes) => panes,
                Err(e) => return Answer::Failed(e),
            };
            match panes.into_iter().find(|p| p.window_id == window) {
                Some(p) => p.pane_id,
                None => return Answer::Refused("the tmux window has no pane".to_string()),
            }
        }
    };
    let agent = Agent::parse(&session.agent).unwrap_or(Agent::Generic);
    let answer = answer_on_pane_with(run, agent, socket, &pane, seen, key);
    if answer == Answer::Sent {
        // The next poll reads the pane again rather than drawing the answered prompt.
        server
            .prompts
            .drop_key(&format!("{}\t{pane}", socket_key(socket).display()));
    }
    answer
}

/// The prompt id and the key, trimmed, or why they are refused: checked before anything is
/// looked up or run.
fn checked_input<'a>(seen: &'a str, key: &'a str) -> Result<(&'a str, &'a str), String> {
    let seen = seen.trim();
    if seen.is_empty() {
        return Err("seen id cannot be blank".to_string());
    }
    let key = key.trim();
    if !matches!(key.as_bytes(), [b'1'..=b'9']) {
        return Err("key must be one digit from 1 to 9".to_string());
    }
    Ok((seen, key))
}

/// Read `pane` fresh and send `key` to it only when it shows the prompt `seen` with `key` among
/// its options. Never the cached reading: the person may have answered in the terminal since.
fn answer_on_pane_with(
    run: &impl Fn(&str) -> Result<String, String>,
    agent: Agent,
    socket: Option<&str>,
    pane: &str,
    seen: &str,
    key: &str,
) -> Answer {
    // A pane that cannot be read is not a pane without the prompt.
    let Some(screen) = look_at_tmux_pane_with(run, socket, pane) else {
        return Answer::Failed("the pane could not be read".to_string());
    };
    match prompt_answerable(agent, &screen, seen, key) {
        Ok(()) => {}
        Err(PromptRefusal::Gone) => return Answer::Gone,
        Err(PromptRefusal::NotAnOption) => return Answer::NotAnOption,
        Err(PromptRefusal::Unreadable) => {
            return Answer::Refused("the prompt options cannot be read reliably".to_string());
        }
    }
    match run(&tmux_key_script(socket, pane, key)) {
        Ok(_) => Answer::Sent,
        Err(e) => Answer::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::infra::terminal::parse_pane_screen;
    use crate::mail::pane_prompt;

    const QUESTION: &str = include_str!("../../fixtures/panes/claude-question.txt");

    #[test]
    fn bad_input_is_refused() {
        assert!(checked_input("", "1").is_err());
        assert!(checked_input(" ", "1").is_err());
        assert!(checked_input("abc", "").is_err());
        assert!(checked_input("abc", "123").is_err());
        assert!(checked_input("abc", "10").is_err());
        assert!(checked_input("abc", "0").is_err());
        assert!(checked_input("abc", "x").is_err());
        assert!(checked_input("abc", "1 2").is_err());
        assert_eq!(checked_input(" abc ", " 2 "), Ok(("abc", "2")));
    }

    /// Runs `answer_on_pane_with` against a pane that shows `screen`, and returns what it
    /// answered with the commands it ran.
    fn answer(screen: &str, seen: &str, key: &str) -> (Answer, Vec<String>) {
        let ran = RefCell::new(Vec::new());
        let run = |cmd: &str| -> Result<String, String> {
            ran.borrow_mut().push(cmd.to_string());
            Ok(if cmd.contains("capture-pane") {
                screen.to_string()
            } else {
                String::new()
            })
        };
        let answer = answer_on_pane_with(&run, Agent::Claude, None, "%1", seen, key);
        (answer, ran.into_inner())
    }

    fn sent(ran: &[String]) -> Vec<&String> {
        ran.iter().filter(|c| c.contains("send-keys")).collect()
    }

    #[test]
    fn a_stale_prompt_is_not_answered() {
        let (answer_of, ran) = answer(QUESTION, "stale", "1");
        assert_eq!(answer_of, Answer::Gone);
        assert!(sent(&ran).is_empty(), "{ran:?}");
    }

    #[test]
    fn a_prompt_gone_from_the_pane_is_not_answered() {
        let id = pane_prompt(Agent::Claude, &parse_pane_screen(QUESTION))
            .unwrap()
            .id;
        let idle = include_str!("../../fixtures/panes/claude-idle.txt");
        let (answer_of, ran) = answer(idle, &id, "1");
        assert_eq!(answer_of, Answer::Gone);
        assert!(sent(&ran).is_empty(), "{ran:?}");
    }

    #[test]
    fn a_key_that_is_not_an_option_is_not_sent() {
        let id = pane_prompt(Agent::Claude, &parse_pane_screen(QUESTION))
            .unwrap()
            .id;
        // 3 is "Type something.", which needs typing and is not offered.
        let (answer_of, ran) = answer(QUESTION, &id, "3");
        assert_eq!(answer_of, Answer::NotAnOption);
        assert!(sent(&ran).is_empty(), "{ran:?}");
    }

    #[test]
    fn a_listed_key_is_sent_alone() {
        let id = pane_prompt(Agent::Claude, &parse_pane_screen(QUESTION))
            .unwrap()
            .id;
        let (answer_of, ran) = answer(QUESTION, &id, "2");
        assert_eq!(answer_of, Answer::Sent);
        assert_eq!(sent(&ran), vec!["tmux send-keys -t %1 2"]);
    }
}
