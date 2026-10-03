use super::*;

// ── looking at a pane before typing into it ──────────────────────────
//
// A wake types a line and Enter into whatever the agent is showing. Enter answers a question
// the agent is asking, and a line typed while a person is halfway through a message is
// appended to theirs and sent. So the built-in tmux wake reads the pane first and types only
// at an empty prompt.
//
// What "an empty prompt" looks like is each agent's business, and the markers below were read
// off captures of the real thing (`src/fixtures/panes`). They are structural — a numbered
// list with a pointer, a key-hint line, a bordered input box — rather than the wording of
// one release, and a screen that matches none of them is not typed into.

/// The line `tmux_capture_script` prints between the screen and the pane's own state.
pub(crate) const PANE_META_SEPARATOR: &str = "@@adjutant:pane@@";

/// A capture of a pane: what is on it, and what tmux says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneScreen {
    /// One line per row, with the colour and attribute escapes `capture-pane -e` leaves in.
    pub text: String,
    /// Whether the pane is in a tmux mode (copy mode and the like).
    pub in_mode: bool,
    pub cursor_x: u32,
    pub cursor_y: u32,
}

/// The command that prints a pane's screen, then `PANE_META_SEPARATOR`, then
/// `pane_in_mode`, `cursor_x` and `cursor_y` separated by tabs.
///
/// `-u` for the reason `tmux_window_home_script` gives: without a UTF-8 locale tmux rewrites
/// every non-ASCII character it prints to `_`, and the prompt glyph is one. `-e` keeps the
/// attributes, because the only thing telling an agent's placeholder text from typed text is
/// that the placeholder is drawn faint. `-J` joins rows the terminal wrapped.
pub fn tmux_capture_script(socket: Option<&str>, pane_id: &str) -> String {
    let prefix = tmux_cmd_prefix(socket).replacen("tmux", "tmux -u", 1);
    let pane_q = sh_quote(pane_id);
    format!(
        "{prefix} capture-pane -p -e -J -t {pane_q} && echo {PANE_META_SEPARATOR} && {prefix} display-message -p -t {pane_q} '#{{pane_in_mode}}\t#{{cursor_x}}\t#{{cursor_y}}'"
    )
}

pub fn parse_pane_screen(output: &str) -> PaneScreen {
    let (text, meta) = match output.rsplit_once(PANE_META_SEPARATOR) {
        Some((text, meta)) => (text, meta.trim()),
        None => (output, ""),
    };
    let mut fields = meta.split('\t');
    let mut next = || fields.next().and_then(|f| f.trim().parse::<u32>().ok());
    PaneScreen {
        text: text.trim_end().to_string(),
        in_mode: next().is_some_and(|n| n != 0),
        cursor_x: next().unwrap_or(0),
        cursor_y: next().unwrap_or(0),
    }
}

/// A tmux pane's screen, or `None` when tmux cannot show it (the pane is gone, no server).
pub fn look_at_tmux_pane(socket: Option<&str>, pane_id: &str) -> Option<PaneScreen> {
    look_at_pane(&run_shell, &tmux_capture_script(socket, pane_id)).ok()
}

pub(super) fn look_at_pane(
    run: &impl Fn(&str) -> Result<String, String>,
    capture: &str,
) -> Result<PaneScreen, String> {
    run(capture).map(|out| parse_pane_screen(&out))
}
