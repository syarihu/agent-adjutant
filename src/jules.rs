//! Talking to the Jules API: start a session from an approved plan, and ask how it is doing.
//!
//! Through `curl` rather than an HTTP client crate, the way pull requests are asked about
//! through `gh`: the binary stays three dependencies deep, and what it runs is a command
//! anybody can run by hand to see the same answer.
//!
//! The key is the one thing here that must not travel. It comes from a command (`julesKey`)
//! at the moment of the call, goes to `curl` on its stdin — an argument would be readable by
//! every process on the machine through `ps` — and is taken out of anything this module says
//! back, errors included, before that text can reach an agent's transcript.

use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};

use crate::config::Hook;

/// Where the API lives. A constant rather than a setting: there is one Jules.
pub const API: &str = "https://jules.googleapis.com/v1alpha";

/// The keychain item the built-in `julesKey` reads, as `security add-generic-password -s`
/// named it.
pub const KEYCHAIN_SERVICE: &str = "jules-api";

/// How long one call may take. The board asks while a person is looking at it, and a network
/// that has gone away should cost a few seconds of a stale badge, not a hung page.
const TIMEOUT_SECS: &str = "20";

/// One session, as much of it as adjutant reads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    /// `QUEUED`, `PLANNING`, `IN_PROGRESS`, `COMPLETED`, … as the API spells them. Kept as
    /// text: a state added later should reach the board as itself rather than fail to parse.
    pub state: String,
    /// The session's page on jules.google.com.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The pull request the session opened, once it has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
}

/// The source name the API knows a GitHub repository by.
pub fn source_name(nwo: &str) -> String {
    format!("sources/github/{nwo}")
}

/// The body of a create call. Plans are auto-approved: the plan a person approved is the
/// prompt, and asking them to approve Jules' restatement of it would be the same question
/// twice. The pull request is opened by Jules as soon as the change is ready.
pub fn create_body(nwo: &str, base: &str, title: &str, prompt: &str) -> Value {
    json!({
        "prompt": prompt,
        "title": title,
        "sourceContext": {
            "source": source_name(nwo),
            "githubRepoContext": { "startingBranch": base },
        },
        "automationMode": "AUTO_CREATE_PR",
        "requirePlanApproval": false,
    })
}

/// Read a session out of what the API returned for it.
pub fn parse_session(value: &Value) -> Result<Session, String> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let id = text("id").ok_or_else(|| format!("no session id in the answer: {value}"))?;
    let pr = value
        .get("outputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|output| output.pointer("/pullRequest/url").and_then(Value::as_str))
        .map(str::to_string);
    Ok(Session {
        id,
        // A session that has just been created comes back without one.
        state: text("state").unwrap_or_else(|| "QUEUED".to_string()),
        url: text("url"),
        title: text("title"),
        pr,
    })
}

/// A session id, as it goes into a URL path. The API's ids are digits; anything else is
/// refused here rather than spliced into a path it could walk out of.
fn check_id(id: &str) -> Result<(), String> {
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(format!("not a Jules session id: {id}"))
    }
}

/// Start a session.
pub fn create(
    key: &Hook,
    nwo: &str,
    base: &str,
    title: &str,
    prompt: &str,
) -> Result<Session, String> {
    let answer = call(
        key,
        "POST",
        "sessions",
        Some(&create_body(nwo, base, title, prompt)),
    )?;
    parse_session(&answer)
}

/// Ask how a session is doing.
pub fn get(key: &Hook, id: &str) -> Result<Session, String> {
    check_id(id)?;
    parse_session(&call(key, "GET", &format!("sessions/{id}"), None)?)
}

/// The key, from the command `julesKey` names.
fn read_key(key: &Hook) -> Result<String, String> {
    let mut command = match key {
        Hook::Off => {
            return Err("julesKey is false: handing tasks to Jules is turned off".to_string());
        }
        Hook::Command(template) => {
            let mut command = Command::new("sh");
            command.args(["-c", template]);
            command
        }
        Hook::BuiltIn if cfg!(target_os = "macos") => {
            let mut command = Command::new("security");
            command.args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"]);
            command
        }
        Hook::BuiltIn => {
            return Err(
                "no julesKey is configured, and there is no keychain to read one from here: \
                 set julesKey to a command that prints the key"
                    .to_string(),
            );
        }
    };
    // Its stderr is dropped rather than passed on: a helper that fails loudly may say more
    // than it should, and nobody reading the error needs more than that it failed.
    let out = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run julesKey: {e}"))?;
    if !out.status.success() {
        return Err(match key {
            Hook::BuiltIn => format!(
                "the keychain has no {KEYCHAIN_SERVICE} item, or would not hand it over: add one \
                 with `security add-generic-password -s {KEYCHAIN_SERVICE} -a \"$USER\" -w`"
            ),
            _ => format!("julesKey exited with {}", out.status),
        });
    }
    let text = String::from_utf8(out.stdout).map_err(|_| "julesKey printed non-UTF-8")?;
    let key = text.trim();
    if key.is_empty() || key.contains(['\r', '\n']) {
        return Err("julesKey printed no key, or more than one line".to_string());
    }
    Ok(key.to_string())
}

/// One request. `curl` is told to append the status code, so an error answer can be told
/// from a good one without `--fail`, which would throw away the API's own reason.
fn call(key: &Hook, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
    let secret = read_key(key)?;
    let redact = |text: &str| text.replace(&secret, "[redacted]");
    let mut command = Command::new("curl");
    command.args([
        "-sS",
        "--max-time",
        TIMEOUT_SECS,
        "-X",
        method,
        "-H",
        "@-",
        "-H",
        "Content-Type: application/json",
        "-w",
        "\n%{http_code}",
    ]);
    if let Some(body) = body {
        command.args(["--data-binary", &body.to_string()]);
    }
    command.arg(format!("{API}/{path}"));
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    {
        let mut stdin = child.stdin.take().ok_or("curl has no stdin")?;
        stdin
            .write_all(format!("x-goog-api-key: {secret}\n").as_bytes())
            .map_err(|e| format!("cannot hand curl the key: {e}"))?;
        // Dropped here, which closes it: curl reads headers from stdin until it ends.
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("cannot wait for curl: {e}"))?;
    let stdout = redact(&String::from_utf8_lossy(&out.stdout));
    if !out.status.success() {
        let said = redact(String::from_utf8_lossy(&out.stderr).trim());
        return Err(if said.is_empty() {
            format!("curl exited with {}", out.status)
        } else {
            format!("curl: {said}")
        });
    }
    let (payload, status) = stdout.rsplit_once('\n').unwrap_or(("", stdout.as_str()));
    let status: u16 = status.trim().parse().unwrap_or(0);
    let value: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let reason = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| payload.trim().chars().take(200).collect());
        return Err(format!("Jules answered {status}: {reason}"));
    }
    if value.is_null() {
        return Err(format!(
            "Jules answered with something that is not JSON: {payload}"
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_create_body_asks_for_a_pull_request_and_no_plan_approval() {
        let body = create_body("acme/widget", "main", "Add a thing", "Do it like this");
        assert_eq!(
            body["sourceContext"]["source"],
            "sources/github/acme/widget"
        );
        assert_eq!(
            body["sourceContext"]["githubRepoContext"]["startingBranch"],
            "main"
        );
        assert_eq!(body["automationMode"], "AUTO_CREATE_PR");
        assert_eq!(body["requirePlanApproval"], false);
        assert_eq!(body["prompt"], "Do it like this");
    }

    #[test]
    fn a_session_is_read_with_its_pull_request() {
        // Bound to a name rather than written as a value: an upper-case JSON value is the
        // shape the tracker-key guard in prompts.rs looks for, and a state is not a key.
        let completed = "COMPLETED";
        let session = parse_session(&json!({
            "id": "7249",
            "state": completed,
            "url": "https://jules.google.com/session/7249",
            "outputs": [
                {"changeSet": {"source": "sources/github/acme/widget"}},
                {"pullRequest": {"url": "https://github.com/acme/widget/pull/3", "title": "t"}},
            ],
        }))
        .unwrap();
        assert_eq!(session.state, completed);
        assert_eq!(
            session.pr.as_deref(),
            Some("https://github.com/acme/widget/pull/3")
        );
    }

    #[test]
    fn a_session_just_created_has_no_state_yet_and_reads_as_queued() {
        let session = parse_session(&json!({"id": "1", "name": "sessions/1"})).unwrap();
        assert_eq!(session.state, "QUEUED");
        assert_eq!(session.pr, None);
    }

    #[test]
    fn an_answer_without_an_id_is_not_a_session() {
        assert!(parse_session(&json!({"error": {}})).is_err());
    }

    #[test]
    fn an_id_that_could_walk_out_of_its_path_is_refused() {
        assert!(check_id("7249036751212567880").is_ok());
        for bad in ["", "../sources", "1?x=y", "1/activities"] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_key_command_that_is_off_or_prints_nothing_is_an_error() {
        assert!(read_key(&Hook::Off).is_err());
        assert!(read_key(&Hook::Command("true".to_string())).is_err());
        assert!(read_key(&Hook::Command("printf 'a\\nb'".to_string())).is_err());
        assert_eq!(
            read_key(&Hook::Command("echo ' k-1 '".to_string())).unwrap(),
            "k-1"
        );
    }
}
