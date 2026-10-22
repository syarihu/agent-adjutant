//! Serving the HTML report a gate carries, to be shown in a sandboxed frame on the board.

use std::io::Write;

use serde_json::json;

use crate::board::Server;
use crate::gate::report;
use crate::infra::http::{self, Request};

/// The policy a report is served, and shown, under. The report is self-contained, so nothing
/// is fetched from anywhere: not from the board (`connect-src` falls back to `'none'`) and not
/// from the internet. Inline scripts and styles are allowed because a self-contained report
/// has no other way to draw a chart. The page carries the same string (`REPORT_CSP`), as the
/// frame it loads the report into does not see this header.
pub(super) const REPORT_POLICY: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; form-action 'none'; base-uri 'none'";

/// `GET /api/gates/<id>/report`.
pub(super) fn serve_report(
    server: &Server,
    id: &str,
    req: &Request,
    out: &mut impl Write,
) -> std::io::Result<()> {
    // The token in a query string is refused here, though every other route takes it. A tab
    // opened on this URL would hold the token in its own address, where the report's scripts
    // can read it. The page fetches the report with the header and never puts the token in a
    // frame's address.
    if req.header("X-Adjutant-Token").is_none() {
        return http::json(
            out,
            403,
            &json!({ "error": "a report is read through the board page" }).to_string(),
        );
    }
    if !report::valid_id(id) {
        return no_such_report(out);
    }
    match report::load(&server.ctx.state, &server.ctx.repo.slug, id) {
        Ok(None) => no_such_report(out),
        Err(e) => http::json(out, 500, &json!({ "error": e }).to_string()),
        Ok(Some(html)) => {
            let policy = format!("{REPORT_POLICY}; sandbox allow-scripts");
            http::respond_with(
                out,
                200,
                "text/html; charset=utf-8",
                &[
                    ("Content-Security-Policy", &policy),
                    ("Referrer-Policy", "no-referrer"),
                ],
                html.as_bytes(),
            )
        }
    }
}

fn no_such_report(out: &mut impl Write) -> std::io::Result<()> {
    http::json(out, 404, &json!({ "error": "no such report" }).to_string())
}
