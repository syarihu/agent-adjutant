use serde_json::{Value, json};

use crate::board::review::HandRefused;
use crate::board::{Check, CheckId, MachineChecks};
use crate::gate::Gate;
use crate::infra::terminal;
use crate::kernel::prompts;
use crate::lifecycle::review::{AskError, CloseError, TaskAskError};
use crate::mail::{wake_holds_for_session, wake_looks_at_screen};
use crate::registry::Context;
use crate::task::{self, Checked, PrState};

/// `refresh`'s answer as the MCP tool and the board hand it on: sorted by what happened, so
/// a reader finds what changed without going through what did not.
pub fn refresh_json(checked: &[Checked]) -> Value {
    let entry = |c: &Checked| {
        let mut out = json!({
            "id": c.task.id,
            "title": c.task.title,
            "pr": c.task.pr,
            "status": c.task.status.as_str(),
            "state": c.state.as_str(),
            // Whose turn it is, read from the summary kept on the record; null until one was.
            "turn": c.task.pr_status.as_ref().and_then(task::pr_turn),
        });
        if let PrState::Unreadable(why) = &c.state {
            out["error"] = json!(why);
        }
        if let Some(why) = &c.failed {
            out["error"] = json!(why);
        }
        out
    };
    let with = |pick: fn(&Checked) -> bool| -> Vec<Value> {
        checked.iter().filter(|c| pick(c)).map(entry).collect()
    };
    json!({
        "done": with(|c| c.moved),
        "open": with(|c| c.state == PrState::Open),
        "closed": with(|c| c.state == PrState::Closed),
        "unreadable": with(|c| matches!(c.state, PrState::Unreadable(_))),
        // Merged, but the record changed while `gh` was being asked, so it was left as it is.
        "skipped": with(|c| c.state == PrState::Merged && !c.moved && c.failed.is_none()),
        // Merged, but the record could not be moved. `error` says why.
        "failed": with(|c| c.failed.is_some()),
    })
}

/// What a caller that kept a record is told. Said, because the procedure it follows has
/// always ended its turn after opening a gate.
pub const RECORDED: &str = "Kept as a record: nobody is asked to answer it. Do not wait; go on \
                        with your work. If a person sends it back, the answer arrives in \
                        `adj outbox`.";

/// `adj gate open --json`, and the MCP tool's answer: the same object, so the procedure can
/// branch on it the same way whichever it used.
pub fn open_json(ctx: &Context, gate: &Gate, served: bool) -> Value {
    let mut out = json!({ "gate": gate, "server": if served { "up" } else { "down" } });
    if !gate.wait {
        out["wait"] = json!(false);
        out["note"] = json!(RECORDED);
    } else {
        let (wake, default_line) = if gate.answered_by_hub() {
            (&ctx.settings.hub_wake, terminal::HUB_WAKE_LINE)
        } else {
            (&ctx.settings.worker_wake, terminal::WORKER_WAKE_LINE)
        };
        if !wake.hook.is_off() {
            out["wakeLine"] = json!(wake.line_or(default_line));
            // Said only where it holds: the built-in tmux wake reads the screen, and a
            // session with a row is held by it on any terminal, so neither types its line
            // into a question, and a caller that knows that can end its turn at an empty
            // prompt instead of asking the same thing in the terminal too.
            if wake_looks_at_screen(&ctx.settings, gate.answered_by_hub())
                || wake_holds_for_session(
                    ctx,
                    std::path::Path::new(&gate.worktree),
                    gate.answered_by_hub(),
                )
            {
                out["wakeChecksScreen"] = json!(true);
            }
        }
    }
    out
}

/// A wake note as a sentence for the person at the terminal.
pub fn wake_note_sentence(note: &str) -> String {
    let mut chars = note.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

/// The text `skill` prints, rendered for the agent the override or the configured runner names.
pub fn skill_text(name: &str, arguments: &str, agent: Option<&str>) -> Result<String, String> {
    prompts::render_skill(&prompts::SkillRequest {
        name,
        arguments,
        agent,
        client_name: None,
        runner_dir: None,
    })
    .map(|r| r.text)
    .map_err(|e| e.to_string())
}

/// Why asking for a read-through was refused, as the CLI and the board both say it.
pub fn review_refusal(why: &AskError) -> String {
    match why {
        AskError::HubStopped => {
            "the code-review hub is not running: start it with `adj hub --review`, or from its row on the board".to_string()
        }
        AskError::NotStartable => {
            "a reader can only be opened with terminal.preset \"tmux\"".to_string()
        }
        AskError::NoRecord => "no such pull request on the list: run 「同期」 on the board first".to_string(),
        AskError::Done => "that pull request is done".to_string(),
        AskError::Failed(why) => why.clone(),
    }
}

/// Why asking for a read-through of a task's PR was refused, as the CLI and the board both say it.
pub fn task_review_refusal(why: &TaskAskError) -> String {
    match why {
        TaskAskError::NoTask => "no such task".to_string(),
        TaskAskError::TaskUnreadable(why) => why.clone(),
        TaskAskError::Syncing => "a sync is running; ask again once it ends".to_string(),
        TaskAskError::PrChanged => "the task's PR changed while it was read; ask again".to_string(),
        TaskAskError::NoPr => "the task has no pull request yet".to_string(),
        TaskAskError::NotReadable(pr) => {
            format!("not a pull request on github.com this can read: {pr}")
        }
        TaskAskError::Refused(why) => review_refusal(why),
    }
}

/// Why the drafts of a task's PR were not handed to its worker, as the board says it.
pub fn hand_refusal(why: &HandRefused) -> String {
    match why {
        HandRefused::NoTask => "no such task".to_string(),
        HandRefused::NoPr => "the task has no pull request yet".to_string(),
        HandRefused::PrChanged => "the task's PR is not the one the drafts are of".to_string(),
        HandRefused::NoWorker => {
            "the task is handed to Jules: it has no worker to tell".to_string()
        }
        HandRefused::NoRecord => "this pull request has not been read".to_string(),
        HandRefused::PrDone => "the pull request is merged or closed".to_string(),
        HandRefused::Nothing => "there is nothing to hand over".to_string(),
        HandRefused::GateChanged => "the gate was answered meanwhile; nothing was sent".to_string(),
        HandRefused::Busy => "the drafts are being handed over already".to_string(),
        HandRefused::NoGate => {
            "the task has no diff waiting and no diff record to send them back on".to_string()
        }
        HandRefused::GateWaiting(kind) => {
            format!("the task has a {} waiting: answer it first", kind.as_str())
        }
        HandRefused::Unreadable(why) | HandRefused::Failed(why) => why.clone(),
    }
}

/// Why a mode of a request for a read is refused, as the board says it.
pub fn unknown_read_mode() -> &'static str {
    "mode must be \"read\" or \"split\""
}

/// What the queue of readers could not do after a request, a hand-in or a failure was taken, as
/// the CLI and the board both say it.
pub fn queue_warning(why: &str) -> String {
    format!("the queue of readers could not be worked: {why}")
}

/// Why a reader was not closed, as the board says it.
pub fn close_refusal(why: &CloseError) -> String {
    match why {
        CloseError::NoRecord => {
            "no such pull request on the list: run 「同期」 on the board first".to_string()
        }
        CloseError::NoReader => "no reader is open on that pull request".to_string(),
        CloseError::Failed(why) => why.clone(),
    }
}

/// The readers a sync found merged or closed and could not close, as the board says it: each
/// stays open, and the next sync tries again.
pub fn close_warning(failures: &[(String, String)]) -> String {
    let each: Vec<String> = failures
        .iter()
        .map(|(id, why)| {
            if id.is_empty() {
                why.clone()
            } else {
                format!("{id}: {why}")
            }
        })
        .collect();
    format!(
        "could not close the readers of merged or closed pull requests: {}",
        each.join("; ")
    )
}

/// How to install `tool` on this machine: the Homebrew line on macOS, the package manager in
/// general elsewhere.
fn install(tool: &str, brew: &str) -> String {
    if cfg!(target_os = "macos") {
        brew.to_string()
    } else {
        format!("install {tool} with your package manager")
    }
}

/// What one startup check says, and what to do about it: `(message, fix)`. The fix is a command
/// or a short instruction, and `None` when the check passed. The CLI prints these and the board
/// page shows them, so the words are kept here.
pub fn machine_check(check: &Check, all: &MachineChecks) -> (String, Option<String>) {
    let detail = check.detail.as_deref().unwrap_or("");
    if check.ok {
        let message = match check.id {
            CheckId::Config => format!("config {}", all.config_path),
            CheckId::Tmux => "tmux found".to_string(),
            CheckId::TmuxVersion => format!("tmux {detail}"),
            CheckId::Gh => "gh found".to_string(),
            CheckId::GhAuth => "gh logged in".to_string(),
            CheckId::Mcp => format!("MCP server registered with {detail}"),
        };
        return (message, None);
    }
    let (message, fix) = match check.id {
        CheckId::Config => (
            detail.to_string(),
            format!("fix or move aside {}", all.config_path),
        ),
        CheckId::Tmux if check.fatal => (
            "tmux was not found, and workers open in tmux".to_string(),
            format!(
                "{}, or set \"terminal\": {{\"preset\": \"iterm2\"}} in {}",
                install("tmux", "brew install tmux"),
                all.config_path
            ),
        ),
        CheckId::Tmux => (
            "tmux was not found; the board cannot open terminals".to_string(),
            install("tmux", "brew install tmux"),
        ),
        CheckId::TmuxVersion => (
            format!("tmux {detail} is older than 3.1; the board's terminal is off"),
            install("tmux", "brew upgrade tmux"),
        ),
        CheckId::Gh => (
            "gh was not found; pull requests, issues and reviews are not read".to_string(),
            install("gh", "brew install gh"),
        ),
        CheckId::GhAuth => (
            match check.detail.as_deref() {
                Some(why) => format!(
                    "gh is not logged in; pull requests, issues and reviews are not read ({why})"
                ),
                None => "gh is not logged in; pull requests, issues and reviews are not read"
                    .to_string(),
            },
            "gh auth login".to_string(),
        ),
        CheckId::Mcp => (
            match check.detail.as_deref() {
                Some(why) => format!(
                    "the adjutant MCP server is not registered with Claude Code, Antigravity or Codex; hubs and workers have no adjutant_* tools ({why})"
                ),
                None => "the adjutant MCP server is not registered with Claude Code, Antigravity or Codex; hubs and workers have no adjutant_* tools".to_string(),
            },
            "adjutant install-mcp (Claude Code), adjutant install-mcp --target agy (Antigravity), or, for Codex, a [mcp_servers.adjutant] table with command = \"<path to adjutant>\" and args = [\"mcp\"] in Codex's config.toml ($CODEX_HOME, ~/.codex by default)".to_string(),
        ),
    };
    (message, Some(fix))
}

/// The checks as the board page and `adj server status --json` carry them.
pub fn machine_checks_json(all: &MachineChecks) -> Value {
    let checks: Vec<Value> = all
        .checks
        .iter()
        .map(|check| {
            let (message, fix) = machine_check(check, all);
            json!({
                "id": check.id.wire(),
                "ok": check.ok,
                "fatal": check.fatal,
                "message": message,
                "fix": fix,
            })
        })
        .collect();
    json!({ "checkedAt": all.checked_at, "checks": checks })
}

#[cfg(test)]
mod tests {
    use super::close_warning;
    use super::*;

    #[test]
    fn a_close_warning_names_each_pr_and_leaves_out_an_empty_id() {
        let some = [("acme/widget#7".to_string(), "stuck".to_string())];
        assert!(close_warning(&some).ends_with("pull requests: acme/widget#7: stuck"));
        let none = [(String::new(), "no config".to_string())];
        assert!(close_warning(&none).ends_with("pull requests: no config"));
    }

    fn failing(id: CheckId, fatal: bool, detail: Option<&str>) -> MachineChecks {
        MachineChecks {
            checks: vec![Check {
                id,
                ok: false,
                fatal,
                detail: detail.map(str::to_string),
            }],
            tmux: None,
            config_path: "/cfg/config.json".to_string(),
            tmux_backend: fatal,
            checked_at: 42,
        }
    }

    #[test]
    fn a_missing_tmux_names_both_ways_out_when_it_stops_the_server() {
        let all = failing(CheckId::Tmux, true, None);
        let (message, fix) = machine_check(&all.checks[0], &all);
        assert!(message.contains("workers open in tmux"), "{message}");
        let fix = fix.unwrap();
        assert!(
            fix.contains("iterm2") && fix.contains("/cfg/config.json"),
            "{fix}"
        );
    }

    #[test]
    fn the_json_carries_the_id_the_words_and_a_null_fix_for_a_pass() {
        let mut all = failing(
            CheckId::GhAuth,
            false,
            Some("gh auth status did not answer"),
        );
        all.checks.push(Check {
            id: CheckId::Gh,
            ok: true,
            fatal: false,
            detail: None,
        });
        let json = machine_checks_json(&all);
        assert_eq!(json["checkedAt"], 42);
        assert_eq!(json["checks"][0]["id"], "gh-auth");
        assert_eq!(json["checks"][0]["ok"], false);
        assert!(
            json["checks"][0]["message"]
                .as_str()
                .unwrap()
                .contains("did not answer")
        );
        assert_eq!(json["checks"][0]["fix"], "gh auth login");
        assert_eq!(json["checks"][1]["fix"], serde_json::Value::Null);
        assert_eq!(json["checks"][1]["message"], "gh found");
    }
}
