use super::*;
use crate::config::Hook;
use std::time::Duration;
// ── reading a pane before waking it ──────────────────────────────

/// A capture as `tmux_capture_script` prints it, from a real one kept in `src/fixtures`.
fn fixture(name: &str) -> &'static str {
    match name {
        "claude-idle" => include_str!("../fixtures/panes/claude-idle.txt"),
        "claude-idle-after-turn" => include_str!("../fixtures/panes/claude-idle-after-turn.txt"),
        "claude-typing" => include_str!("../fixtures/panes/claude-typing.txt"),
        "claude-working" => include_str!("../fixtures/panes/claude-working.txt"),
        "claude-working-tool" => include_str!("../fixtures/panes/claude-working-tool.txt"),
        "claude-working-typed" => include_str!("../fixtures/panes/claude-working-typed.txt"),
        "claude-question" => include_str!("../fixtures/panes/claude-question.txt"),
        "claude-permission" => include_str!("../fixtures/panes/claude-permission.txt"),
        "claude-menu" => include_str!("../fixtures/panes/claude-menu.txt"),
        "agy-idle" => include_str!("../fixtures/panes/agy-idle.txt"),
        "agy-idle-after-turn" => include_str!("../fixtures/panes/agy-idle-after-turn.txt"),
        "agy-typing" => include_str!("../fixtures/panes/agy-typing.txt"),
        "agy-working" => include_str!("../fixtures/panes/agy-working.txt"),
        "agy-working-typed" => include_str!("../fixtures/panes/agy-working-typed.txt"),
        "agy-question" => include_str!("../fixtures/panes/agy-question.txt"),
        "agy-permission" => include_str!("../fixtures/panes/agy-permission.txt"),
        "agy-menu" => include_str!("../fixtures/panes/agy-menu.txt"),
        other => panic!("no fixture called {other}"),
    }
}

const FIXTURES: &[(Agent, &str, PaneState)] = &[
    (Agent::Claude, "claude-idle", PaneState::Idle),
    (Agent::Claude, "claude-idle-after-turn", PaneState::Idle),
    (Agent::Claude, "claude-typing", PaneState::Typing),
    (Agent::Claude, "claude-working", PaneState::Working),
    (Agent::Claude, "claude-working-tool", PaneState::Working),
    (Agent::Claude, "claude-working-typed", PaneState::Typing),
    (Agent::Claude, "claude-question", PaneState::Asking),
    (Agent::Claude, "claude-permission", PaneState::Asking),
    (Agent::Claude, "claude-menu", PaneState::Asking),
    (Agent::Agy, "agy-idle", PaneState::Idle),
    (Agent::Agy, "agy-idle-after-turn", PaneState::Idle),
    (Agent::Agy, "agy-typing", PaneState::Typing),
    (Agent::Agy, "agy-working", PaneState::Working),
    (Agent::Agy, "agy-working-typed", PaneState::Typing),
    (Agent::Agy, "agy-question", PaneState::Asking),
    (Agent::Agy, "agy-permission", PaneState::Asking),
    (Agent::Agy, "agy-menu", PaneState::Asking),
];

#[test]
fn every_captured_screen_reads_as_what_it_was() {
    for (agent, name, expected) in FIXTURES {
        let screen = parse_pane_screen(fixture(name));
        assert_eq!(pane_state(*agent, &screen), *expected, "{name}");
    }
}

#[test]
fn a_capture_carries_escapes_on_every_line_and_reads_the_same() {
    // The fixtures keep escapes only where the placeholder needs them; a real capture
    // has them everywhere.
    for (agent, name, expected) in FIXTURES {
        let mut screen = parse_pane_screen(fixture(name));
        screen.text = screen
            .text
            .lines()
            .map(|l| format!("\x1b[38;5;244m{l}\x1b[0m"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(pane_state(*agent, &screen), *expected, "{name}");
    }
}

#[test]
fn a_suggestion_in_an_empty_box_is_not_typed_text() {
    // Claude fills an empty box with a suggestion, drawn faint. Read as text it would
    // make every fresh session look like somebody was typing.
    let screen = parse_pane_screen(fixture("claude-idle"));
    assert!(screen.text.contains("Try \""), "{}", screen.text);
    assert_eq!(pane_state(Agent::Claude, &screen), PaneState::Idle);
    // The same words, not faint, are typed text.
    let mut typed = screen.clone();
    typed.text = typed.text.replace("\x1b[2m", "");
    assert_eq!(pane_state(Agent::Claude, &typed), PaneState::Typing);
}

// ── the last line of output ──────────────────────────────────────

#[test]
fn the_last_line_is_the_one_above_the_input_box() {
    let last = |agent, name| last_output_line(agent, &parse_pane_screen(fixture(name)));
    // Not the box, its border, the prompt or the footer under it; and not the effort level
    // the agent draws at the right of the row above the box.
    assert_eq!(
        last(Agent::Claude, "claude-idle-after-turn").as_deref(),
        Some("✻ Brewed for 3s · done 2:20")
    );
    // Nor the tip it shows under the spinner, or the hint about the terminal it is in.
    assert_eq!(
        last(Agent::Claude, "claude-working").as_deref(),
        Some("✻ Pondering… (3s · thinking)")
    );
    assert_eq!(
        last(Agent::Claude, "claude-working-tool").as_deref(),
        Some("✻ Pondering… (5s · ↓ 264 tokens · thought for 2s)")
    );
    assert_eq!(
        last(Agent::Agy, "agy-working").as_deref(),
        Some("⡿  Generating...")
    );
    assert_eq!(
        last(Agent::Agy, "agy-idle-after-turn").as_deref(),
        Some("rustic appeal, and universally restorative, comforting charm.")
    );
}

#[test]
fn a_screen_without_an_input_box_gives_its_last_line() {
    // A question replaces the box, so there is nothing to look above; its key hints are not
    // the question.
    let question = parse_pane_screen(fixture("claude-question"));
    assert_eq!(
        last_output_line(Agent::Claude, &question).as_deref(),
        Some("4. Chat about this")
    );
    // An agent whose screen is not known has no box to find, whatever it draws.
    let generic = parse_pane_screen("$ make\ncc -o a a.c\n\n  built a\n\n");
    assert_eq!(
        last_output_line(Agent::Generic, &generic).as_deref(),
        Some("built a")
    );
}

#[test]
fn a_line_that_only_mentions_a_key_is_not_a_hint() {
    let screen = parse_pane_screen("building\nPress Esc to cancel the build\n");
    assert_eq!(
        last_output_line(Agent::Generic, &screen).as_deref(),
        Some("Press Esc to cancel the build")
    );
}

#[test]
fn the_last_line_has_no_escapes_and_is_cut_short() {
    let coloured = parse_pane_screen("\x1b[31m\x1b[1merror\x1b[0m: no such file\n");
    assert_eq!(
        last_output_line(Agent::Generic, &coloured).as_deref(),
        Some("error: no such file")
    );
    let long = parse_pane_screen(&format!("{}\n", "あ".repeat(250)));
    let line = last_output_line(Agent::Generic, &long).unwrap();
    assert_eq!(line.chars().count(), LAST_LINE_CHARS + 1);
    assert!(line.ends_with('…'), "{line}");
    // A line of exactly the limit is whole.
    let exact = parse_pane_screen(&format!("{}\n", "a".repeat(LAST_LINE_CHARS)));
    assert_eq!(
        last_output_line(Agent::Generic, &exact).map(|l| l.len()),
        Some(LAST_LINE_CHARS)
    );
}

#[test]
fn a_blank_screen_has_no_last_line() {
    for agent in [Agent::Claude, Agent::Agy, Agent::Generic] {
        let blank = parse_pane_screen("\n  \n\n");
        assert_eq!(last_output_line(agent, &blank), None);
    }
    // Only a box and its footer: nothing was written above it.
    let empty_box = parse_pane_screen(&format!(
        "\n\n{rule}\n❯\n{rule}\n  ? for shortcuts\n",
        rule = "─".repeat(40)
    ));
    assert_eq!(last_output_line(Agent::Claude, &empty_box), None);
}

#[test]
fn a_turn_past_a_minute_is_still_a_turn_in_progress() {
    // The timer runs `3s`, then `1m 5s`, then `1h 2m`.
    let working = fixture("claude-working");
    assert!(working.contains("(3s ·"), "{working}");
    for timer in ["(59s ·", "(1m 5s ·", "(12m ·", "(1h 2m ·"] {
        let mut screen = parse_pane_screen(working);
        screen.text = screen.text.replace("(3s ·", timer);
        assert_eq!(
            pane_state(Agent::Claude, &screen),
            PaneState::Working,
            "{timer}"
        );
    }
    // The line a finished turn leaves has no brackets to read.
    let mut done = parse_pane_screen(working);
    done.text = done
        .text
        .replace("Pondering… (3s · thinking)", "Baked for 1m 5s · done");
    assert_eq!(pane_state(Agent::Claude, &done), PaneState::Idle);
}

#[test]
fn a_spinner_line_before_its_first_tick_is_a_turn_too() {
    let working = fixture("claude-working");
    let mut screen = parse_pane_screen(working);
    screen.text = screen
        .text
        .replace("Pondering… (3s · thinking)", "Pondering…");
    assert_eq!(pane_state(Agent::Claude, &screen), PaneState::Working);
    // The dot also leads ordinary lines; one ending in an ellipsis is not a spinner.
    let mut listed = parse_pane_screen(working);
    listed.text = listed
        .text
        .replace("✻ Pondering… (3s · thinking)", "· and so on…");
    assert_eq!(pane_state(Agent::Claude, &listed), PaneState::Idle);
}

#[test]
fn the_ascii_spinner_of_other_platforms_is_a_turn_and_a_bullet_is_not() {
    let working = fixture("claude-working");
    let mut star = parse_pane_screen(working);
    star.text = star.text.replace('✻', "*");
    assert_eq!(pane_state(Agent::Claude, &star), PaneState::Working);
    let mut bullet = parse_pane_screen(working);
    bullet.text = bullet
        .text
        .replace("✻ Pondering… (3s · thinking)", "* item in a list");
    assert_eq!(pane_state(Agent::Claude, &bullet), PaneState::Idle);
}

#[test]
fn an_echo_of_an_old_numbered_message_in_the_transcript_is_not_a_question() {
    for (agent, name, echo) in [
        (Agent::Claude, "claude-idle-after-turn", "❯ 1. do this"),
        (Agent::Agy, "agy-idle-after-turn", "> 1. do this"),
    ] {
        let mut screen = parse_pane_screen(fixture(name));
        screen.text = format!("{echo}\n{}", screen.text);
        assert_eq!(pane_state(agent, &screen), PaneState::Idle, "{name}");
    }
    // Typed in the box, it is still somebody typing.
    let mut typing = parse_pane_screen(fixture("claude-typing"));
    typing.text = typing.text.replace("hello typed", "1. foo");
    assert_eq!(pane_state(Agent::Claude, &typing), PaneState::Typing);
    let mut typing = parse_pane_screen(fixture("agy-typing"));
    typing.text = typing.text.replace("hello typed", "1. foo");
    assert_eq!(pane_state(Agent::Agy, &typing), PaneState::Typing);
}

#[test]
fn a_pane_in_copy_mode_is_not_typed_into() {
    for (agent, name, _) in FIXTURES {
        let mut screen = parse_pane_screen(fixture(name));
        screen.in_mode = true;
        assert_eq!(pane_state(*agent, &screen), PaneState::CopyMode, "{name}");
    }
}

#[test]
fn an_agent_of_unknown_kind_is_never_looked_at() {
    for (_, name, _) in FIXTURES {
        let screen = parse_pane_screen(fixture(name));
        assert_eq!(
            pane_state(Agent::Generic, &screen),
            PaneState::Idle,
            "{name}"
        );
    }
}

#[test]
fn a_screen_that_is_not_the_agents_is_unknown() {
    for text in [
        "",
        "$ ",
        "user@host project % ls\nCargo.toml  src\nuser@host project %",
        "❯ not in a box\n",
    ] {
        let screen = parse_pane_screen(&format!("{text}\n{PANE_META_SEPARATOR}\n0\t0\t0\n"));
        for agent in [Agent::Claude, Agent::Agy] {
            assert_eq!(pane_state(agent, &screen), PaneState::Unknown, "{text:?}");
        }
    }
}

#[test]
fn a_prompt_with_a_lot_under_it_is_not_taken_for_the_input_box() {
    let rule = "─".repeat(40);
    let mut text = format!("{rule}\n❯\n{rule}\n");
    for n in 0..=MAX_LINES_BELOW_INPUT {
        text.push_str(&format!("something drawn over it {n}\n"));
    }
    let screen = parse_pane_screen(&format!("{text}{PANE_META_SEPARATOR}\n0\t0\t0\n"));
    assert_eq!(pane_state(Agent::Claude, &screen), PaneState::Unknown);
}

#[test]
fn a_confirmation_is_a_question_whatever_the_box_says() {
    let rule = "─".repeat(40);
    let text = format!("Overwrite the file? (y/n)\n{rule}\n❯\n{rule}\n");
    let screen = parse_pane_screen(&format!("{text}{PANE_META_SEPARATOR}\n0\t0\t0\n"));
    assert_eq!(pane_state(Agent::Claude, &screen), PaneState::Asking);
}

/// A pane the wake looks at, played back: each capture returns the next screen (the last
/// one for as long as it is asked), and everything run is written down.
struct FakePane {
    screens: std::cell::RefCell<std::collections::VecDeque<String>>,
    log: std::cell::RefCell<Vec<String>>,
    waits: std::cell::RefCell<Vec<Duration>>,
}

impl FakePane {
    fn new(screens: &[String]) -> Self {
        FakePane {
            screens: std::cell::RefCell::new(screens.iter().cloned().collect()),
            log: Default::default(),
            waits: Default::default(),
        }
    }

    fn run(&self, cmd: &str) -> Result<String, String> {
        self.log.borrow_mut().push(cmd.to_string());
        if cmd.contains("list-panes") {
            Ok("%1\t12345\t/dev/ttys005\t@1\tadjutant\t1\tworker\n".to_string())
        } else if cmd.contains("capture-pane") {
            let mut screens = self.screens.borrow_mut();
            let next = if screens.len() > 1 {
                screens.pop_front()
            } else {
                screens.front().cloned()
            };
            next.ok_or_else(|| "no screen".to_string())
        } else if cmd.contains("send-keys") {
            Ok(format!("{WOKE_MARKER}\n"))
        } else {
            Err(format!("unexpected command: {cmd}"))
        }
    }

    fn count(&self, needle: &str) -> usize {
        self.log
            .borrow()
            .iter()
            .filter(|c| c.contains(needle))
            .count()
    }

    fn wake(&self, agent: Agent, line: &str) -> Performed {
        let term = TerminalSettings {
            preset: Some("tmux".to_string()),
            ..Default::default()
        };
        let locks = tempfile::tempdir().unwrap();
        wake_with_clock(
            |cmd| self.run(cmd),
            |d| self.waits.borrow_mut().push(d),
            locks.path(),
            Some("ttys005".to_string()),
            &term,
            &Wake::default(),
            &WakeRequest {
                pid: 12345,
                subject: "s",
                line,
                look: look_before_typing(agent),
                dry_run: false,
            },
        )
        .unwrap()
    }
}

/// `screen` with `line` typed at its prompt, as the agent would show it after the keys.
fn with_typed(screen: &str, empty: &str, line: &str) -> String {
    screen.replacen(empty, line, 1)
}

const WAKE: &str =
    "Something arrived in the inbox. Check it with adjutant_pending and deal with it.";

fn claude_idle_with_line_typed() -> String {
    with_typed(fixture("claude-typing"), "hello typed", WAKE)
}

#[test]
fn a_question_on_screen_is_not_answered_by_the_wake() {
    let pane = FakePane::new(&[fixture("claude-question").to_string()]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("question or a menu"),
        "{}",
        done.description
    );
    assert!(
        done.description.contains("not typed"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys"), 0, "{:?}", pane.log.borrow());
    // It is waited for, in case the person answers, and then given up on.
    let looks = 1 + (WAKE_READY_BUDGET.as_millis() / WAKE_READY_POLL.as_millis()) as usize;
    assert_eq!(pane.count("capture-pane"), looks);
    assert_eq!(pane.waits.borrow().len(), looks - 1);
}

#[test]
fn a_busy_agent_is_typed_into_once_it_is_back_at_its_prompt() {
    let pane = FakePane::new(&[
        fixture("claude-working").to_string(),
        fixture("claude-idle").to_string(),
        claude_idle_with_line_typed(),
    ]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(done.ran, "{done:?}");
    assert!(
        done.description.contains("woke the session"),
        "{}",
        done.description
    );
    assert_eq!(*pane.waits.borrow(), vec![WAKE_READY_POLL]);
    let log = pane.log.borrow();
    let typed = log.iter().position(|c| c.contains("send-keys -l")).unwrap();
    let enter = log
        .iter()
        .position(|c| c.contains("send-keys -t %1 Enter"))
        .unwrap();
    assert!(typed < enter, "{log:?}");
    assert!(log[typed].contains(WAKE), "{log:?}");
}

#[test]
fn an_agent_busy_for_good_is_looked_at_for_the_whole_budget_and_then_left() {
    let pane = FakePane::new(&[fixture("agy-working").to_string()]);
    let done = pane.wake(Agent::Agy, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("middle of a turn"),
        "{}",
        done.description
    );
    let looks = 1 + (WAKE_READY_BUDGET.as_millis() / WAKE_READY_POLL.as_millis()) as usize;
    assert_eq!(pane.count("capture-pane"), looks);
    assert_eq!(pane.count("send-keys"), 0);
}

#[test]
fn a_person_partway_through_a_message_is_left_alone() {
    let pane = FakePane::new(&[fixture("agy-typing").to_string()]);
    let done = pane.wake(Agent::Agy, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("text typed"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys"), 0);
}

#[test]
fn a_screen_that_is_not_recognised_is_not_typed_into() {
    let pane = FakePane::new(&["$ \n@@adjutant:pane@@\n0\t2\t0\n".to_string()]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("not recognised"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys"), 0);
}

#[test]
fn a_pane_that_cannot_be_read_is_not_typed_into() {
    let pane = FakePane::new(&[]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("could not be read"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys"), 0);
}

#[test]
fn enter_is_not_pressed_when_the_line_did_not_reach_the_prompt() {
    // The screen changed between looking and typing, or the keys went somewhere else:
    // Enter would answer whatever is there.
    let pane = FakePane::new(&[fixture("claude-idle").to_string()]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("Enter was not pressed"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys -l"), 1);
    assert_eq!(pane.count("Enter"), 0);
    assert_eq!(pane.count("capture-pane"), 1 + WAKE_ECHO_LOOKS as usize);
}

#[test]
fn enter_is_not_pressed_when_the_person_started_typing_before_the_line_went_in() {
    // Their text and the line are in the box together; Enter would send both.
    let both = with_typed(
        fixture("claude-typing"),
        "hello typed",
        &format!("hello typed {WAKE}"),
    );
    let pane = FakePane::new(&[fixture("claude-idle").to_string(), both]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(done.screen, "{done:?}");
    assert!(
        done.description.contains("left at the prompt"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("Enter"), 0);
}

/// One pane shared by wakes running at the same moment: what is typed in its box, and
/// what has been sent from it.
struct SharedPane {
    state: std::sync::Mutex<(String, Vec<String>)>,
}

impl SharedPane {
    fn run(&self, cmd: &str) -> Result<String, String> {
        if cmd.contains("list-panes") {
            return Ok("%1\t12345\t/dev/ttys005\t@1\tadjutant\t1\tworker\n".to_string());
        }
        if cmd.contains("capture-pane") {
            let typed = self.state.lock().unwrap().0.clone();
            // Slow enough for another wake to look in between.
            std::thread::sleep(Duration::from_millis(30));
            return Ok(if typed.is_empty() {
                fixture("claude-idle").to_string()
            } else {
                with_typed(fixture("claude-typing"), "hello typed", &typed)
            });
        }
        let mut state = self.state.lock().unwrap();
        if cmd.contains("send-keys -l") {
            let line = cmd.split('\'').nth(1).unwrap().to_string();
            if !state.0.is_empty() {
                state.0.push(' ');
            }
            state.0.push_str(&line);
        } else if cmd.contains("Enter") {
            let sent = std::mem::take(&mut state.0);
            state.1.push(sent);
            return Ok(format!("{WOKE_MARKER}\n"));
        }
        Ok(String::new())
    }
}

#[test]
fn wakes_at_the_same_moment_take_turns_at_the_pane() {
    let pane = std::sync::Arc::new(SharedPane {
        state: Default::default(),
    });
    let locks = tempfile::tempdir().unwrap();
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        ..Default::default()
    };
    let lines = ["first line from one worker", "second line from another"];
    let results: Vec<Performed> = std::thread::scope(|scope| {
        let handles: Vec<_> = lines
            .iter()
            .map(|line| {
                let (pane, term, locks) = (&pane, &term, locks.path());
                scope.spawn(move || {
                    wake_with_clock(
                        |cmd| pane.run(cmd),
                        std::thread::sleep,
                        locks,
                        Some("ttys005".to_string()),
                        term,
                        &Wake::default(),
                        &WakeRequest {
                            pid: 12345,
                            subject: "s",
                            line,
                            look: look_before_typing(Agent::Claude),
                            dry_run: false,
                        },
                    )
                    .unwrap()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(results.iter().all(|r| r.ran), "{results:?}");
    let mut sent = pane.state.lock().unwrap().1.clone();
    sent.sort();
    assert_eq!(sent, vec![lines[0].to_string(), lines[1].to_string()]);
}

#[test]
fn a_wake_that_cannot_get_the_pane_within_the_budget_gives_up() {
    let pane = FakePane::new(&[fixture("claude-idle").to_string()]);
    let locks = tempfile::tempdir().unwrap();
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        ..Default::default()
    };
    // Someone else holds the pane, as `lock_pane` takes it.
    let mut waited = Duration::ZERO;
    let held = lock_pane(locks.path(), term.tmux_socket(), "%1", &|_| {}, &mut waited)
        .unwrap()
        .unwrap();
    let done = wake_with_clock(
        |cmd| pane.run(cmd),
        |d| pane.waits.borrow_mut().push(d),
        locks.path(),
        Some("ttys005".to_string()),
        &term,
        &Wake::default(),
        &WakeRequest {
            pid: 12345,
            subject: "s",
            line: WAKE,
            look: look_before_typing(Agent::Claude),
            dry_run: false,
        },
    )
    .unwrap();
    assert!(!done.ran && done.screen, "{done:?}");
    assert!(
        done.description.contains("another wake"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys"), 0);
    assert_eq!(pane.count("capture-pane"), 0);
    let waits = (WAKE_READY_BUDGET.as_millis() / WAKE_READY_POLL.as_millis()) as usize;
    assert_eq!(pane.waits.borrow().len(), waits);
    drop(held);
}

#[test]
fn a_line_wrapped_in_the_box_still_counts_as_shown() {
    let (first, second) = WAKE.split_at(60);
    let rule = "─".repeat(40);
    let wrapped = format!(
        "{rule}\n❯ {first}\n  {second}\n{rule}\n  footer\n{PANE_META_SEPARATOR}\n0\t2\t0\n"
    );
    let pane = FakePane::new(&[fixture("claude-idle").to_string(), wrapped]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(done.ran, "{done:?}");
}

#[test]
fn agy_is_typed_into_at_its_prompt_too() {
    let pane = FakePane::new(&[
        fixture("agy-idle").to_string(),
        with_typed(fixture("agy-typing"), "hello typed", WAKE),
    ]);
    let done = pane.wake(Agent::Agy, WAKE);
    assert!(done.ran, "{done:?}");
    assert!(pane.waits.borrow().is_empty());
}

#[test]
fn where_the_screen_is_not_read_nothing_is_captured() {
    // Generic: today's single script, typed without looking.
    let pane = FakePane::new(&[]);
    let done = pane.wake(Agent::Generic, WAKE);
    assert!(done.ran, "{done:?}");
    assert_eq!(pane.count("capture-pane"), 0);
    assert!(done.script.contains("sleep"), "{}", done.script);

    // A dry run only prints.
    let pane = FakePane::new(&[]);
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        ..Default::default()
    };
    let locks = tempfile::tempdir().unwrap();
    let done = wake_with_clock(
        |cmd| pane.run(cmd),
        |d| pane.waits.borrow_mut().push(d),
        locks.path(),
        None,
        &term,
        &Wake::default(),
        &WakeRequest {
            pid: 12345,
            subject: "s",
            line: WAKE,
            look: look_before_typing(Agent::Claude),
            dry_run: true,
        },
    );
    // No tty, so the pane is found by pid.
    let done = done.unwrap();
    assert_eq!(pane.count("capture-pane"), 0);
    assert!(done.script.contains("send-keys -l"), "{}", done.script);
    assert!(!done.ran);

    // A template is someone else's command; the built-in for another terminal is too.
    for (wake, terminal) in [
        (
            Wake {
                hook: Hook::Command("tmux send-keys -t {tty} {line} Enter".into()),
                line: None,
            },
            term.clone(),
        ),
        (Wake::default(), TerminalSettings::default()),
    ] {
        let pane = FakePane::new(&[]);
        let locks = tempfile::tempdir().unwrap();
        let done = wake_with_clock(
            |cmd| {
                pane.log.borrow_mut().push(cmd.to_string());
                Ok(WOKE_MARKER.to_string())
            },
            |d| pane.waits.borrow_mut().push(d),
            locks.path(),
            Some("ttys005".to_string()),
            &terminal,
            &wake,
            &WakeRequest {
                pid: 12345,
                subject: "s",
                line: WAKE,
                look: look_before_typing(Agent::Claude),
                dry_run: false,
            },
        )
        .unwrap();
        assert!(done.ran, "{done:?}");
        assert_eq!(pane.count("capture-pane"), 0, "{:?}", pane.log.borrow());
        assert!(pane.waits.borrow().is_empty());
    }
}
