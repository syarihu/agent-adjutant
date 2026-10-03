//! Reading what an agent's screen shows and whether it is safe to type into.
use super::*;

/// A box at the bottom of the screen with more than this under it is not an input box but
/// something drawn over one.
pub(super) const MAX_LINES_BELOW_INPUT: usize = 5;

/// What an agent's screen says it is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneState {
    /// Sitting at an empty input prompt: the one state a wake is typed into.
    Idle,
    /// A question, a permission prompt or a menu is up, and Enter would answer it.
    Asking,
    /// There is text in the input box: a person is midway through writing.
    Typing,
    /// In the middle of a turn. Typed text would be taken as a message to the running turn
    /// rather than the next one, which the wake was not written for.
    Working,
    /// The pane is scrolled back or searching (tmux copy mode); keys go to tmux, not the agent.
    CopyMode,
    /// None of the above could be told from the screen.
    Unknown,
}

impl PaneState {
    /// Why nothing was typed, for whoever is told the wake did not happen.
    pub fn why_not_typed(self) -> &'static str {
        match self {
            PaneState::Idle => "its prompt is empty",
            PaneState::Asking => "its screen shows a question or a menu",
            PaneState::Typing => "there is text typed at its prompt",
            PaneState::Working => "it is in the middle of a turn",
            PaneState::CopyMode => "its pane is in tmux copy mode",
            PaneState::Unknown => "its screen was not recognised",
        }
    }
}

/// The character an agent draws in front of its input line. Old messages in the transcript
/// start with it too; the box around the live one is what tells them apart.
fn prompt_glyph(agent: Agent) -> char {
    match agent {
        Agent::Agy => '>',
        _ => '❯',
    }
}

/// Lines that mean a choice is on screen and Enter would make it, whichever agent draws it.
const ASKING_HINTS: &[&str] = &["(y/n)", "[y/n]", "(yes/no)", "[yes/no]"];
const CLAUDE_ASKING_HINTS: &[&str] = &[
    "esc to cancel",
    "enter to select",
    "enter to confirm",
    "tab to amend",
];
// agy's footer says `esc to cancel` while it works as well as while it asks, so that is not a
// marker for it; the navigation hint is only ever drawn with a list.
const AGY_ASKING_HINTS: &[&str] = &["↑/↓ navigate"];

/// A line the spinner is drawn on. Claude counts the seconds it has been at it in brackets
/// after an ellipsis (`✻ … (3s · thinking)`, and `1m 5s` once it passes a minute), with a verb the user can change, so it is the
/// shape that is looked for; the line left behind when a turn ends (`✻ Baked for 4s · done`)
/// has no ellipsis and no brackets.
fn claude_spinner(line: &str) -> bool {
    let line = line.trim_start();
    let Some(first) = line.chars().next() else {
        return false;
    };
    if !"✻✽✶✳✢·*".contains(first) {
        return false;
    }
    // Before the first tick there is no timer yet: `✻ Pondering…`. Not with the dot, which
    // also leads ordinary list lines.
    if first != '·' && line.trim_end().ends_with('…') {
        return true;
    }
    let Some((_, rest)) = line.split_once("… (") else {
        return false;
    };
    // The timer is `3s`, then `1m 5s`, then `1h 2m`: a turn of a minute or more still counts.
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].starts_with(['h', 'm', 's'])
}

/// agy draws a braille spinner in front of what it is doing.
fn agy_spinner(line: &str) -> bool {
    line.trim_start()
        .chars()
        .next()
        .is_some_and(|c| ('\u{2800}'..='\u{28ff}').contains(&c))
}

fn working_marker(agent: Agent, line: &str) -> bool {
    let lower = line.to_lowercase();
    match agent {
        Agent::Claude => claude_spinner(line) || lower.contains("esc to interrupt"),
        Agent::Agy => agy_spinner(line) || lower.trim_start().starts_with("esc to cancel"),
        Agent::Generic => false,
    }
}

/// `line` without the escape sequences `capture-pane -e` puts in it.
fn strip_escapes(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            // A control sequence ends at its first byte in @..~.
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

/// Whether everything drawn after the prompt glyph on this line is drawn faint, which is how
/// an agent shows the suggestion it fills an empty box with. Text someone typed is not.
fn faint_after_glyph(raw: &str, glyph: char) -> bool {
    let mut faint = false;
    let mut past_glyph = false;
    let mut any = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() != Some(&'[') {
                continue;
            }
            chars.next();
            let mut params = String::new();
            let mut last = ' ';
            for c in chars.by_ref() {
                last = c;
                if ('@'..='~').contains(&c) {
                    break;
                }
                params.push(c);
            }
            if last == 'm' {
                let mut parts = params.split(';');
                while let Some(p) = parts.next() {
                    match p {
                        "" | "0" | "22" => faint = false,
                        "2" => faint = true,
                        // A colour's own arguments are not attributes: `38;5;2` is not faint.
                        "38" | "48" => {
                            let skip = if parts.next() == Some("2") { 3 } else { 1 };
                            for _ in 0..skip {
                                parts.next();
                            }
                        }
                        _ => {}
                    }
                }
            }
            continue;
        }
        if !past_glyph {
            past_glyph = c == glyph;
        } else if !c.is_whitespace() {
            any = true;
            if !faint {
                return false;
            }
        }
    }
    any
}

/// A line that is a horizontal rule: the edge of the input box.
fn is_border(plain: &str) -> bool {
    let line = plain.trim();
    line.starts_with("───") && line.chars().filter(|&c| c == '─').count() >= 10
}

/// A line that is one of a numbered list's items with the pointer on it: `❯ 1. Yes`.
fn is_pointed_option(plain: &str) -> bool {
    let line = plain.trim_start();
    let Some(rest) = line.strip_prefix(['❯', '>', '›']) else {
        return false;
    };
    let rest = rest.trim_start();
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].starts_with(['.', ')'])
}

struct Line {
    raw: String,
    plain: String,
}

struct InputBox {
    /// What is typed in it, apart from a suggestion the agent drew there.
    text: String,
    /// The row of its top border.
    top: usize,
    /// The row of its bottom border.
    bottom: usize,
}

/// The input box nearest the bottom: a line starting with the prompt glyph that has a border
/// above it and, further down, another one.
fn input_box(agent: Agent, lines: &[Line]) -> Option<InputBox> {
    let glyph = prompt_glyph(agent);
    for top in (0..lines.len().saturating_sub(1)).rev() {
        if !is_border(&lines[top].plain) || !lines[top + 1].plain.trim_start().starts_with(glyph) {
            continue;
        }
        let Some(bottom) = (top + 2..lines.len()).find(|&i| is_border(&lines[i].plain)) else {
            continue;
        };
        let below = lines[bottom + 1..]
            .iter()
            .filter(|l| !l.plain.trim().is_empty())
            .count();
        if below > MAX_LINES_BELOW_INPUT {
            continue;
        }
        let first = &lines[top + 1];
        let mut text = first
            .plain
            .trim_start()
            .trim_start_matches(glyph)
            .trim()
            .to_string();
        let suggestion = agent == Agent::Claude && faint_after_glyph(&first.raw, glyph);
        if suggestion {
            text.clear();
        }
        for line in &lines[top + 2..bottom] {
            text.push(' ');
            text.push_str(line.plain.trim());
        }
        return Some(InputBox {
            text: text.trim().to_string(),
            top,
            bottom,
        });
    }
    None
}

/// What `pane_state` decided, and what it found typed in the input box on the way.
struct Reading {
    state: PaneState,
    typed: String,
}

fn read_pane(agent: Agent, screen: &PaneScreen) -> Reading {
    let reading = |state| Reading {
        state,
        typed: String::new(),
    };
    if agent == Agent::Generic {
        return reading(PaneState::Idle);
    }
    if screen.in_mode {
        return reading(PaneState::CopyMode);
    }
    let lines: Vec<Line> = screen
        .text
        .lines()
        .map(|raw| Line {
            plain: strip_escapes(raw),
            raw: raw.to_string(),
        })
        .collect();
    let hints = match agent {
        Agent::Agy => AGY_ASKING_HINTS,
        _ => CLAUDE_ASKING_HINTS,
    };
    let input = input_box(agent, &lines);
    // A pointed option is looked for only where a question would be drawn. Above a live input
    // box is the transcript, where an echo of what the person once typed (`❯ 1. do this`)
    // looks the same; a real menu or question replaces the box, so with none on screen every
    // line counts. The box's own line is above its bottom border and is not looked at, so
    // someone typing `1. foo` is still typing.
    let pointed_from = input.as_ref().map_or(0, |b| b.bottom + 1);
    let asking = lines.iter().enumerate().any(|(row, l)| {
        let lower = l.plain.to_lowercase();
        (row >= pointed_from && is_pointed_option(&l.plain))
            || ASKING_HINTS.iter().any(|h| lower.contains(h))
            || hints.iter().any(|h| lower.contains(h))
    });
    if asking {
        return reading(PaneState::Asking);
    }
    let Some(input) = input else {
        return reading(PaneState::Unknown);
    };
    if !input.text.is_empty() {
        return Reading {
            state: PaneState::Typing,
            typed: input.text,
        };
    }
    if lines.iter().any(|l| working_marker(agent, &l.plain)) {
        return reading(PaneState::Working);
    }
    reading(PaneState::Idle)
}

/// The longest line `last_output_line` returns, in characters.
pub(super) const LAST_LINE_CHARS: usize = 200;

/// A line indented at least this far is the agent's right-aligned chrome (the effort level, a
/// key hint), not something it wrote. This can drop a line of output that really is indented
/// that deep; that is traded for not showing Claude's hints as the last thing it said.
const RIGHT_ALIGNED_INDENT: usize = 24;

/// The footer a menu or a question draws to say which keys answer it.
fn is_key_hint(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    let hint = |s: &str| s.starts_with("enter to select") || s.starts_with("esc to cancel");
    // Either the hint itself, or one of several joined by the dot a footer separates them with;
    // a sentence that only mentions the keys is output.
    hint(&lower) || (lower.contains('·') && lower.split('·').any(|part| hint(part.trim())))
}

/// Lines an agent draws between its output and its input box that are not output: hints about
/// itself and about the terminal it found itself in.
fn is_chrome(plain: &str) -> bool {
    let line = plain.trim();
    let indent = plain.chars().take_while(|c| c.is_whitespace()).count();
    indent >= RIGHT_ALIGNED_INDENT
        || line
            .strip_prefix('⎿')
            .is_some_and(|rest| rest.trim_start().starts_with("Tip:"))
        || line.starts_with("tmux detected")
        || is_key_hint(line)
}

/// The last thing the agent wrote, as one line of what its screen shows: the last line above
/// the input box that has anything in it, without the escapes and the agent's own chrome, cut
/// to `LAST_LINE_CHARS`. A screen with no input box (a generic agent's, or one showing a menu)
/// gives its last line. `None` for a blank screen.
pub fn last_output_line(agent: Agent, screen: &PaneScreen) -> Option<String> {
    let lines: Vec<Line> = screen
        .text
        .lines()
        .map(|raw| Line {
            plain: strip_escapes(raw),
            raw: raw.to_string(),
        })
        .collect();
    let end = match agent {
        Agent::Generic => lines.len(),
        _ => input_box(agent, &lines).map_or(lines.len(), |b| b.top),
    };
    let line = lines[..end]
        .iter()
        .rev()
        .map(|l| l.plain.as_str())
        .find(|l| !l.trim().is_empty() && !is_border(l) && !is_chrome(l))?
        .trim();
    let mut chars = line.chars();
    let cut: String = chars.by_ref().take(LAST_LINE_CHARS).collect();
    Some(if chars.next().is_some() {
        format!("{}…", cut.trim_end())
    } else {
        cut
    })
}

/// What the agent's screen says it is doing. `Generic` is `Idle` without looking: nothing is
/// known of how its screen reads, and it is how the wake behaved before it looked at all.
pub fn pane_state(agent: Agent, screen: &PaneScreen) -> PaneState {
    read_pane(agent, screen).state
}

/// Does the input box hold the line and nothing else? Whitespace is ignored because a line
/// the box is too narrow for is wrapped at a space that was not in it. Anything more in the
/// box is someone else's typing that the line has been appended to, and Enter would send both.
pub(super) fn shows_typed_line(agent: Agent, screen: &PaneScreen, line: &str) -> bool {
    let squeeze = |s: &str| s.split_whitespace().collect::<String>();
    let reading = read_pane(agent, screen);
    reading.state == PaneState::Typing && squeeze(&reading.typed) == squeeze(line)
}

/// How the built-in tmux wake reads `agent`'s pane before typing. `None` for `Generic`:
/// nothing is known of how its screen reads, so it is typed into without looking.
pub fn look_before_typing(agent: Agent) -> Option<LookBeforeTyping> {
    if agent == Agent::Generic {
        return None;
    }
    Some(LookBeforeTyping {
        may_type: Box::new(move |screen| match pane_state(agent, screen) {
            PaneState::Idle => Ok(()),
            state => Err(state.why_not_typed()),
        }),
        holds_just: Box::new(move |screen, line| shows_typed_line(agent, screen, line)),
    })
}
