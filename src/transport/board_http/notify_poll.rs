//! What a poll says about who is open to ring notifications, and what it is handed back.
//!
//! The page's poll is how the server learns that a client is open (`X-Adjutant-Notify: 1`, and
//! `X-Adjutant-Shell: app` from the desktop app), and how a queued notice reaches that client.
//! The answers go in headers rather than in the body: `/api/boards` is a bare array and
//! `/api/state` is compared as a whole to decide whether to redraw, and a notice is one time
//! news that must not make it look as if the state changed.

use std::path::Path;

use crate::infra::http::Request;
use crate::registry::{Client, Notice, claim_notices, note_notify_client, notify_clients};

use super::auth::page_rings;

/// Header the answer carries: `1` when the desktop app is open, else `0`.
pub(super) const APP_OPEN_HEADER: &str = "X-Adjutant-App-Open";
/// Header the answer carries when it hands over notices: a JSON array, ASCII only.
pub(super) const NOTICES_HEADER: &str = "X-Adjutant-Notices";

/// Header a board served alone answers its state with: its own slug, which its page cannot read
/// from anywhere else. A slug is lowercase letters, digits and `-`, so it is a valid header value.
pub(super) const BOARD_SLUG_HEADER: &str = "X-Adjutant-Board-Slug";

/// Header the answer carries when some boards have a notifier of their own: a JSON array of
/// their slugs, ASCII only.
pub(super) const OWN_NOTIFIER_HEADER: &str = "X-Adjutant-Own-Notifier";

/// The most notices one poll takes, and how much of each it keeps: they travel in a header.
const MAX_NOTICES: usize = 3;
const TITLE_CHARS: usize = 100;
const MESSAGE_CHARS: usize = 200;

/// Which client sent `req`, when it can show notifications at all, and whether it also rings
/// waits and gates.
///
/// A client can show notifications when it says so (`X-Adjutant-Can-Notify: 1`: the browser's
/// permission is granted, which in the app the shim decides). A page older than that header says
/// it with `X-Adjutant-Notify: 1`, which implies it. `X-Adjutant-Notify: 1` is the narrower
/// answer, can show *and* 確認待ち is on, and is what waits and gates follow. The desktop app is
/// the app only by `X-Adjutant-Shell: app`: another shell is a page. A client that cannot show
/// notifications is neither, so it silences nobody.
pub(super) fn notify_client(req: &Request) -> Option<(Client, bool)> {
    let waits = page_rings(req);
    if !waits && req.header("x-adjutant-can-notify") != Some("1") {
        return None;
    }
    let client = if req.header("x-adjutant-shell") == Some("app") {
        Client::App
    } else {
        Client::Page
    };
    Some((client, waits))
}

/// Which of these boards keep a notifier of their own: `notification` is a command, or `false`.
/// Their waits and gates are rung (or not) exactly as before, whether or not the app is open, so
/// the page does not hold its rings for them back. Each repository's config is read once.
/// `boards` are `(slug, nwo)`.
pub(super) fn own_notifier<'a>(
    boards: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<String> {
    let mut by_nwo: std::collections::HashMap<&str, bool> = std::collections::HashMap::new();
    boards
        .into_iter()
        .filter(|(_, nwo)| !*by_nwo.entry(nwo).or_insert_with(|| built_in(nwo)))
        .map(|(slug, _)| slug.to_string())
        .collect()
}

fn built_in(nwo: &str) -> bool {
    crate::kernel::config::resolve_config(nwo)
        .map(|resolved| resolved.settings.notification)
        .is_ok_and(|hook| matches!(hook, crate::infra::terminal::Hook::BuiltIn))
}

/// Note this poll, and answer it: whether the app is open (the latest of every server process),
/// and the notices it takes. A notice goes to the app when the app is open, and otherwise to a
/// page that can ring; a request that cannot ring takes none. Only a repository whose
/// `notification` is unset ever queues a notice, so what is taken is not narrowed by the other
/// repositories' settings. `own_notifier` are the board slugs that have a notifier of their own
/// (`own_notifier`, `""` for a board served alone): the page does not hold its rings for those
/// back while the app is open.
pub(super) fn poll_headers(
    root: &Path,
    req: &Request,
    now: i64,
    own_notifier: &[String],
) -> Vec<(&'static str, String)> {
    let seen = notify_client(req);
    let client = seen.map(|(client, _)| client);
    if let Some((client, waits)) = seen {
        note_notify_client(root, client, waits, now);
    }
    let app_open = notify_clients(root).app_fresh(now);
    let mut headers = vec![(
        APP_OPEN_HEADER,
        if app_open { "1" } else { "0" }.to_string(),
    )];
    if !own_notifier.is_empty() {
        let list = serde_json::to_string(own_notifier).unwrap_or_else(|_| "[]".to_string());
        headers.push((OWN_NOTIFIER_HEADER, ascii_only(&list)));
    }
    let takes = match client {
        Some(Client::App) => true,
        Some(Client::Page) => !app_open,
        None => false,
    };
    if takes {
        let notices = claim_notices(root, MAX_NOTICES);
        if !notices.is_empty() {
            headers.push((NOTICES_HEADER, notices_header(&notices)));
        }
    }
    headers
}

/// The notices as the JSON the page reads, with every character beyond ASCII written as an
/// escape: a header value is not for raw text, and the page's `JSON.parse` reads them back.
fn notices_header(notices: &[Notice]) -> String {
    let shown: Vec<serde_json::Value> = notices
        .iter()
        .map(|n| {
            let mut shown = serde_json::json!({
                "id": n.id,
                "title": n.title.chars().take(TITLE_CHARS).collect::<String>(),
                "message": n.message.chars().take(MESSAGE_CHARS).collect::<String>(),
                "nwo": n.nwo,
            });
            // What the page rings the same event under (`ringWaiting`), so it shows it once.
            if let Some(tag) = &n.tag {
                shown["tag"] = serde_json::json!(tag);
            }
            shown
        })
        .collect();
    ascii_only(&serde_json::to_string(&shown).unwrap_or_else(|_| "[]".to_string()))
}

/// JSON text with every character beyond ASCII (and DEL) written as an escape.
fn ascii_only(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii() && c != '\u{7f}' {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

/// A JSON answer with the poll's headers.
pub(super) fn json_with(
    out: &mut impl std::io::Write,
    body: &str,
    headers: &[(&'static str, String)],
) -> std::io::Result<()> {
    let extra: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    crate::infra::http::respond_with(
        out,
        200,
        "application/json; charset=utf-8",
        &extra,
        body.as_bytes(),
    )
}

#[cfg(test)]
mod tests;
