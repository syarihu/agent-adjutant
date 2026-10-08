//! Static assets the board serves.

/// The page. One file, no build step, no network fetches — it is read from the binary and
/// runs from there. The source is kept in pieces under `src/ui/` only so it can be read; they
/// are joined here in order, so the browser still gets a single page and every request it
/// makes still carries the token. The scripts share one global scope, so their order matters.
pub(super) const UI_HTML: &str = concat!(
    include_str!("../../ui/page-head.html"),
    include_str!("../../ui/tokens.css"),
    include_str!("../../ui/components.css"),
    include_str!("../../ui/shell.css"),
    include_str!("../../ui/board.css"),
    include_str!("../../ui/review.css"),
    include_str!("../../ui/task-view.css"),
    include_str!("../../ui/console-and-dialog.css"),
    include_str!("../../ui/terminal.css"),
    include_str!("../../ui/sessions.css"),
    include_str!("../../ui/my-work.css"),
    include_str!("../../ui/page-body.html"),
    include_str!("../../ui/util.js"),
    include_str!("../../ui/core.js"),
    include_str!("../../ui/nav.js"),
    include_str!("../../ui/terminal.js"),
    include_str!("../../ui/board.js"),
    include_str!("../../ui/cards.js"),
    include_str!("../../ui/task-panel.js"),
    include_str!("../../ui/actions.js"),
    include_str!("../../ui/review.js"),
    include_str!("../../ui/decide.js"),
    include_str!("../../ui/task-view.js"),
    include_str!("../../ui/sessions.js"),
    include_str!("../../ui/session-actions.js"),
    include_str!("../../ui/sessions-side.js"),
    include_str!("../../ui/sessions-start.js"),
    include_str!("../../ui/my-work-seen.js"),
    include_str!("../../ui/my-work.js"),
    include_str!("../../ui/main.js"),
    include_str!("../../ui/page-end.html"),
);

/// The terminal the board opens on a tmux session, served only by the resident server and only
/// when a page asks for it: xterm.js and the two addons it is used with, as one script. The
/// license notice comes first, as the licenses ask for it to travel with the code (the files
/// themselves are documented in `src/ui/vendor/xterm/README.md`).
const XTERM_JS: &str = concat!(
    "/*! xterm.js - MIT License\n",
    include_str!("../../ui/vendor/xterm/LICENSE"),
    "\n@xterm/addon-fit and @xterm/addon-unicode11: Copyright (c) 2019, The xterm.js authors\n",
    "(https://github.com/xtermjs/xterm.js), under the same license.\n*/\n",
    include_str!("../../ui/vendor/xterm/xterm.js"),
    "\n",
    include_str!("../../ui/vendor/xterm/addon-fit.js"),
    "\n",
    include_str!("../../ui/vendor/xterm/addon-unicode11.js"),
);

const XTERM_CSS: &str = concat!(
    "/*! xterm.js - MIT License\n",
    include_str!("../../ui/vendor/xterm/LICENSE"),
    "*/\n",
    include_str!("../../ui/vendor/xterm/xterm.css"),
);

/// The scripts and styles the board terminal loads, as `(content type, body)`. A page fetches
/// them only when it opens a terminal.
pub(super) fn vendor_asset(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "/vendor/xterm.js" => Some(("text/javascript; charset=utf-8", XTERM_JS)),
        "/vendor/xterm.css" => Some(("text/css; charset=utf-8", XTERM_CSS)),
        _ => None,
    }
}
