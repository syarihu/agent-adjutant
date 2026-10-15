//! Asking the resident server a question over its own HTTP, from a process that is not it.

use std::path::Path;
use std::time::Duration;

/// Longer than the page waits: the first answer of the work list asks GitHub, `ps` and git for
/// every repository before it is cached.
const ASK_TIMEOUT: Duration = Duration::from_secs(30);

/// The JSON `GET path` of the resident server answers with.
///
/// The resident is found through its record and the token it was started with, both under the
/// state root, so this works from any directory. Without a live one the answer says how to start
/// it; a refusal passes on the `error` the server gave, or its status.
pub fn ask_resident(root: &Path, path: &str) -> Result<serde_json::Value, String> {
    let (_, port) = crate::registry::live_resident(root)
        .ok_or("the resident server is not running: start it with `adj server start`")?;
    let token = super::stored_token(root).ok_or("the resident server's token cannot be read")?;
    let (status, body) = crate::infra::http::get(port, path, &token, ASK_TIMEOUT)?;
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&body);
    if (200..300).contains(&status) {
        return parsed.map_err(|e| format!("the resident server's answer is not JSON: {e}"));
    }
    Err(parsed
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str().map(str::to_string)))
        .unwrap_or_else(|| format!("the resident server answered {status}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_resident_the_answer_says_how_to_start_one() {
        let sandbox = crate::testing::Sandbox::empty();
        let err = ask_resident(&sandbox.state(), "/api/work").unwrap_err();
        assert!(err.contains("adj server start"), "{err}");
    }
}
