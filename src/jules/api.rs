//! The Jules API client: start a session from an approved plan, and ask how it is doing.
//!
//! Through `curl` rather than an HTTP client crate, the way pull requests are asked about
//! through `gh`: the binary stays three dependencies deep, and what it runs is a command
//! anybody can run by hand to see the same answer.
//!
//! The key is the one thing here that must not travel. It comes from a command (`julesKey`)
//! at the moment of the call, goes to `curl` on its stdin — an argument would be readable by
//! every process on the machine through `ps` — and is taken out of anything this module says
//! back, errors included, before that text can reach an agent's transcript.

use super::*;

use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

use crate::infra::terminal::Hook;

/// Where the API lives. A constant rather than a setting: there is one Jules.
pub const API: &str = "https://jules.googleapis.com/v1alpha";

/// The keychain item the built-in `julesKey` reads, as `security add-generic-password -s`
/// named it.
pub const KEYCHAIN_SERVICE: &str = "jules-api";

/// How long one call may take. The board asks while a person is looking at it, and a network
/// that has gone away should cost a few seconds of a stale badge, not a hung page.
const TIMEOUT_SECS: &str = "20";

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
pub(super) fn read_key(key: &Hook) -> Result<String, String> {
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
