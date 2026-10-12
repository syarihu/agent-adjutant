use serde_json::{Value, json};

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
        TaskAskError::Task(why) => why.clone(),
        TaskAskError::NoPr => "the task has no pull request yet".to_string(),
        TaskAskError::NotReadable(pr) => {
            format!("not a pull request on github.com this can read: {pr}")
        }
        TaskAskError::Refused(why) => review_refusal(why),
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

#[cfg(test)]
mod tests {
    use super::close_warning;

    #[test]
    fn a_close_warning_names_each_pr_and_leaves_out_an_empty_id() {
        let some = [("acme/widget#7".to_string(), "stuck".to_string())];
        assert!(close_warning(&some).ends_with("pull requests: acme/widget#7: stuck"));
        let none = [(String::new(), "no config".to_string())];
        assert!(close_warning(&none).ends_with("pull requests: no config"));
    }
}
