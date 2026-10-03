use super::*;

// ── shared ───────────────────────────────────────────────────────────

/// A value given as `-` is read from stdin, trailing newlines dropped.
///
/// For text that came from somewhere else — a task's title, the reason a start failed, a
/// comment typed on the board. On a command line it has to be quoted, and whatever quote is
/// chosen, the text can close it and run the rest as shell. Written to a file by the agent's
/// file tool and redirected in, it never passes through the shell at all.
pub(super) fn dash_is_stdin(value: &str) -> Result<String, String> {
    if value != "-" {
        return Ok(value.to_string());
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("cannot read stdin: {e}"))?;
    Ok(buf.trim_end_matches(['\n', '\r']).to_string())
}

/// The message body, from the flag or from stdin. Long reports do not belong on a command
/// line, and the two ways in should behave identically.
pub(super) fn read_body(body: Option<&str>) -> Result<String, String> {
    let body = match body {
        // `-` is stdin, as `task add --help` says and as most tools read it. Taken literally
        // it became a body of one dash, and the card's title with it.
        Some(body) if body != "-" => body.to_string(),
        _ => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| format!("cannot read stdin: {e}"))?;
            buf
        }
    };
    if body.trim().is_empty() {
        return Err("the body is empty: pass it with --body or on stdin".to_string());
    }
    Ok(body)
}
