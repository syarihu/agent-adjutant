//! Reading what an agent's screen shows and whether it is safe to type into.
use serde::{Deserialize, Serialize};

use crate::infra::agent::Agent;
use crate::infra::terminal::*;

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

/// The mode line Claude draws under the input box (`⏵⏵ auto mode on (shift+tab to cycle)`,
/// `⏸ manual mode on`): the footer that carries `esc to interrupt` while a turn runs.
fn claude_mode_line(line: &str) -> bool {
    line.trim_start().starts_with(['⏵', '⏸'])
}

/// The footer of a turn that is running: `esc to interrupt` after the mode. The footer is cut
/// to the pane's width with an ellipsis, so in a narrow pane it reads `esc to i…`, and the
/// fullscreen UI draws no spinner line while the reply streams: the footer is then the only
/// sign of the turn. A cut that is a prefix of the words (three characters or more) counts,
/// but only as the last segment of a mode line, after a `·`. `e…`, `es…` and a bare `…` are
/// left out on purpose: too short to tell from the cut `← for agents` hint or from nothing at
/// all, and a turn taken for idle there is the lesser harm than an idle one taken for running.
///
/// Only the footer's own row says so: a transcript line that reads like a cut footer (a diff
/// or tool output quoting one) is not a turn, so callers look at the footer's row alone.
fn claude_running_footer(line: &str) -> bool {
    line.to_lowercase().contains("esc to interrupt") || claude_cut_running_footer(line)
}

/// The cut forms of `claude_running_footer`: a mode line ending in a prefix of the words.
fn claude_cut_running_footer(line: &str) -> bool {
    if !claude_mode_line(line) {
        return false;
    }
    let mut segments = line.rsplit('·');
    let last = segments.next().unwrap_or_default().trim();
    if segments.next().is_none() {
        return false;
    }
    let Some(cut) = last.strip_suffix('…') else {
        return false;
    };
    let cut = cut.trim().to_lowercase();
    cut.chars().count() >= 3 && "esc to interrupt".starts_with(&cut)
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

/// The top edge of the input box: a rule, or one with the session's name drawn into it
/// (`────── adjutant-read-x ─`, from `claude -n`). The name takes room from the rule, so in a
/// narrow pane too few dashes are left for `is_border` (or none before the name). Whatever
/// ends in ` ─` after some text is taken for it; the box still needs its prompt line right
/// below and its full bottom rule, so a transcript line that ends so does not make one.
fn is_box_top(plain: &str) -> bool {
    let line = plain.trim();
    is_border(plain) || (line.ends_with(" ─") && !line.trim_end_matches('─').trim_end().is_empty())
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
/// above it (which may carry the session's name) and, further down, another one.
fn input_box(agent: Agent, lines: &[Line]) -> Option<InputBox> {
    let glyph = prompt_glyph(agent);
    for top in (0..lines.len().saturating_sub(1)).rev() {
        if !is_box_top(&lines[top].plain) || !lines[top + 1].plain.trim_start().starts_with(glyph) {
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
        // The box is not found when the screen is not the agent's, but also when it is drawn in
        // a way not known; a running footer is still read, so the turn is not typed into. It
        // is looked for after the questions above, which win, and only on the bottom line,
        // where the footer is drawn, as the mode line it is (output that only mentions the
        // keys is not one).
        let bottom_line = lines.iter().rev().find(|l| !l.plain.trim().is_empty());
        if agent == Agent::Claude
            && bottom_line
                .is_some_and(|l| claude_mode_line(&l.plain) && claude_running_footer(&l.plain))
        {
            return reading(PaneState::Working);
        }
        return reading(PaneState::Unknown);
    };
    if !input.text.is_empty() {
        return Reading {
            state: PaneState::Typing,
            typed: input.text,
        };
    }
    // The cut footer counts only under the box, where the footer is drawn; the spinner and the
    // whole words count anywhere, as they always did.
    let footer_cut = agent == Agent::Claude
        && lines[input.bottom + 1..]
            .iter()
            .any(|l| claude_cut_running_footer(&l.plain));
    if footer_cut || lines.iter().any(|l| working_marker(agent, &l.plain)) {
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
        || claude_mode_line(line)
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

/// The most rows a dialog's directory is read over: each break doubles what it can stand for.
const MAX_PATH_ROWS: usize = 4;

/// The question an agent asks, before it does anything, about the directory it was started in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustDialog {
    /// The directory the dialog names, as it is drawn (a home directory may be written `~/`),
    /// a wrapped path joined without a separator: for telling a person which directory it was.
    pub dir: String,
    /// Every directory the drawn rows can stand for, which is all that may be compared. A path
    /// the agent's layout wrapped has lost what was at the break: a space, or nothing inside a
    /// word. So each break is read both ways, and a one-row path has the one spelling.
    pub spellings: Vec<String>,
    /// Whether the pointer is already on "Yes, I trust this folder" (agy's starts there,
    /// Claude Code's starts on "No, exit").
    pub yes_pointed: bool,
}

/// The folder-trust dialog `agent` shows on a directory it has not been told to trust, with the
/// directory it names. `None` for any other screen, and for a dialog that names no directory:
/// what cannot be checked is not answered. `Generic` is never read.
///
/// Both the dialog's own question and the `Accessing workspace:` line it puts above the
/// directory are required, so a transcript that merely quotes one of them is not taken for it.
pub fn trust_dialog(agent: Agent, screen: &PaneScreen) -> Option<TrustDialog> {
    let question = match agent {
        Agent::Claude => "Is this a project you created or one you trust",
        Agent::Agy => "Do you trust the contents of this project",
        Agent::Generic => return None,
    };
    let plain: Vec<String> = screen.text.lines().map(strip_escapes).collect();
    // The agent wraps its own text in a narrow pane as it does the path, so the question is looked
    // for across lines with the whitespace left out.
    let squeeze = |s: &str| s.split_whitespace().collect::<String>();
    if !squeeze(&plain.join(" ")).contains(&squeeze(question)) {
        return None;
    }
    let at = plain
        .iter()
        .position(|l| l.trim() == "Accessing workspace:")?;
    // A long path is wrapped by the agent's own layout, which `capture-pane -J` cannot join, so
    // the path is the run of rows up to the next blank one. More rows than `MAX_PATH_ROWS` is
    // not read: it cannot be told which directory that is.
    let rows: Vec<&str> = plain[at + 1..]
        .iter()
        .map(|l| l.trim())
        .skip_while(|l| l.is_empty())
        .take_while(|l| !l.is_empty())
        .collect();
    if rows.is_empty() || rows.len() > MAX_PATH_ROWS {
        return None;
    }
    let dir = rows.concat();
    let mut spellings = vec![rows[0].to_string()];
    for row in &rows[1..] {
        spellings = spellings
            .iter()
            .flat_map(|s| [format!("{s}{row}"), format!("{s} {row}")])
            .collect();
    }
    let yes_pointed = plain.iter().any(|l| {
        let line = l.trim();
        (line.starts_with('❯') || line.starts_with('>'))
            && line.contains("Yes, I trust this folder")
    });
    Some(TrustDialog {
        dir,
        spellings,
        yes_pointed,
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

/// Permission (a yes/no-shaped tool approval) or Choice (any other numbered list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptKind {
    Permission,
    Choice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptOption {
    /// A single digit ("1".."9").
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanePrompt {
    /// Permission (a yes/no-shaped tool approval) or Choice (any other numbered list).
    pub kind: PromptKind,
    /// The question line as drawn, e.g. "Do you want to proceed?", "Which tea do you prefer?".
    pub question: Option<String>,
    /// For a permission prompt: up to 3 non-blank lines between the prompt's top rule and the
    /// question (e.g. "Bash command", "touch probe.txt"), without `Tip:` lines. Empty otherwise allowed.
    pub detail: Vec<String>,
    /// The options a press may answer, in screen order. Empty when they cannot be read reliably.
    pub options: Vec<PromptOption>,
    /// A fingerprint of what is on screen: a short digest of kind, question, detail and *every*
    /// numbered option label (including the ones left out of `options`) and which one is pointed.
    pub id: String,
}

/// Why a key is not sent to a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptRefusal {
    /// No prompt on screen now, or not the one the board saw.
    Gone,
    /// The key is not one of the prompt's options.
    NotAnOption,
    /// The prompt's options could not be read.
    Unreadable,
}

fn is_typing_option(label: &str) -> bool {
    let lower = label.trim().to_lowercase();
    lower.starts_with("type something")
        || lower.starts_with("write-in")
        || lower.starts_with("chat about this")
}

#[derive(Debug, Clone)]
struct RawOption {
    line_idx: usize,
    pointed: bool,
    number: usize,
    label: String,
    has_checkbox: bool,
}

fn parse_option_line(line: &str) -> Option<(bool, usize, String, bool)> {
    let trimmed = line.trim_start();
    let (pointed, rest) = if let Some(after) = trimmed.strip_prefix(['❯', '>', '›']) {
        (true, after.trim_start())
    } else {
        (false, trimmed)
    };
    let digits_len = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits_len == 0 {
        return None;
    }
    let num_str = &rest[..digits_len];
    let after_num = &rest[digits_len..];
    let after_delim = after_num.strip_prefix(['.', ')'])?;
    if !after_delim.is_empty() && !after_delim.starts_with(char::is_whitespace) {
        return None;
    }
    let number: usize = num_str.parse().ok()?;
    let label = after_delim.trim().to_string();
    let has_checkbox = line.contains('☐')
        || line.contains('☒')
        || line.contains("[ ]")
        || line.contains("[x]")
        || line.contains("[X]");
    Some((pointed, number, label, has_checkbox))
}

fn cut_line(line: &str) -> String {
    let mut chars = line.chars();
    let cut: String = chars.by_ref().take(LAST_LINE_CHARS).collect();
    if chars.next().is_some() {
        format!("{}…", cut.trim_end())
    } else {
        cut
    }
}

/// The question a session asks in its terminal alone (a permission prompt, a numbered choice
/// list), as far as the screen tells it. `None` unless the screen is asking, and for the
/// folder-trust dialog, which is answered elsewhere. Options that cannot be read reliably (a
/// pane cut short, a multi-select list) leave `options` empty: what was not read is not
/// offered as an answer.
pub fn pane_prompt(agent: Agent, screen: &PaneScreen) -> Option<PanePrompt> {
    if agent == Agent::Generic
        || pane_state(agent, screen) != PaneState::Asking
        || trust_dialog(agent, screen).is_some()
    {
        return None;
    }
    let lines: Vec<Line> = screen
        .text
        .lines()
        .map(|raw| Line {
            plain: strip_escapes(raw),
            raw: raw.to_string(),
        })
        .collect();
    let input = input_box(agent, &lines);
    let start_idx = input.as_ref().map_or(0, |b| b.bottom + 1);
    let search_lines = &lines[start_idx..];

    let mut candidates: Vec<RawOption> = Vec::new();
    for (idx, line) in search_lines.iter().enumerate() {
        if let Some((pointed, number, label, has_checkbox)) = parse_option_line(&line.plain) {
            candidates.push(RawOption {
                line_idx: idx,
                pointed,
                number,
                label,
                has_checkbox,
            });
        }
    }
    let last = candidates.last()?;
    let mut block = vec![last.clone()];
    let mut prev_num = last.number;
    let mut prev_idx = last.line_idx;

    for cand in candidates.iter().rev().skip(1) {
        if cand.line_idx >= prev_idx {
            continue;
        }
        let between_valid = search_lines[cand.line_idx + 1..prev_idx].iter().all(|l| {
            let plain = l.plain.trim();
            plain.is_empty()
                || is_border(plain)
                || l.plain.chars().take_while(|c| c.is_whitespace()).count() >= 2
        });
        if !between_valid {
            break;
        }
        if cand.number + 1 == prev_num {
            block.push(cand.clone());
            prev_num = cand.number;
            prev_idx = cand.line_idx;
            if cand.number == 1 {
                break;
            }
        } else {
            break;
        }
    }
    block.reverse();

    let first_opt_idx = block[0].line_idx;
    let mut question = None;
    let mut q_idx = None;
    for (i, line) in search_lines[..first_opt_idx].iter().enumerate().rev() {
        let plain = line.plain.trim();
        if plain.is_empty()
            || is_key_hint(plain)
            || plain.starts_with('☐')
            || plain.starts_with('☒')
        {
            continue;
        }
        if is_border(plain) {
            break;
        }
        let q_text = plain.strip_prefix("? ").unwrap_or(plain).trim();
        question = Some(cut_line(q_text));
        q_idx = Some(i);
        break;
    }

    let first_yes = block.first().is_some_and(|o| o.label.starts_with("Yes"));
    let last_no = block.last().is_some_and(|o| o.label.starts_with("No"));
    let kind = if first_yes && last_no {
        PromptKind::Permission
    } else {
        PromptKind::Choice
    };

    let detail = if kind == PromptKind::Permission {
        if let Some(qi) = q_idx {
            let top_rule = search_lines[..qi]
                .iter()
                .enumerate()
                .rposition(|(_, l)| is_border(&l.plain));
            if let Some(ti) = top_rule {
                search_lines[ti + 1..qi]
                    .iter()
                    .map(|l| l.plain.trim())
                    .filter(|s| !s.is_empty() && !s.starts_with("Tip:"))
                    .take(3)
                    .map(cut_line)
                    .collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let has_at_least_two = block.len() >= 2;
    let numbers_sequential = block.first().is_some_and(|o| o.number == 1)
        && block.windows(2).all(|w| w[1].number == w[0].number + 1);
    let exactly_one_pointed = block.iter().filter(|o| o.pointed).count() == 1;
    let has_question = question.is_some();
    let has_multi_select = block.iter().any(|o| o.has_checkbox);

    let is_reliable = has_at_least_two
        && numbers_sequential
        && exactly_one_pointed
        && has_question
        && !has_multi_select
        // One key per option: an option numbered 10 would be typed as `1` then `0`, and the `1`
        // alone already answers.
        && block.iter().all(|o| o.number <= 9);

    let options = if is_reliable {
        block
            .iter()
            .filter(|o| !is_typing_option(&o.label))
            .map(|o| PromptOption {
                key: o.number.to_string(),
                label: o.label.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut fingerprint = String::new();
    fingerprint.push_str(match kind {
        PromptKind::Permission => "permission\n",
        PromptKind::Choice => "choice\n",
    });
    fingerprint.push_str(question.as_deref().unwrap_or(""));
    fingerprint.push('\n');
    for d in &detail {
        fingerprint.push_str(d);
        fingerprint.push('\n');
    }
    fingerprint.push_str("---\n");
    for opt in &block {
        fingerprint.push_str(if opt.pointed { ">" } else { " " });
        fingerprint.push_str(&opt.number.to_string());
        fingerprint.push(':');
        fingerprint.push_str(&opt.label);
        fingerprint.push('\n');
    }
    let id = crate::kernel::identity::short_digest(&fingerprint);

    Some(PanePrompt {
        kind,
        question,
        detail,
        options,
        id,
    })
}

/// Whether `key` may be sent to answer the prompt the board saw as `seen_id`, on this screen.
pub fn prompt_answerable(
    agent: Agent,
    screen: &PaneScreen,
    seen_id: &str,
    key: &str,
) -> Result<(), PromptRefusal> {
    let Some(prompt) = pane_prompt(agent, screen) else {
        return Err(PromptRefusal::Gone);
    };
    if prompt.id != seen_id {
        return Err(PromptRefusal::Gone);
    }
    if prompt.options.is_empty() {
        return Err(PromptRefusal::Unreadable);
    }
    if !prompt.options.iter().any(|opt| opt.key == key) {
        return Err(PromptRefusal::NotAnOption);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
